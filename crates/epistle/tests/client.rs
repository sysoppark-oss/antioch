//! 클라이언트를 메모리 전송으로 시험한다: 요청·응답 짝짓기, 알림, 시한, 끊김, 틀린 줄.

use epistle::client::{mem, CallError, Client, ClientOptions, CloseReason, HelloError, LineSink, MAX_BAD_LINES};
use epistle::rpc::{RpcError, MAX_LINE, VERSION_MISMATCH};
use serde_json::{json, Value};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const T: Duration = Duration::from_secs(2);

struct Rig {
    client: Arc<Client>,
    peer: mem::Peer,
    events: Arc<Mutex<Vec<(String, Value)>>>,
    closed: mpsc::Receiver<CloseReason>,
}

fn rig() -> Rig {
    rig_with(ClientOptions::default())
}

fn rig_with(opts: ClientOptions) -> Rig {
    let (source, sink, peer) = mem::pair();
    let events = Arc::new(Mutex::new(Vec::new()));
    let ev = events.clone();
    let (tx, closed) = mpsc::channel();
    let client = Client::start_with(
        source,
        sink,
        Box::new(move |m, p| {
            // 시험용: 이 알림을 받으면 콜백이 죽는다
            assert!(m != "boom", "알림 콜백이 죽음(시험)");
            ev.lock().unwrap().push((m.to_string(), p))
        }),
        Box::new(move |r| {
            let _ = tx.send(r);
        }),
        opts,
    );
    Rig { client: Arc::new(client), peer, events, closed }
}

/// 다른 스레드에서 호출하고 결과를 받을 채널을 돌려준다
fn call_bg(c: &Arc<Client>, method: &'static str, timeout: Duration) -> mpsc::Receiver<Result<Value, CallError>> {
    let (tx, rx) = mpsc::channel();
    let c = c.clone();
    std::thread::spawn(move || {
        let _ = tx.send(c.call(method, json!({}), timeout));
    });
    rx
}

#[test]
fn 요청과_응답을_id로_짝짓는다() {
    let r = rig();
    let a = call_bg(&r.client, "rack.health", T);
    let req_a = r.peer.next_request().unwrap();
    let b = call_bg(&r.client, "rack.widgets", T);
    let req_b = r.peer.next_request().unwrap();
    assert_eq!(req_a["method"], "rack.health");
    assert_eq!(req_a["jsonrpc"], "2.0");
    assert!(req_a["params"].is_object());
    assert_ne!(req_a["id"], req_b["id"]);
    // 답을 거꾸로 보내도 제 호출에 닿는다
    r.peer.send(json!({"jsonrpc":"2.0","id":req_b["id"],"result":{"who":"b"}}));
    r.peer.send(json!({"jsonrpc":"2.0","id":req_a["id"],"result":{"who":"a"}}));
    assert_eq!(a.recv_timeout(T).unwrap().unwrap()["who"], "a");
    assert_eq!(b.recv_timeout(T).unwrap().unwrap()["who"], "b");
}

#[test]
fn 오류_응답과_알림() {
    let r = rig();
    let a = call_bg(&r.client, "rack.hello", T);
    let req = r.peer.next_request().unwrap();
    r.peer.send(json!({"jsonrpc":"2.0","method":"rack.event","params":{"id":"netx:evt:1"}}));
    r.peer.send(json!({"jsonrpc":"2.0","id":req["id"],"error":{"code":VERSION_MISMATCH,"message":"version_mismatch"}}));
    assert_eq!(a.recv_timeout(T).unwrap(), Err(CallError::Rpc(RpcError { code: VERSION_MISMATCH, message: "version_mismatch".into() })));
    let ev = r.events.lock().unwrap();
    assert_eq!(ev.len(), 1);
    assert_eq!(ev[0].0, "rack.event");
    assert_eq!(ev[0].1["id"], "netx:evt:1");
}

#[test]
fn 답이_없으면_시한_뒤에_돌아오고_늦은_답은_버린다() {
    let r = rig();
    let a = call_bg(&r.client, "rack.widgets", Duration::from_millis(80));
    let req = r.peer.next_request().unwrap();
    assert_eq!(a.recv_timeout(T).unwrap(), Err(CallError::Timeout));
    // 시한이 지난 뒤에 온 답, 지어낸 id 의 답 — 다음 호출에 섞이지 않는다
    r.peer.send(json!({"jsonrpc":"2.0","id":req["id"],"result":{"late":true}}));
    r.peer.send(json!({"jsonrpc":"2.0","id":999_999,"result":{"forged":true}}));
    let b = call_bg(&r.client, "rack.health", T);
    let req_b = r.peer.next_request().unwrap();
    r.peer.send(json!({"jsonrpc":"2.0","id":req_b["id"],"result":{"state":"ok"}}));
    assert_eq!(b.recv_timeout(T).unwrap().unwrap(), json!({"state":"ok"}));
}

#[test]
fn 상대가_닫으면_기다리던_호출이_끝나고_끊김을_알린다() {
    let r = rig();
    let a = call_bg(&r.client, "rack.health", Duration::from_secs(30));
    r.peer.next_request().unwrap();
    r.peer.close();
    assert_eq!(a.recv_timeout(T).unwrap(), Err(CallError::Closed));
    assert_eq!(r.closed.recv_timeout(T).unwrap(), CloseReason::Eof);
    assert!(r.client.is_closed());
    assert_eq!(r.client.call("rack.health", json!({}), T), Err(CallError::Closed), "끊긴 뒤의 호출은 바로 돌아온다");
}

#[test]
fn 우리가_놓으면_끊김_콜백을_부르지_않고_상대가_끊김을_본다() {
    let Rig { client, peer, closed, .. } = rig();
    drop(client);
    assert!(peer.client_gone());
    assert!(closed.recv_timeout(Duration::from_millis(300)).is_err());
}

#[test]
fn 틀린_줄은_몇_번까지_넘기고_계속되면_끊는다() {
    let r = rig();
    for _ in 0..MAX_BAD_LINES - 1 {
        r.peer.send_raw(b"<html>\n");
    }
    // 서버가 우리에게 보낸 "요청"도 틀린 줄로 센다 — 섀시는 요청을 받지 않는다
    let a = call_bg(&r.client, "rack.health", T);
    let req = r.peer.next_request().unwrap();
    r.peer.send(json!({"jsonrpc":"2.0","id":req["id"],"result":{}}));
    assert!(a.recv_timeout(T).unwrap().is_ok(), "아직은 살아 있다");
    r.peer.send(json!({"jsonrpc":"2.0","id":1,"method":"shell.exec","params":{}}));
    assert_eq!(r.closed.recv_timeout(T).unwrap(), CloseReason::Garbage);
    assert!(r.events.lock().unwrap().is_empty());
}

#[test]
fn 상한을_넘는_메시지는_바로_끊는다() {
    let r = rig();
    r.peer.send_raw(&vec![b'a'; MAX_LINE + 1]);
    assert!(matches!(r.closed.recv_timeout(T).unwrap(), CloseReason::Read(_)));
}

#[test]
fn hello는_판과_이름을_본다() {
    let hello = |reply: Value| {
        let r = rig();
        let (tx, rx) = mpsc::channel();
        let c = r.client.clone();
        std::thread::spawn(move || {
            let _ = tx.send(c.hello("storyfirewall", "0.1.0", T));
        });
        let req = r.peer.next_request().unwrap();
        assert_eq!(req["method"], "rack.hello");
        assert_eq!(req["params"], json!({"client":"antioch","client_version":"0.1.0","api":[1,0]}));
        r.peer.send(json!({"jsonrpc":"2.0","id":req["id"],"result":reply}));
        rx.recv_timeout(T).unwrap()
    };
    let ok = json!({"module":"storyfirewall","module_version":"0.1.6","api":[1,0],"granted":["read","operate"]});
    assert_eq!(hello(ok.clone()).unwrap().module_version, "0.1.6");

    let mut other = ok.clone();
    other["module"] = json!("netx");
    assert_eq!(hello(other), Err(HelloError::WrongModule));
    let mut v2 = ok.clone();
    v2["api"] = json!([2, 0]);
    assert_eq!(hello(v2), Err(HelloError::ApiMismatch));
    assert_eq!(hello(json!({"module":"storyfirewall"})), Err(HelloError::Malformed));
    // 모르는 권한 이름이 섞이면 응답 전체를 믿지 않는다
    let mut odd = ok;
    odd["granted"] = json!(["read", "root"]);
    assert_eq!(hello(odd), Err(HelloError::Malformed));
}

// ── 검증 답변(섀시 2차) 반영 ─────────────────────────────────────────────

#[test]
fn c3_알림_콜백이_죽어도_연결이_산_것처럼_남지_않는다() {
    let r = rig();
    let a = call_bg(&r.client, "rack.health", Duration::from_secs(30));
    r.peer.next_request().unwrap();
    r.peer.send(json!({"jsonrpc":"2.0","method":"boom","params":{}}));
    // 기다리던 호출이 시한(30초)을 다 쓰지 않고 바로 끝난다
    assert_eq!(a.recv_timeout(T).unwrap(), Err(CallError::Closed));
    assert_eq!(r.closed.recv_timeout(T).unwrap(), CloseReason::Panicked);
    assert!(r.client.is_closed());
    assert_eq!(r.client.call("rack.health", json!({}), T), Err(CallError::Closed));
}

#[test]
fn c2_쓰기가_시한을_넘기면_호출이_시한_초과로_돌아온다() {
    struct Stuck;
    impl LineSink for Stuck {
        fn write_line(&mut self, _line: &[u8], timeout: Duration) -> std::io::Result<()> {
            std::thread::sleep(timeout);
            Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "쓰기 시한 초과"))
        }
    }
    let (source, _sink, _peer) = mem::pair();
    let client = Client::start(source, Box::new(Stuck), Box::new(|_, _| {}), Box::new(|_| {}));
    let t0 = std::time::Instant::now();
    assert_eq!(client.call("rack.health", json!({}), Duration::from_millis(100)), Err(CallError::Timeout));
    assert!(t0.elapsed() < Duration::from_secs(2));
}

#[test]
fn c6_틀린_줄_세기는_시간이_지나면_덜어_준다() {
    // 덜어 주는 간격을 짧게 잡고, 그 간격보다 길게 띄워 틀린 줄을 상한의 두 배 넘게 보낸다
    let r = rig_with(ClientOptions { bad_line_forgive: Duration::from_millis(20), ..Default::default() });
    for _ in 0..MAX_BAD_LINES * 2 {
        r.peer.send_raw(b"<html>\n");
        std::thread::sleep(Duration::from_millis(40));
    }
    assert!(r.closed.recv_timeout(Duration::from_millis(100)).is_err(), "드문드문 온 틀린 줄로는 끊지 않는다");
    assert!(!r.client.is_closed());
}

#[test]
fn c6_정상_줄을_섞어_보내도_틀린_줄_세기는_줄지_않는다() {
    let r = rig();
    for i in 0..MAX_BAD_LINES {
        r.peer.send(json!({"jsonrpc":"2.0","method":"rack.event","params":{"n":i}}));
        r.peer.send_raw(b"<html>\n");
    }
    assert_eq!(r.closed.recv_timeout(T).unwrap(), CloseReason::Garbage);
}

#[test]
fn q5_알림이_초당_상한을_넘으면_버리고_센다() {
    let r = rig_with(ClientOptions { notify_per_sec: 20, ..Default::default() });
    for i in 0..500 {
        r.peer.send(json!({"jsonrpc":"2.0","method":"rack.event","params":{"n":i}}));
    }
    // 뒤따라 보낸 요청의 답이 오면 앞의 알림은 모두 처리된 것이다
    let a = call_bg(&r.client, "rack.health", T);
    let req = r.peer.next_request().unwrap();
    r.peer.send(json!({"jsonrpc":"2.0","id":req["id"],"result":{}}));
    a.recv_timeout(T).unwrap().unwrap();
    let got = r.events.lock().unwrap().len() as u64;
    assert!((20..=40).contains(&got), "받은 알림 {got}건(1초 창이 한 번 넘어갈 수 있다)");
    assert_eq!(got + r.client.dropped_notifications(), 500);
}

#[test]
fn q5_닫힌_뒤에_들어온_호출은_표에_남지_않고_바로_끝난다() {
    let r = rig();
    r.peer.close();
    assert_eq!(r.closed.recv_timeout(T).unwrap(), CloseReason::Eof);
    for _ in 0..50 {
        assert_eq!(r.client.call("rack.health", json!({}), Duration::from_secs(30)), Err(CallError::Closed));
    }
}
