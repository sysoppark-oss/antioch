//! Windows 러너에서만 도는 시험: 실제 네임드 파이프와 등록부로 접속·서버 확인·발견을 본다.
//! 시험용 서버는 모듈이 지켜야 할 모양(첫 인스턴스 깃발, 원격 거부, 인스턴스 만들기를 주지 않는 DACL)으로 연다.
#![cfg(windows)]

use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

use epistle::client::{CallError, HelloError};
use epistle::discover::{discover, Distrust, Host, RegistryView, Slot, SlotState};
use epistle::names::{same_path, under_admin_root, valid_pipe_name};
use epistle::win::{self, ConnectError, WinHost, WinRegistry};
use epistle::ModuleKind;
use serde_json::{json, Value};

use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, LocalFree, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Security::Authorization::{ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1};
use windows_sys::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
use windows_sys::Win32::Storage::FileSystem::{ReadFile, WriteFile, FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_MESSAGE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_MESSAGE,
    PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};
use windows_sys::Win32::System::RemoteDesktop::ProcessIdToSessionId;

const T: Duration = Duration::from_secs(5);
/// 스토리방화벽 서비스와 같은 DACL: 인증된 사용자는 읽기 + 데이터 쓰기 + 속성 쓰기(인스턴스 만들기 없음), 관리자·SYSTEM 전체
const SDDL: &str = "D:P(A;;0x12018b;;;AU)(A;;GA;;;BA)(A;;GA;;;SY)";

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

static SEQ: AtomicU32 = AtomicU32::new(0);
fn pipe_name(id: &str) -> String {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let salt = ((std::process::id() as u128) << 96) ^ ((SEQ.fetch_add(1, Ordering::SeqCst) as u128) << 80) ^ nanos;
    format!(r"\\.\pipe\{id}-{salt:032x}")
}

/// 파이프 인스턴스를 만든다. 실패하면 오류 코드
fn create_pipe(name: &str, first: bool) -> Result<usize, u32> {
    let mut sd: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
    let ok = unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(wide(SDDL).as_ptr(), SDDL_REVISION_1, &mut sd, std::ptr::null_mut()) };
    assert!(ok != 0, "SDDL 변환 실패");
    let sa = SECURITY_ATTRIBUTES { nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: sd, bInheritHandle: 0 };
    let mut open_mode = PIPE_ACCESS_DUPLEX;
    if first {
        open_mode |= FILE_FLAG_FIRST_PIPE_INSTANCE;
    }
    let h = unsafe {
        CreateNamedPipeW(
            wide(name).as_ptr(),
            open_mode,
            PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            PIPE_UNLIMITED_INSTANCES,
            1 << 21,
            1 << 21,
            0,
            &sa,
        )
    };
    let err = unsafe { GetLastError() };
    unsafe { LocalFree(sd as _) };
    if h == INVALID_HANDLE_VALUE {
        Err(err)
    } else {
        Ok(h as usize)
    }
}

struct Server {
    name: String,
    /// 서버가 받은 메시지 수
    received: Arc<AtomicU32>,
    done: mpsc::Receiver<()>,
}

/// 연결 하나를 받아 rack.* 에 답하는 흉내 모듈. `answer_as`: rack.hello·describe 에 적어 보낼 모듈 이름
fn serve(id: &str, answer_as: &'static str) -> Server {
    serve_after(id, answer_as, 0)
}

/// `stall_ms`: 접속을 받은 뒤 이만큼은 읽지 않고 가만히 있는다(읽지 않는 서버 흉내)
fn serve_after(id: &str, answer_as: &'static str, stall_ms: u64) -> Server {
    let name = pipe_name(id);
    let h = create_pipe(&name, true).expect("첫 인스턴스 만들기");
    let received = Arc::new(AtomicU32::new(0));
    let (tx, done) = mpsc::channel();
    let rc = received.clone();
    std::thread::spawn(move || {
        let h = h as HANDLE;
        unsafe { ConnectNamedPipe(h, std::ptr::null_mut()) }; // 이미 붙었으면 0(ERROR_PIPE_CONNECTED) — 그대로 진행
        std::thread::sleep(Duration::from_millis(stall_ms));
        let mut buf = vec![0u8; 1 << 21];
        let write = |v: Value| {
            let line = format!("{v}\n");
            let mut n = 0u32;
            unsafe { WriteFile(h, line.as_ptr(), line.len() as u32, &mut n, std::ptr::null_mut()) };
        };
        loop {
            let mut n = 0u32;
            if unsafe { ReadFile(h, buf.as_mut_ptr(), buf.len() as u32, &mut n, std::ptr::null_mut()) } == 0 || n == 0 {
                break;
            }
            rc.fetch_add(1, Ordering::SeqCst);
            let Ok(req) = serde_json::from_slice::<Value>(&buf[..n as usize]) else { break };
            let id = req["id"].clone();
            match req["method"].as_str().unwrap_or("") {
                "rack.hello" => write(json!({"jsonrpc":"2.0","id":id,"result":{"module":answer_as,"module_version":"9.9.9","api":[1,0],"granted":["read"]}})),
                "rack.health" => write(json!({"jsonrpc":"2.0","id":id,"result":{"state":"ok","since":1794355200,"notes":[],"checks":[]}})),
                "rack.subscribe" => {
                    write(json!({"jsonrpc":"2.0","id":id,"result":{}}));
                    write(json!({"jsonrpc":"2.0","method":"rack.event","params":{"id":format!("{answer_as}:evt:1"),"ts":1794355321,"module":answer_as,"severity":"info","kind":"test","title":"시험 사건"}}));
                }
                "big" => write(json!({"jsonrpc":"2.0","id":id,"result":{"blob":"가".repeat(200_000)}})),
                // 상한(1MB)을 넘는 메시지
                "huge" => write(json!({"jsonrpc":"2.0","id":id,"result":{"blob":"a".repeat(1_100_000)}})),
                "silent" => {}
                _ => write(json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"method not found"}})),
            }
        }
        unsafe {
            DisconnectNamedPipe(h);
            CloseHandle(h);
        }
        let _ = tx.send(());
    });
    Server { name, received, done }
}

fn me() -> String {
    win::final_path(std::env::current_exe().unwrap().to_str().unwrap()).expect("내 실행 파일의 최종 경로")
}

fn slot(id: &str, kind: ModuleKind, exe_final: String, pipe: &str) -> Slot {
    Slot { id: id.into(), kind, exe_final, state: SlotState::Running { pipe: pipe.into() } }
}

/// 시험 실행 파일이 든 빌드 폴더를 "설치 폴더"로 쳐서 붙는다(시험용 서버는 이 시험 프로세스다)
fn connect_here(
    slot: &Slot,
    on_notify: Box<dyn Fn(&str, Value) + Send>,
    on_close: Box<dyn FnOnce(epistle::client::CloseReason) + Send>,
) -> Result<(epistle::client::Client, win::Verified), ConnectError> {
    win::connect_in(slot, &build_roots(), on_notify, on_close)
}

fn build_roots() -> Vec<String> {
    let exe = std::env::current_exe().unwrap();
    let dir = exe.parent().unwrap().parent().unwrap().to_str().unwrap().to_string();
    let mut v = vec![dir.clone()];
    if let Some(f) = win::final_path(&dir) {
        v.push(f);
    }
    v
}

fn quiet() -> (Box<dyn Fn(&str, Value) + Send>, Box<dyn FnOnce(epistle::client::CloseReason) + Send>) {
    (Box::new(|_, _| {}), Box::new(|_| {}))
}

#[test]
fn 등록된_실행_파일이_연_파이프에_붙어_주고받는다() {
    let s = serve("zzci_a", "zzci_a");
    assert!(valid_pipe_name("zzci_a", &s.name));
    let (tx, events) = mpsc::channel();
    let (client, verified) = connect_here(
        &slot("zzci_a", ModuleKind::App, me(), &s.name),
        Box::new(move |m, p| {
            let _ = tx.send((m.to_string(), p));
        }),
        Box::new(|_| {}),
    )
    .expect("접속");
    assert!(verified.image_checked);

    let hello = client.hello("zzci_a", "0.1.0", T).expect("hello");
    assert_eq!(hello.module_version, "9.9.9");
    assert_eq!(client.call("rack.health", json!({}), T).unwrap()["state"], "ok");
    // 큰 메시지(약 600KB)도 한 메시지로 받는다
    let big = client.call("big", json!({}), T).unwrap();
    assert_eq!(big["blob"].as_str().unwrap().chars().count(), 200_000);
    // 읽기 스레드가 기다리는 동안에도 쓰기가 막히지 않는다(overlapped) — 답이 없는 요청 뒤에 다음 요청
    assert_eq!(client.call("silent", json!({}), Duration::from_millis(300)), Err(CallError::Timeout));
    assert!(matches!(client.call("nope", json!({}), T), Err(CallError::Rpc(e)) if e.code == -32601));
    client.call("rack.subscribe", json!({}), T).unwrap();
    let (method, params) = events.recv_timeout(T).expect("알림");
    assert_eq!(method, "rack.event");
    assert_eq!(params["id"], "zzci_a:evt:1");

    drop(client); // 우리가 놓으면 서버가 끊김을 본다
    s.done.recv_timeout(T).expect("서버 스레드 종료");
}

#[test]
fn c2_읽지_않는_서버에_쓰다가_멈추지_않고_읽기_스레드도_살아_있다() {
    // 서버가 4초 동안 읽지 않는다. 파이프 버퍼(2MB)가 차면 쓰기가 끝나지 않는다
    let s = serve_after("zzci_h", "zzci_h", 4000);
    let (tx, closed) = mpsc::channel();
    let (client, _) = connect_here(
        &slot("zzci_h", ModuleKind::App, me(), &s.name),
        Box::new(|_, _| {}),
        Box::new(move |r| {
            let _ = tx.send(r);
        }),
    )
    .expect("접속");
    let blob = "a".repeat(900_000);
    let t0 = std::time::Instant::now();
    for i in 0..5 {
        let r = client.call("fill", json!({ "blob": blob }), Duration::from_millis(300));
        println!("C2|쓰기 {i}|{:?}|{}ms", r.as_ref().err(), t0.elapsed().as_millis());
        assert_eq!(r, Err(CallError::Timeout));
    }
    // 고치기 전에는 세 번째쯤에서 영영 멈췄다. 호출마다 시한(0.3초) 안팎으로 돌아와야 한다
    assert!(t0.elapsed() < Duration::from_millis(3000), "쓰기가 시한을 지키지 않음: {:?}", t0.elapsed());
    // 쓰기를 취소한 것이 읽기 스레드의 대기 중인 읽기까지 취소하지 않았는지: 서버가 깨어난 뒤 답을 받는다
    assert!(closed.try_recv().is_err(), "쓰기 취소가 연결을 끊었다");
    std::thread::sleep(Duration::from_millis(4200).saturating_sub(t0.elapsed()));
    assert_eq!(client.call("rack.health", json!({}), T).expect("서버가 깨어난 뒤의 호출")["state"], "ok");
    assert!(!client.is_closed());
}

#[test]
fn 상한을_넘는_메시지를_보내는_서버는_끊는다() {
    let s = serve("zzci_g", "zzci_g");
    let (tx, closed) = mpsc::channel();
    let (client, _) = connect_here(
        &slot("zzci_g", ModuleKind::App, me(), &s.name),
        Box::new(|_, _| {}),
        Box::new(move |r| {
            let _ = tx.send(r);
        }),
    )
    .expect("접속");
    assert_eq!(client.call("huge", json!({}), T), Err(CallError::Closed));
    assert!(matches!(closed.recv_timeout(T).unwrap(), epistle::client::CloseReason::Read(_)));
}

#[test]
fn 등록된_실행_파일이_아닌_서버에는_한_바이트도_보내지_않는다() {
    // 가짜 서버(이 시험 프로세스)가 파이프를 열었는데, 등록부에는 다른 실행 파일이 적혀 있는 상황
    let s = serve("zzci_b", "zzci_b");
    let other = win::final_path(r"C:\Windows\System32\cmd.exe").expect("cmd.exe");
    let (n, c) = quiet();
    let r = connect_here(&slot("zzci_b", ModuleKind::App, other, &s.name), n, c);
    assert_eq!(r.err(), Some(ConnectError::ServerMismatch));
    s.done.recv_timeout(T).expect("서버 스레드 종료");
    assert_eq!(s.received.load(Ordering::SeqCst), 0, "서버가 받은 메시지가 없어야 한다");
}

#[test]
fn c1_설치_폴더_밖에서_뜬_서버는_경로가_맞아도_받지_않는다() {
    // 실제 규칙(Program Files 만)으로 붙는다. 서버는 빌드 폴더에서 뜬 이 시험 프로세스이고,
    // 등록된 Exe 를 서버의 실행 파일과 똑같이 맞춰 두어도(= 풀어 낸 값은 일치) 거부해야 한다
    println!("C1|떴다고 받아들이는 폴더|{:?}", win::launch_roots());
    let s = serve("zzci_i", "zzci_i");
    let (n, c) = quiet();
    let r = win::connect(&slot("zzci_i", ModuleKind::App, me(), &s.name), n, c);
    assert_eq!(r.err(), Some(ConnectError::ServerMismatch));
    s.done.recv_timeout(T).expect("서버 스레드 종료");
    assert_eq!(s.received.load(Ordering::SeqCst), 0, "서버가 받은 메시지가 없어야 한다");
}

#[test]
fn 다른_이름으로_답하는_서버는_hello에서_거른다() {
    let s = serve("zzci_c", "storyfirewall");
    let (n, c) = quiet();
    let (client, _) = connect_here(&slot("zzci_c", ModuleKind::App, me(), &s.name), n, c).expect("접속");
    assert_eq!(client.hello("zzci_c", "0.1.0", T), Err(HelloError::WrongModule));
}

#[test]
fn 서비스형은_session_0_의_서버만_받는다() {
    let s = serve("zzci_d", "zzci_d");
    let mut my_session = u32::MAX;
    assert!(unsafe { ProcessIdToSessionId(std::process::id(), &mut my_session) } != 0);
    println!("이 시험 프로세스의 session = {my_session}");
    let (n, c) = quiet();
    let r = connect_here(&slot("zzci_d", ModuleKind::Service, me(), &s.name), n, c);
    if my_session == 0 {
        assert!(r.is_ok(), "session 0 에서 돌면 통과해야 한다");
    } else {
        assert_eq!(r.err(), Some(ConnectError::NotSession0));
        s.done.recv_timeout(T).expect("서버 스레드 종료");
        assert_eq!(s.received.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn 꺼진_슬롯과_없는_파이프() {
    let (n, c) = quiet();
    let off = Slot { id: "zzci_e".into(), kind: ModuleKind::App, exe_final: me(), state: SlotState::Off };
    assert_eq!(connect_here(&off, n, c).err(), Some(ConnectError::NotRunning));
    let (n, c) = quiet();
    let gone = slot("zzci_e", ModuleKind::App, me(), &pipe_name("zzci_e"));
    assert!(matches!(connect_here(&gone, n, c), Err(ConnectError::Open(_))));
}

#[test]
fn 첫_인스턴스_깃발은_이름_선점을_막는다() {
    let name = pipe_name("zzci_f");
    let first = create_pipe(&name, true).expect("첫 인스턴스");
    // 같은 이름으로 "첫 인스턴스"를 또 만들 수 없다 = 누가 먼저 만들어 둔 이름이면 서버가 알아챈다
    assert_eq!(create_pipe(&name, true).err(), Some(5), "ERROR_ACCESS_DENIED");
    unsafe { CloseHandle(first as HANDLE) };
}

#[test]
fn 최종_경로와_관리자_폴더() {
    let roots = WinHost.admin_roots();
    println!("관리자 폴더: {roots:?}");
    assert!(!roots.is_empty());
    assert!(roots.iter().any(|r| r.to_lowercase().ends_with(r"\program files")));
    // 짧은 이름(8.3)으로 적어도 같은 폴더로 풀린다. 짧은 이름이 꺼진 볼륨이면 None — 그때는 건너뛴다
    if let Some(short) = win::final_path(r"C:\PROGRA~1") {
        assert!(same_path(&short, r"C:\Program Files"), "{short}");
    }
    assert!(win::final_path(r"C:\이런_파일은_없다\x.exe").is_none());
    let me = me();
    assert!(same_path(&me, std::env::current_exe().unwrap().to_str().unwrap()));
    assert!(!under_admin_root(&me, &roots), "시험 실행 파일은 Program Files 아래가 아니다: {me}");
    assert_eq!(win::process_image(std::process::id()).and_then(|p| win::final_path(&p)).map(|p| same_path(&p, &me)), Some(true));
}

fn reg(args: &[&str]) -> bool {
    Command::new("reg").args(args).output().map(|o| o.status.success()).unwrap_or(false)
}

#[test]
fn hkcu_실행_중_값은_글자열일_때만_읽는다() {
    let key = r"HKCU\SOFTWARE\Antioch\Running\zzci_reg";
    let good = pipe_name("zzci_reg");
    assert!(reg(&["add", key, "/v", "PipeName", "/t", "REG_SZ", "/d", &good, "/f"]));
    assert_eq!(WinRegistry.running_pipe("zzci_reg").as_deref(), Some(good.as_str()));
    // 형식이 다른 값(REG_DWORD)
    assert!(reg(&["add", key, "/v", "PipeName", "/t", "REG_DWORD", "/d", "1", "/f"]));
    assert_eq!(WinRegistry.running_pipe("zzci_reg"), None);
    // 너무 긴 값
    let long = "a".repeat(5000);
    assert!(reg(&["add", key, "/v", "PipeName", "/t", "REG_SZ", "/d", &long, "/f"]));
    assert_eq!(WinRegistry.running_pipe("zzci_reg"), None);
    // 이름 형식이 틀리면 등록부를 읽지도 않는다
    assert_eq!(WinRegistry.running_pipe(r"..\zzci_reg"), None);
    assert!(reg(&["delete", key, "/f"]));
    assert_eq!(WinRegistry.running_pipe("zzci_reg"), None);
}

/// Program Files 아래에서 아무 실행 파일 하나
fn some_installed_exe() -> Option<String> {
    fn walk(dir: &std::path::Path, depth: u32) -> Option<String> {
        let mut subs = Vec::new();
        for e in std::fs::read_dir(dir).ok()?.flatten() {
            let p = e.path();
            if p.is_dir() {
                subs.push(p);
            } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("exe")) {
                return p.to_str().map(str::to_string);
            }
        }
        if depth == 0 {
            return None;
        }
        subs.into_iter().find_map(|d| walk(&d, depth - 1))
    }
    walk(std::path::Path::new(r"C:\Program Files"), 3)
}

#[test]
fn 등록부에서_슬롯을_찾는다() {
    let base = r"HKLM\SOFTWARE\Antioch\Modules";
    let ids = ["zzci_ok", "zzci_user", "zzci_svc"];
    let cleanup = || {
        for id in ids {
            reg(&["delete", &format!(r"{base}\{id}"), "/f"]);
            reg(&["delete", &format!(r"HKCU\SOFTWARE\Antioch\Running\{id}"), "/f"]);
        }
        reg(&["delete", &format!(r"{base}\ZZCI-Bad"), "/f"]);
    };
    cleanup();
    let Some(installed) = some_installed_exe() else {
        println!("건너뜀: Program Files 아래에서 실행 파일을 찾지 못함");
        return;
    };
    let add = |id: &str, name: &str, value: &str| reg(&["add", &format!(r"{base}\{id}"), "/v", name, "/t", "REG_SZ", "/d", value, "/f"]);
    if !add("zzci_ok", "Exe", &installed) {
        println!("건너뜀: HKLM 에 쓸 권한이 없음(관리자가 아님)");
        return;
    }
    println!("설치된 실행 파일: {installed}");
    add("zzci_ok", "Kind", "app");
    add("zzci_ok", "Api", "1.0");
    // 사용자 폴더의 실행 파일을 등록한 모듈
    add("zzci_user", "Exe", std::env::current_exe().unwrap().to_str().unwrap());
    add("zzci_user", "Kind", "app");
    // 서비스형: HKLM 의 파이프 이름만 본다
    let svc_pipe = pipe_name("zzci_svc");
    add("zzci_svc", "Exe", &installed);
    add("zzci_svc", "Kind", "service");
    add("zzci_svc", "PipeName", &svc_pipe);
    // 형식이 틀린 id
    add("ZZCI-Bad", "Exe", &installed);
    // HKCU: 앱형의 실행 중 이름, 서비스형에 대한 가짜 이름, HKLM 에 없는 모듈
    let ok_pipe = pipe_name("zzci_ok");
    let hkcu = |id: &str, v: &str| reg(&["add", &format!(r"HKCU\SOFTWARE\Antioch\Running\{id}"), "/v", "PipeName", "/t", "REG_SZ", "/d", v, "/f"]);
    hkcu("zzci_ok", &ok_pipe);
    hkcu("zzci_user", &pipe_name("zzci_user"));
    hkcu("zzci_svc", &pipe_name("zzci_svc"));

    let slots = discover(&WinRegistry, &WinHost);
    cleanup();
    let find = |id: &str| slots.iter().find(|s| s.id == id);
    println!("{:#?}", slots.iter().filter(|s| s.id.starts_with("zzci")).collect::<Vec<_>>());

    let ok = find("zzci_ok").expect("zzci_ok");
    assert_eq!(ok.state, SlotState::Running { pipe: ok_pipe });
    assert!(same_path(&ok.exe_final, &installed));
    assert_eq!(find("zzci_user").expect("zzci_user").state, SlotState::Untrusted(Distrust::NotAdminFolder));
    let svc = find("zzci_svc").expect("zzci_svc");
    assert_eq!(svc.kind, ModuleKind::Service);
    assert_eq!(svc.state, SlotState::Running { pipe: svc_pipe }, "HKCU 의 가짜 이름이 아니라 HKLM 의 이름");
    assert!(!slots.iter().any(|s| s.id.eq_ignore_ascii_case("zzci-bad")));
}
