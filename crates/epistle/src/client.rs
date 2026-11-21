//! 모듈에 붙는 쪽의 클라이언트. 전송(파이프)은 `LineSource`/`LineSink` 뒤에 있어 Linux 에서 메모리 전송으로 시험한다.
//! 읽기 스레드 하나가 응답(id 있음)은 기다리는 호출에게, 알림(id 없음)은 콜백으로 넘긴다.
//! (구조는 스토리방화벽 화면의 client.rs — 러너와 실기기에서 확인된 것 — 을 따랐다.)

use std::collections::HashMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::rpc::{self, BadLine, Incoming, RpcError};
use crate::{api_compatible, HelloReply, EPISTLE_API};

/// 한 메시지(한 줄)를 읽는다. `Ok(None)` = 상대가 닫음. 상한(rpc::MAX_LINE)을 넘는 메시지는 Err 로 끝낸다
pub trait LineSource: Send {
    fn read_line(&mut self) -> std::io::Result<Option<Vec<u8>>>;
}
/// 한 메시지(한 줄)를 쓴다. 놓일(drop) 때 읽기 쪽의 대기를 풀어 주어야 한다.
/// `timeout` 안에 쓰지 못하면 `ErrorKind::TimedOut` 으로 돌아온다 — 상대가 읽지 않아도 부른 쪽이 멈추지 않는다
pub trait LineSink: Send {
    fn write_line(&mut self, line: &[u8], timeout: Duration) -> std::io::Result<()>;
}

/// 모양이 틀린 줄을 이만큼 받으면 연결을 끊는다(쓰레기를 쏟아붓는 상대)
pub const MAX_BAD_LINES: u32 = 8;

/// 읽기 스레드의 한도
#[derive(Debug, Clone, Copy)]
pub struct ClientOptions {
    /// 틀린 줄 세기를 이 시간마다 하나씩 덜어 준다. 정상 줄을 받는다고 덜지는 않는다 —
    /// 정상 줄과 틀린 줄을 번갈아 보내는 상대가 끝없이 쓰레기를 보낼 수 없게
    pub bad_line_forgive: Duration,
    /// 콜백으로 넘기는 알림의 초당 상한. 넘는 것은 버리고 센다(화면 쪽 상한의 앞단)
    pub notify_per_sec: u32,
}
impl Default for ClientOptions {
    fn default() -> Self {
        Self { bad_line_forgive: Duration::from_secs(60), notify_per_sec: 100 }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum CallError {
    /// 시한 안에 답이 없음
    Timeout,
    /// 연결이 끊김
    Closed,
    /// 보내지 못함
    Send(String),
    /// 모듈이 오류로 답함
    Rpc(RpcError),
}

/// 연결이 끝난 까닭
#[derive(Debug, Clone, PartialEq)]
pub enum CloseReason {
    /// 상대가 닫음
    Eof,
    /// 읽기 오류(상한을 넘는 메시지 포함)
    Read(String),
    /// 모양이 틀린 줄이 너무 많음
    Garbage,
    /// 읽기 스레드가 뜻밖에 끝남(알림 콜백이 죽은 경우 등)
    Panicked,
}

type Waiters = Arc<Mutex<HashMap<i64, Sender<Result<Value, RpcError>>>>>;

/// 잠금을 쥔 스레드가 죽었어도(poison) 표는 쓸 수 있다 — 안에 든 것은 보내는 쪽 채널뿐이다
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// 읽기 스레드의 지킴이. 스레드가 어떻게 끝나든(정상 종료, 콜백의 패닉) 놓일 때
/// `closed` 를 세우고, 기다리던 호출을 깨우고, 끊김을 한 번 알린다.
struct ReaderGuard {
    waiters: Waiters,
    closing: Arc<AtomicBool>,
    closed: Arc<AtomicBool>,
    on_close: Option<Box<dyn FnOnce(CloseReason) + Send>>,
    /// 정상으로 끝났을 때의 까닭. 비어 있으면 뜻밖의 종료다
    reason: Option<CloseReason>,
}
impl Drop for ReaderGuard {
    fn drop(&mut self) {
        // 순서: 깃발 먼저, 그다음 표 비우기. `call` 은 표의 잠금 안에서 깃발을 다시 보므로 틈이 없다
        self.closed.store(true, Ordering::SeqCst);
        lock(&self.waiters).clear();
        if !self.closing.load(Ordering::SeqCst) {
            if let Some(cb) = self.on_close.take() {
                let reason = self.reason.take().unwrap_or(CloseReason::Panicked);
                // 풀리는 중(unwinding)에 콜백까지 죽으면 프로세스가 끝난다 — 여기서 잡는다
                let _ = catch_unwind(AssertUnwindSafe(move || cb(reason)));
            }
        }
    }
}

pub struct Client {
    sink: Mutex<Box<dyn LineSink>>,
    next_id: Mutex<i64>,
    waiters: Waiters,
    /// 우리가 놓는 중 — 읽기 스레드가 끊김 콜백을 부르지 않게
    closing: Arc<AtomicBool>,
    closed: Arc<AtomicBool>,
    dropped_notifications: Arc<AtomicU64>,
}

impl Drop for Client {
    fn drop(&mut self) {
        // 필드(sink)는 이 함수 뒤에 놓인다 → 깃발이 먼저 선다
        self.closing.store(true, Ordering::SeqCst);
    }
}

impl Client {
    /// `on_notify(method, params)`: 알림. `on_close(reason)`: 상대 쪽 사정으로 끊겼을 때 한 번(우리가 놓은 경우는 부르지 않는다).
    /// 두 콜백은 읽기 스레드에서 불린다 — 오래 붙잡지 않는다.
    pub fn start(
        source: Box<dyn LineSource>,
        sink: Box<dyn LineSink>,
        on_notify: Box<dyn Fn(&str, Value) + Send>,
        on_close: Box<dyn FnOnce(CloseReason) + Send>,
    ) -> Client {
        Self::start_with(source, sink, on_notify, on_close, ClientOptions::default())
    }

    pub fn start_with(
        mut source: Box<dyn LineSource>,
        sink: Box<dyn LineSink>,
        on_notify: Box<dyn Fn(&str, Value) + Send>,
        on_close: Box<dyn FnOnce(CloseReason) + Send>,
        opts: ClientOptions,
    ) -> Client {
        let waiters: Waiters = Arc::new(Mutex::new(HashMap::new()));
        let closing = Arc::new(AtomicBool::new(false));
        let closed = Arc::new(AtomicBool::new(false));
        let dropped = Arc::new(AtomicU64::new(0));
        let guard = ReaderGuard { waiters: waiters.clone(), closing: closing.clone(), closed: closed.clone(), on_close: Some(on_close), reason: None };
        let (w, dropped2) = (waiters.clone(), dropped.clone());
        std::thread::spawn(move || {
            // 지킴이를 스레드 안으로 통째로 옮긴다 — 스레드가 끝날 때(패닉 포함) 놓인다
            let mut guard = guard;
            let mut bad = 0u32;
            let mut forgiven_at = Instant::now();
            let (mut win_start, mut win_count) = (Instant::now(), 0u32);
            let reason = loop {
                let line = match source.read_line() {
                    Ok(Some(l)) => l,
                    Ok(None) => break CloseReason::Eof,
                    Err(e) => break CloseReason::Read(e.to_string()),
                };
                match rpc::parse_incoming(trim(&line)) {
                    Ok(Incoming::Response { id, result }) => {
                        // 기다리는 호출이 없는 id(시한이 지난 답, 지어낸 id)는 버린다
                        if let Some(tx) = lock(&w).remove(&id) {
                            let _ = tx.send(result);
                        }
                    }
                    Ok(Incoming::Notification { method, params }) => {
                        if win_start.elapsed() >= Duration::from_secs(1) {
                            win_start = Instant::now();
                            win_count = 0;
                        }
                        win_count += 1;
                        if win_count > opts.notify_per_sec {
                            dropped2.fetch_add(1, Ordering::Relaxed);
                        } else {
                            on_notify(&method, params);
                        }
                    }
                    Err(BadLine::TooLong) => break CloseReason::Read("메시지가 상한을 넘음".into()),
                    Err(_) => {
                        // 지난 시간만큼 덜어 준 뒤에 센다
                        let forgive = opts.bad_line_forgive.as_millis().max(1);
                        let n = (forgiven_at.elapsed().as_millis() / forgive).min(u32::MAX as u128) as u32;
                        if n > 0 {
                            bad = bad.saturating_sub(n);
                            forgiven_at = Instant::now();
                        }
                        bad += 1;
                        if bad >= MAX_BAD_LINES {
                            break CloseReason::Garbage;
                        }
                    }
                }
            };
            guard.reason = Some(reason);
            // 놓이면서 깃발·깨우기·끊김 알림을 한다(패닉으로 끝날 때도 같다)
            drop(guard);
        });
        Client { sink: Mutex::new(sink), next_id: Mutex::new(1), waiters, closing, closed, dropped_notifications: dropped }
    }

    /// 초당 상한을 넘어 버린 알림의 수
    pub fn dropped_notifications(&self) -> u64 {
        self.dropped_notifications.load(Ordering::Relaxed)
    }

    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    /// 요청을 보내고 답을 기다린다. `params` 는 객체. `timeout` 은 쓰기와 답 기다리기를 합친 시한이다
    pub fn call(&self, method: &str, params: Value, timeout: Duration) -> Result<Value, CallError> {
        let started = Instant::now();
        let id = {
            let mut n = lock(&self.next_id);
            let id = *n;
            *n += 1;
            id
        };
        let (tx, rx) = mpsc::channel();
        {
            // 표의 잠금 안에서 닫힘을 본다 — 읽기 스레드가 표를 비운 뒤에 끼어들어 영영 기다리는 일이 없게
            let mut w = lock(&self.waiters);
            if self.is_closed() {
                return Err(CallError::Closed);
            }
            w.insert(id, tx);
        }
        let line = rpc::request_line(id, method, params);
        if let Err(e) = lock(&self.sink).write_line(line.as_bytes(), timeout) {
            lock(&self.waiters).remove(&id);
            return Err(if e.kind() == std::io::ErrorKind::TimedOut { CallError::Timeout } else { CallError::Send(e.to_string()) });
        }
        match rx.recv_timeout(timeout.saturating_sub(started.elapsed())) {
            Ok(Ok(v)) => Ok(v),
            Ok(Err(e)) => Err(CallError::Rpc(e)),
            Err(RecvTimeoutError::Timeout) => {
                lock(&self.waiters).remove(&id);
                Err(CallError::Timeout)
            }
            Err(RecvTimeoutError::Disconnected) => Err(CallError::Closed),
        }
    }

    /// rack.hello: 판을 맞추고, 답한 모듈 이름이 등록부의 슬롯 id 와 같은지 본다(변경 제안 2판 2장)
    pub fn hello(&self, slot_id: &str, client_version: &str, timeout: Duration) -> Result<HelloReply, HelloError> {
        let params = serde_json::json!({"client": "antioch", "client_version": client_version, "api": [EPISTLE_API.0, EPISTLE_API.1]});
        let v = self.call("rack.hello", params, timeout).map_err(HelloError::Call)?;
        let reply: HelloReply = serde_json::from_value(v).map_err(|_| HelloError::Malformed)?;
        if reply.module != slot_id {
            return Err(HelloError::WrongModule);
        }
        if !api_compatible(reply.api) {
            return Err(HelloError::ApiMismatch);
        }
        Ok(reply)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum HelloError {
    Call(CallError),
    Malformed,
    /// 등록부의 슬롯과 다른 이름으로 답함
    WrongModule,
    ApiMismatch,
}

fn trim(line: &[u8]) -> &[u8] {
    let mut l = line;
    while let [rest @ .., b'\n' | b'\r' | b' '] = l {
        l = rest;
    }
    l
}

/// 시험용 메모리 전송: 한쪽은 클라이언트에 끼우고, 다른 쪽(`Peer`)이 모듈 흉내를 낸다
pub mod mem {
    use super::{LineSink, LineSource};
    use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
    use std::time::Duration;

    pub struct Source(Receiver<Option<Vec<u8>>>);
    impl LineSource for Source {
        fn read_line(&mut self) -> std::io::Result<Option<Vec<u8>>> {
            match self.0.recv() {
                Ok(Some(l)) if l.len() > crate::rpc::MAX_LINE => Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "메시지가 상한을 넘음")),
                Ok(l) => Ok(l),
                Err(_) => Ok(None),
            }
        }
    }
    pub struct Sink {
        tx: Sender<Vec<u8>>,
        /// 놓일 때 읽기 쪽을 깨운다
        wake: Sender<Option<Vec<u8>>>,
    }
    impl LineSink for Sink {
        fn write_line(&mut self, line: &[u8], _timeout: std::time::Duration) -> std::io::Result<()> {
            self.tx.send(line.to_vec()).map_err(|_| std::io::Error::new(std::io::ErrorKind::BrokenPipe, "상대가 닫음"))
        }
    }
    impl Drop for Sink {
        fn drop(&mut self) {
            let _ = self.wake.send(None);
        }
    }

    /// 모듈 쪽
    pub struct Peer {
        rx: Receiver<Vec<u8>>,
        tx: Sender<Option<Vec<u8>>>,
    }
    impl Peer {
        /// 클라이언트가 보낸 다음 요청(JSON). 2초 안에 없으면 None
        pub fn next_request(&self) -> Option<serde_json::Value> {
            match self.rx.recv_timeout(Duration::from_secs(2)) {
                Ok(l) => serde_json::from_slice(&l).ok(),
                Err(RecvTimeoutError::Timeout) | Err(RecvTimeoutError::Disconnected) => None,
            }
        }
        pub fn send_raw(&self, line: &[u8]) {
            let _ = self.tx.send(Some(line.to_vec()));
        }
        pub fn send(&self, v: serde_json::Value) {
            self.send_raw(format!("{v}\n").as_bytes());
        }
        pub fn close(&self) {
            let _ = self.tx.send(None);
        }
        /// 클라이언트가 연결을 놓았는가(보내는 쪽이 사라짐)
        pub fn client_gone(&self) -> bool {
            matches!(self.rx.recv_timeout(Duration::from_secs(2)), Err(RecvTimeoutError::Disconnected))
        }
    }

    pub fn pair() -> (Box<Source>, Box<Sink>, Peer) {
        let (to_client, client_rx) = mpsc::channel();
        let (to_peer, peer_rx) = mpsc::channel();
        (Box::new(Source(client_rx)), Box::new(Sink { tx: to_peer, wake: to_client.clone() }), Peer { rx: peer_rx, tx: to_client })
    }
}
