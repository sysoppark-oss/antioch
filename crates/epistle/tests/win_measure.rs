//! 재서 정할 것 — 「답변반영 — 안디옥 섀시 2차」 4장의 M1·M2. Windows 러너에서만 돈다.
//! 이 파일의 시험은 **판정하지 않고 적는다**: 결과는 `M1|…`, `M2|…` 로 시작하는 줄로 로그에 남긴다.
//!
//! 도우미 프로세스는 이 시험 실행 파일을 다시 띄워 만든다(`helper_entry` 시험을 환경 변수와 함께).
#![cfg(windows)]

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use epistle::win;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
use windows_sys::Win32::Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX};
use windows_sys::Win32::System::Pipes::{ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_MESSAGE, PIPE_TYPE_MESSAGE, PIPE_WAIT};
use windows_sys::Win32::System::Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION};

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// QueryFullProcessImageNameW 가 돌려주는 글자열. flags: 0 = Win32 경로, 1 = 원시(NT) 경로
fn image(pid: u32, flags: u32) -> String {
    let h = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if h.is_null() {
        return format!("(열지 못함: {})", std::io::Error::last_os_error());
    }
    let mut buf = vec![0u16; 2048];
    let mut n = buf.len() as u32;
    let ok = unsafe { QueryFullProcessImageNameW(h, flags, buf.as_mut_ptr(), &mut n) } != 0;
    let err = std::io::Error::last_os_error();
    unsafe { CloseHandle(h) };
    if ok {
        String::from_utf16_lossy(&buf[..n as usize])
    } else {
        format!("(실패: {err})")
    }
}

fn spawn_helper(exe: &Path, mode: &str, envs: &[(&str, String)]) -> std::process::Child {
    let mut c = Command::new(exe);
    c.args(["helper_entry", "--exact", "--nocapture", "--test-threads=1"])
        .env("EPISTLE_HELPER", mode)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    for (k, v) in envs {
        c.env(k, v);
    }
    c.spawn().expect("도우미 프로세스 띄우기")
}

fn kill(pid: u32) {
    let _ = Command::new("taskkill").args(["/PID", &pid.to_string(), "/F", "/T"]).output();
}

fn run(cmd: &str, args: &[&str]) -> (bool, String) {
    match Command::new(cmd).args(args).output() {
        Ok(o) => (o.status.success(), format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)).trim().replace(['\r', '\n'], " ")),
        Err(e) => (false, e.to_string()),
    }
}

/// 도우미로 불렸을 때만 일한다. 보통의 시험 실행에서는 바로 끝난다
#[test]
fn helper_entry() {
    let Ok(mode) = std::env::var("EPISTLE_HELPER") else { return };
    match mode.as_str() {
        "sleep" => std::thread::sleep(Duration::from_secs(60)),
        // M2 의 A: 파이프를 만들고, 서버 핸들을 물려받는 자식 B 를 띄운 뒤 끝난다
        "m2_a" => {
            let name = std::env::var("EPISTLE_PIPE").unwrap();
            let sa = SECURITY_ATTRIBUTES { nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: std::ptr::null_mut(), bInheritHandle: 1 };
            let h = unsafe {
                CreateNamedPipeW(wide(&name).as_ptr(), PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT, 1, 4096, 4096, 0, &sa)
            };
            let out = std::env::var("EPISTLE_OUT").unwrap();
            if h == INVALID_HANDLE_VALUE {
                std::fs::write(&out, format!("ERR 파이프 만들기 실패: {}", std::io::Error::last_os_error())).unwrap();
                std::process::exit(2);
            }
            let b = spawn_helper(&std::env::current_exe().unwrap(), "m2_b", &[("EPISTLE_HANDLE", (h as usize).to_string())]);
            std::fs::write(&out, format!("{} {}", std::process::id(), b.id())).unwrap();
            // B 가 핸들을 쥐고 있으므로 파이프는 살아 있다. A 는 여기서 끝난다
            std::process::exit(0);
        }
        // M2 의 B: 물려받은 서버 핸들로 접속을 기다린다
        "m2_b" => {
            let h = std::env::var("EPISTLE_HANDLE").unwrap().parse::<usize>().unwrap() as HANDLE;
            unsafe { ConnectNamedPipe(h, std::ptr::null_mut()) };
            std::thread::sleep(Duration::from_secs(30));
        }
        _ => {}
    }
    std::process::exit(0);
}

fn base_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("epistle_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// M1: 띄운 뒤 폴더를 바꿔치기하면 QueryFullProcessImageNameW 가 묵은 경로를 주는가
#[test]
fn m1_띄운_뒤_경로를_바꿔치기했을_때_프로세스의_실행_파일_경로() {
    println!("M1|운영체제|{}", run("cmd", &["/c", "ver"]).1);
    let base = base_dir("m1");
    let x = base.join("x");
    std::fs::create_dir_all(&x).unwrap();
    let app = x.join("app.exe");
    std::fs::copy(std::env::current_exe().unwrap(), &app).unwrap();
    println!("M1|띄울 실행 파일|{}", app.display());
    let child = spawn_helper(&app, "sleep", &[]);
    let pid = child.id();
    std::thread::sleep(Duration::from_millis(700));

    let note = |label: &str| {
        let w = image(pid, 0);
        let resolved = win::final_path(&w).unwrap_or_else(|| "(열지 못함)".into());
        println!("M1|{label}|win32={w}|native={}|win32 경로를 다시 열어 푼 값={resolved}", image(pid, 1));
        (w, resolved)
    };
    note("① 띄운 직후");

    // ② 폴더 이름 바꾸기 — 되는지부터
    let moved = base.join("x_moved");
    match std::fs::rename(&x, &moved) {
        Ok(()) => {
            println!("M1|② 실행 중인 파일이 든 폴더의 이름 바꾸기|됨");
        }
        Err(e) => {
            println!("M1|② 실행 중인 파일이 든 폴더의 이름 바꾸기|안 됨: {e}");
            std::fs::create_dir_all(&moved).unwrap();
            match std::fs::rename(&app, moved.join("app.exe")) {
                Ok(()) => println!("M1|② 대신 실행 파일만 옮기기|됨"),
                Err(e) => println!("M1|② 대신 실행 파일만 옮기기|안 됨: {e}"),
            }
            match std::fs::remove_dir(&x) {
                Ok(()) => println!("M1|② 빈 폴더 x 지우기|됨"),
                Err(e) => println!("M1|② 빈 폴더 x 지우기|안 됨: {e}"),
            }
        }
    }
    note("② 옮긴 뒤(x 는 없음)");

    // ③ 같은 자리에 다른 폴더를 가리키는 연결(junction). 그 폴더에는 다른 실행 파일을 같은 이름으로 둔다
    let target = base.join("target");
    std::fs::create_dir_all(&target).unwrap();
    std::fs::copy(r"C:\Windows\System32\cmd.exe", target.join("app.exe")).unwrap();
    let (ok, out) = run("cmd", &["/c", "mklink", "/J", x.to_str().unwrap(), target.to_str().unwrap()]);
    println!("M1|③ 연결(junction) 만들기|{}|{out}", if ok { "됨" } else { "안 됨" });
    let (w, resolved) = note("③ x 를 연결로 바꾼 뒤");

    // 판별에 쓸 한 줄: 돌려준 글자열이 여전히 …\x\app.exe 인가, 그리고 그것을 다시 열면 바꿔 둔 파일로 풀리는가
    let stale = epistle::names::norm_path(&w) == epistle::names::norm_path(app.to_str().unwrap());
    let target_final = win::final_path(target.join("app.exe").to_str().unwrap()).unwrap_or_default();
    let fooled = !target_final.is_empty() && epistle::names::same_path(&resolved, &target_final);
    println!("M1|결과|돌려준 경로가 띄울 때의 경로 그대로인가={stale}|다시 열어 풀면 바꿔 둔 파일이 되는가={fooled}");
    // C1 을 (가)로 고친 규칙으로 같은 상황을 판정한다: "진짜 설치 폴더"는 target, 등록된 Exe 는 target\app.exe 라고 치면
    // 옛 규칙(풀어 낸 값만 비교)은 속고, 새 규칙(돌려준 값 자체가 설치 폴더 아래)은 거부해야 한다
    let install_roots = vec![target.to_str().unwrap().to_string(), win::final_path(target.to_str().unwrap()).unwrap_or_default()];
    let new_rule = epistle::names::server_image_ok(&w, &resolved, &target_final, &install_roots);
    println!("M1|C1 고친 뒤|옛 규칙이 속는가={fooled}|새 규칙이 통과시키는가={new_rule}");
    if fooled {
        assert!(!new_rule, "바꿔치기한 서버를 새 규칙이 통과시켰다");
    }

    kill(pid);
    std::thread::sleep(Duration::from_millis(300));
    let _ = std::fs::remove_dir(&x); // 연결만 지운다(가리키는 폴더는 건드리지 않는다)
    let _ = std::fs::remove_dir_all(&base);
}

/// M1 의 곁가지: 짧은 이름(8.3)으로 띄운 프로세스가 돌려주는 글자열
#[test]
fn m1_짧은_이름으로_띄운_프로세스의_실행_파일_경로() {
    println!("M1|짧은 이름 설정|{}", run("fsutil", &["8dot3name", "query", "C:"]).1);
    let short_dir = r"C:\PROGRA~1";
    let Some(resolved) = win::final_path(short_dir) else {
        println!("M1|짧은 이름으로 띄움|건너뜀: {short_dir} 가 열리지 않음(이 볼륨은 짧은 이름을 만들지 않음)");
        return;
    };
    println!("M1|{short_dir} 를 풀면|{resolved}");
    let exe = format!(r"{short_dir}\PowerShell\7\pwsh.exe");
    if !Path::new(&exe).exists() {
        println!("M1|짧은 이름으로 띄움|건너뜀: {exe} 없음");
        return;
    }
    let child = Command::new(&exe)
        .args(["-NoProfile", "-Command", "Start-Sleep -Seconds 20"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    let Ok(child) = child else {
        println!("M1|짧은 이름으로 띄움|건너뜀: 띄우지 못함");
        return;
    };
    std::thread::sleep(Duration::from_millis(1500));
    let w = image(child.id(), 0);
    println!("M1|짧은 이름으로 띄움|띄운 경로={exe}|win32={w}|native={}|풀기 전 값에 ~ 가 남아 있는가={}", image(child.id(), 1), w.contains('~'));
    kill(child.id());
}

/// M2: 파이프를 만든 프로세스 A 가 서버 핸들을 자식 B 에 물려주고 끝나면, GetNamedPipeServerProcessId 는 누구의 PID 를 주는가
#[test]
fn m2_서버_핸들을_물려주고_끝난_뒤의_서버_pid() {
    let base = base_dir("m2");
    let out = base.join("pids.txt");
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let name = format!(r"\\.\pipe\zzci_m2-{:032x}", nanos ^ ((std::process::id() as u128) << 96));
    let mut a = spawn_helper(&std::env::current_exe().unwrap(), "m2_a", &[("EPISTLE_PIPE", name.clone()), ("EPISTLE_OUT", out.to_str().unwrap().to_string())]);
    let a_pid = a.id();
    let status = a.wait().unwrap();
    let text = std::fs::read_to_string(&out).unwrap_or_default();
    println!("M2|A 의 종료|{status}|기록={text}");
    let mut it = text.split_whitespace();
    let (Some(ra), Some(rb)) = (it.next().and_then(|s| s.parse::<u32>().ok()), it.next().and_then(|s| s.parse::<u32>().ok())) else {
        println!("M2|결과|잴 수 없음(도우미가 PID 를 남기지 못함)");
        return;
    };
    assert_eq!(ra, a_pid);
    std::thread::sleep(Duration::from_millis(500)); // A 가 완전히 끝나고 B 가 접속을 기다릴 때까지
    println!("M2|A 는 끝났는가|A 의 실행 파일 조회={}", image(a_pid, 0));

    match win::open_unchecked(&name) {
        Ok((_h, spid)) => {
            let who = if spid == a_pid {
                "A (파이프를 만들고 끝난 프로세스)"
            } else if spid == rb {
                "B (핸들을 물려받아 쥐고 있는 프로세스)"
            } else {
                "둘 다 아님"
            };
            println!("M2|결과|A={a_pid}|B={rb}|GetNamedPipeServerProcessId={spid}|누구인가={who}|그 PID 의 실행 파일 조회={}", image(spid, 0));
        }
        Err(e) => println!("M2|결과|파이프를 열지 못함: {e} (A 가 끝나며 파이프가 사라졌을 수 있다)"),
    }
    kill(rb);
    let _ = std::fs::remove_dir_all(&base);
}
