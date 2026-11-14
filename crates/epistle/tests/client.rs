//! 클라이언트를 메모리 전송으로 시험한다: 요청·응답 짝짓기, 알림, 시한, 끊김, 틀린 줄.

use epistle::client::{mem, CallError, Client, CloseReason, HelloError, MAX_BAD_LINES};
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
    let (source, sink, peer) = mem::pair();
    let events = Arc::new(Mutex::new(Vec::new()));
    let ev = events.clone();
    let (tx, closed) = mpsc::channel();
    let client = Client::start(
        source,
        sink,
        Box::new(move |m, p| ev.lock().unwrap().push((m.to_string(), p))),
        Box::new(move |r| {
            let _ = tx.send(r);
        }),
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
