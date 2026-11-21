//! Windows 쪽: 등록부 읽기, 네임드 파이프 접속, 파이프 서버 확인 (설계 5.2, 8.3 확정 + 제미나이 통합 3차 3-A).
//!
//! FFI 호출은 스토리방화벽 화면의 `client.rs`·`rack.rs`(러너와 실기기에서 확인된 것)와 같은 모양이다.
//! 이 파일은 Linux 에서 빌드되지 않는다 — Windows 러너의 `tests/win_pipe.rs` 가 실제 파이프·등록부로 시험한다.
//!
//! 접속 순서(`connect`): 파이프 열기(식별 수준 SQOS) → 서버 PID → **서버 확인** → 그 뒤에야 첫 바이트를 보낸다.

use std::sync::Arc;

use std::time::Duration;

use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_IO_PENDING, ERROR_MORE_DATA, HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, GetFinalPathNameByHandleW, GetShortPathNameW, ReadFile, WriteFile, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OVERLAPPED, FILE_GENERIC_READ,
    FILE_NAME_NORMALIZED, FILE_SHARE_DELETE, FILE_SHARE_NONE, FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_WRITE_ATTRIBUTES, FILE_WRITE_DATA,
    OPEN_EXISTING, SECURITY_IDENTIFICATION, SECURITY_SQOS_PRESENT, VOLUME_NAME_DOS,
};
use windows_sys::Win32::System::Com::CoTaskMemFree;
use windows_sys::Win32::System::Pipes::{
    GetNamedPipeServerProcessId, GetNamedPipeServerSessionId, SetNamedPipeHandleState, WaitNamedPipeW, PIPE_READMODE_MESSAGE,
};
use windows_sys::Win32::System::Registry::{
    RegCloseKey, RegEnumKeyExW, RegGetValueW, RegOpenKeyExW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_64KEY, RRF_RT_REG_SZ, RRF_SUBKEY_WOW6464KEY,
};
use windows_sys::Win32::System::Threading::{
    CreateEventW, OpenProcess, QueryFullProcessImageNameW, WaitForSingleObject, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows_sys::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};
use windows_sys::Win32::UI::Shell::{FOLDERID_ProgramFiles, FOLDERID_ProgramFilesX86, SHGetKnownFolderPath};

use crate::client::{Client, CloseReason, LineSink, LineSource};
use crate::discover::{Host, RawModule, RegistryView, Slot, SlotState};
use crate::names::{server_image_ok, MODULES_KEY, RUNNING_KEY};
use crate::rpc::MAX_LINE;
use crate::{valid_module_id, ModuleKind};

const ERROR_PIPE_BUSY: i32 = 231;
const ERROR_BROKEN_PIPE: i32 = 109;
const ERROR_OPERATION_ABORTED: i32 = 995;
/// 등록부에서 받아들이는 모듈 등록 수의 상한(이름 형식이 맞는 것만 센다)
const MAX_KEYS: usize = 64;
/// 하위 키를 열거하는 횟수의 상한(형식이 틀린 이름이 많아도 여기서 멈춘다)
const MAX_ENUM: u32 = 1024;
/// 파이프의 빈 인스턴스를 기다리는 시간. 한 번만 기다린다 — 인스턴스를 잡아 두는 가짜 서버가 발견을 오래 붙잡지 못하게
const BUSY_WAIT_MS: u32 = 500;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// 닫히는 핸들
struct Owned(HANDLE);
impl Drop for Owned {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

/// 등록부의 글자열 값. 없거나, 글자열(REG_SZ)이 아니거나, 너무 길면 None. 값은 자료다 — 부르는 쪽이 형식을 검사한다
pub fn reg_sz(hive: HKEY, key: &str, value: &str) -> Option<String> {
    let (k, v) = (wide(key), wide(value));
    let mut buf = [0u16; 1024];
    let mut cb = (buf.len() * 2) as u32;
    // 64비트 보기만 읽는다(규격 보강 2판 2장 2번 — HKCU 도 같은 깃발을 명시한다)
    // RRF_RT_REG_SZ: 다른 형식(REG_DWORD, REG_EXPAND_SZ …)이면 실패한다. 버퍼보다 길면 ERROR_MORE_DATA 로 실패한다
    let r = unsafe { RegGetValueW(hive, k.as_ptr(), v.as_ptr(), RRF_RT_REG_SZ | RRF_SUBKEY_WOW6464KEY, std::ptr::null_mut(), buf.as_mut_ptr() as *mut _, &mut cb) };
    if r != 0 {
        return None;
    }
    let n = (cb as usize / 2).min(buf.len());
    let s = String::from_utf16(&buf[..n]).ok()?;
    let s = s.trim_end_matches('\0').to_string();
    (!s.is_empty()).then_some(s)
}

/// 실제 등록부
pub struct WinRegistry;

impl RegistryView for WinRegistry {
    fn modules(&self) -> Vec<RawModule> {
        let mut out = Vec::new();
        let mut key: HKEY = std::ptr::null_mut();
        if unsafe { RegOpenKeyExW(HKEY_LOCAL_MACHINE, wide(MODULES_KEY).as_ptr(), 0, KEY_READ | KEY_WOW64_64KEY, &mut key) } != 0 {
            return out; // 키가 없음 = 등록된 모듈 없음
        }
        for i in 0..MAX_ENUM {
            if out.len() >= MAX_KEYS {
                break;
            }
            let mut name = [0u16; 260];
            let mut len = name.len() as u32;
            let r = unsafe {
                RegEnumKeyExW(key, i, name.as_mut_ptr(), &mut len, std::ptr::null(), std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut())
            };
            if r != 0 {
                break;
            }
            let Ok(id) = String::from_utf16(&name[..len as usize]) else { continue };
            // 형식이 틀린 이름은 값을 읽지도 않는다(이름이 경로 조각으로 쓰이므로)
            if !valid_module_id(&id) {
                continue;
            }
            let sub = format!(r"{MODULES_KEY}\{id}");
            out.push(RawModule {
                exe: reg_sz(HKEY_LOCAL_MACHINE, &sub, "Exe"),
                kind: reg_sz(HKEY_LOCAL_MACHINE, &sub, "Kind"),
                api: reg_sz(HKEY_LOCAL_MACHINE, &sub, "Api"),
                pipe_name: reg_sz(HKEY_LOCAL_MACHINE, &sub, "PipeName"),
                id,
            });
        }
        unsafe { RegCloseKey(key) };
        out
    }

    fn running_pipe(&self, id: &str) -> Option<String> {
        if !valid_module_id(id) {
            return None;
        }
        reg_sz(HKEY_CURRENT_USER, &format!(r"{RUNNING_KEY}\{id}"), "PipeName")
    }
}

/// 운영체제가 알려 주는 최종 경로(짧은 이름·링크를 푼 것, 8.3 의 5번). 파일을 열 수 없으면 None
pub fn final_path(path: &str) -> Option<String> {
    let w = wide(path);
    // 권한 0 으로 연다(속성만) — 읽기 권한이 없어도 경로는 얻는다
    let h = unsafe {
        CreateFileW(w.as_ptr(), 0, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, std::ptr::null(), OPEN_EXISTING, FILE_FLAG_BACKUP_SEMANTICS, std::ptr::null_mut())
    };
    if h == INVALID_HANDLE_VALUE {
        return None;
    }
    let h = Owned(h);
    let mut buf = vec![0u16; 2048];
    let n = unsafe { GetFinalPathNameByHandleW(h.0, buf.as_mut_ptr(), buf.len() as u32, FILE_NAME_NORMALIZED | VOLUME_NAME_DOS) } as usize;
    if n == 0 || n >= buf.len() {
        return None;
    }
    String::from_utf16(&buf[..n]).ok()
}

/// Program Files 폴더들 — 환경 변수가 아니라 운영체제에 묻는다(환경 변수는 실행한 쪽이 바꿀 수 있다)
pub fn admin_roots() -> Vec<String> {
    let mut out = Vec::new();
    for id in [&FOLDERID_ProgramFiles, &FOLDERID_ProgramFilesX86] {
        let mut p: *mut u16 = std::ptr::null_mut();
        let hr = unsafe { SHGetKnownFolderPath(id, 0, std::ptr::null_mut(), &mut p) };
        if hr >= 0 && !p.is_null() {
            let mut n = 0usize;
            while unsafe { *p.add(n) } != 0 && n < 4096 {
                n += 1;
            }
            if let Ok(s) = String::from_utf16(unsafe { std::slice::from_raw_parts(p, n) }) {
                if !out.contains(&s) {
                    out.push(s);
                }
            }
        }
        if !p.is_null() {
            unsafe { CoTaskMemFree(p as *const _) };
        }
    }
    out
}

/// 짧은 이름(8.3) 모양의 경로. 그 볼륨이 짧은 이름을 만들지 않으면 받은 경로가 그대로 나온다
fn short_path(path: &str) -> Option<String> {
    let w = wide(path);
    let mut buf = vec![0u16; 2048];
    let n = unsafe { GetShortPathNameW(w.as_ptr(), buf.as_mut_ptr(), buf.len() as u32) } as usize;
    if n == 0 || n >= buf.len() {
        return None;
    }
    String::from_utf16(&buf[..n]).ok()
}

/// 서버 프로세스가 "떴다고 적힌 자리"로 받아들이는 폴더들: Program Files 의 긴 이름, 최종 경로, 짧은 이름 모양.
/// 짧은 이름은 운영체제에 물은 값만 넣는다 — 다른 폴더가 그 이름을 가질 수 없다(같은 폴더 안에서 이름이 겹치지 않는다).
pub fn launch_roots() -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for r in admin_roots() {
        for form in [Some(r.clone()), final_path(&r), short_path(&r)].into_iter().flatten() {
            if !out.iter().any(|o| o.eq_ignore_ascii_case(&form)) {
                out.push(form);
            }
        }
    }
    out
}

/// 실제 PC
pub struct WinHost;
impl Host for WinHost {
    fn final_path(&self, path: &str) -> Option<String> {
        final_path(path)
    }
    fn admin_roots(&self) -> Vec<String> {
        // 폴더 자체도 최종 경로로 맞춘다
        admin_roots().iter().filter_map(|r| final_path(r)).collect()
    }
}

/// 프로세스의 실행 파일 전체 경로. 핸들을 먼저 잡고 그 핸들로 읽는다(PID 재사용 틈, 8.3). 열 권한이 없으면 None
pub fn process_image(pid: u32) -> Option<String> {
    let h = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if h.is_null() {
        return None;
    }
    let h = Owned(h);
    let mut buf = vec![0u16; 2048];
    let mut n = buf.len() as u32;
    if unsafe { QueryFullProcessImageNameW(h.0, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut n) } == 0 {
        return None;
    }
    String::from_utf16(&buf[..n as usize]).ok()
}

/// 파이프 핸들 (읽기·쓰기 반쪽이 함께 쥔다. 마지막이 놓을 때 닫는다)
pub struct Handle(HANDLE);
unsafe impl Send for Handle {}
unsafe impl Sync for Handle {}
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

/// 파이프를 열고 (핸들, 서버 PID) 를 돌려준다. **서버가 누구인지는 확인하지 않는다** — `verify_server` 가 한다.
///
/// - SQOS 를 식별 수준으로 건다: 서버가 우리 토큰으로 행동(가장)하지 못한다.
/// - GENERIC_WRITE 대신 개별 권한만 청한다: 서버 DACL 은 인스턴스 만들기(FILE_CREATE_PIPE_INSTANCE)를 주지 않는다.
/// - overlapped 로 연다: 동기 핸들을 읽기·쓰기 두 스레드가 나눠 쓰면 대기 중인 읽기가 쓰기를 막는다(스토리방화벽 A22 실측).
pub fn open_unchecked(name: &str) -> std::io::Result<(Arc<Handle>, u32)> {
    let w = wide(name);
    let mut tries = 0;
    let h = loop {
        let access = FILE_GENERIC_READ | FILE_WRITE_DATA | FILE_WRITE_ATTRIBUTES;
        let h = unsafe {
            CreateFileW(
                w.as_ptr(),
                access,
                FILE_SHARE_NONE,
                std::ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED | SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION,
                std::ptr::null_mut(),
            )
        };
        if h != INVALID_HANDLE_VALUE {
            break h;
        }
        let err = std::io::Error::last_os_error();
        // 서버의 빈 인스턴스가 잠시 없으면 한 번, 0.5초만 기다린다
        if err.raw_os_error() == Some(ERROR_PIPE_BUSY) && tries < 1 {
            tries += 1;
            unsafe { WaitNamedPipeW(w.as_ptr(), BUSY_WAIT_MS) };
            continue;
        }
        return Err(err);
    };
    let handle = Arc::new(Handle(h));
    let mut spid = 0u32;
    if unsafe { GetNamedPipeServerProcessId(h, &mut spid) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    let mut mode = PIPE_READMODE_MESSAGE;
    if unsafe { SetNamedPipeHandleState(h, &mut mode, std::ptr::null(), std::ptr::null()) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok((handle, spid))
}

struct Event(HANDLE);
unsafe impl Send for Event {}
impl Event {
    fn new() -> std::io::Result<Self> {
        let e = unsafe { CreateEventW(std::ptr::null(), 1, 0, std::ptr::null()) };
        if e.is_null() {
            return Err(std::io::Error::last_os_error());
        }
        Ok(Event(e))
    }
}
impl Drop for Event {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

/// overlapped 한 번을 끝까지 기다린다. 돌려주는 값: 메시지가 더 남았는가
fn wait(h: HANDLE, ov: &mut OVERLAPPED, started_ok: bool, n: &mut u32) -> std::io::Result<bool> {
    if !started_ok {
        let e = unsafe { GetLastError() };
        if e == ERROR_MORE_DATA {
            return Ok(true);
        }
        if e != ERROR_IO_PENDING {
            return Err(std::io::Error::from_raw_os_error(e as i32));
        }
    }
    if unsafe { GetOverlappedResult(h, ov, n, 1) } == 0 {
        let e = unsafe { GetLastError() };
        if e == ERROR_MORE_DATA {
            return Ok(true);
        }
        return Err(std::io::Error::from_raw_os_error(e as i32));
    }
    Ok(false)
}

pub struct PipeLine {
    h: Arc<Handle>,
    ev: Event,
    buf: Vec<u8>,
}
impl PipeLine {
    pub fn new(h: Arc<Handle>) -> std::io::Result<Self> {
        // 상한보다 조금 큰 버퍼 하나로 메시지를 통째로 받는다(스토리방화벽 화면과 같은 방식)
        Ok(Self { h, ev: Event::new()?, buf: vec![0u8; MAX_LINE + 4096] })
    }
}
impl LineSource for PipeLine {
    fn read_line(&mut self) -> std::io::Result<Option<Vec<u8>>> {
        // 메시지 모드: 한 번 읽기 = 메시지 하나. 버퍼에 다 들어오지 않으면(ERROR_MORE_DATA) 상한을 넘은 것이므로 끝낸다.
        // (러너 실측 run-1: 작은 버퍼로 이어 읽으면, 곧바로 ERROR_MORE_DATA 로 돌아온 첫 조각의 길이를 얻지 못해 조각을 잃었다)
        let mut ov: OVERLAPPED = unsafe { std::mem::zeroed() };
        ov.hEvent = self.ev.0;
        let mut n = 0u32;
        let ok = unsafe { ReadFile(self.h.0, self.buf.as_mut_ptr(), self.buf.len() as u32, &mut n, &mut ov) } != 0;
        match wait(self.h.0, &mut ov, ok, &mut n) {
            Ok(true) => Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "메시지가 상한을 넘음")),
            Ok(false) if n as usize > MAX_LINE => Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "메시지가 상한을 넘음")),
            Ok(false) => Ok(if n == 0 { None } else { Some(self.buf[..n as usize].to_vec()) }),
            // 끊김·취소 = 끝
            Err(e) if matches!(e.raw_os_error(), Some(ERROR_BROKEN_PIPE) | Some(ERROR_OPERATION_ABORTED)) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

pub struct PipeSink {
    h: Arc<Handle>,
    ev: Event,
}
impl PipeSink {
    pub fn new(h: Arc<Handle>) -> std::io::Result<Self> {
        Ok(Self { h, ev: Event::new()? })
    }
}
impl LineSink for PipeSink {
    fn write_line(&mut self, line: &[u8], timeout: Duration) -> std::io::Result<()> {
        let mut ov: OVERLAPPED = unsafe { std::mem::zeroed() };
        ov.hEvent = self.ev.0;
        let mut n = 0u32;
        let ok = unsafe { WriteFile(self.h.0, line.as_ptr(), line.len() as u32, &mut n, &mut ov) } != 0;
        if !ok {
            let e = unsafe { GetLastError() };
            if e != ERROR_IO_PENDING {
                return Err(std::io::Error::from_raw_os_error(e as i32));
            }
            // 상대가 읽지 않아 버퍼가 차면 쓰기가 끝나지 않는다 — 시한까지만 기다린다
            let ms = timeout.as_millis().min(u32::MAX as u128 - 1) as u32;
            let w = unsafe { WaitForSingleObject(self.ev.0, ms) };
            if w != WAIT_OBJECT_0 {
                // **이 쓰기만** 취소한다(읽기 스레드의 대기 중인 읽기는 건드리지 않는다)
                unsafe { CancelIoEx(self.h.0, &ov) };
                // 취소가 끝날 때까지 기다린 뒤에 돌아간다 — `ov` 는 이 함수의 스택에 있고, 커널이 아직 쓰고 있을 수 있다
                let done = unsafe { GetOverlappedResult(self.h.0, &ov, &mut n, 1) } != 0;
                if done && n as usize == line.len() {
                    return Ok(()); // 취소보다 쓰기가 먼저 끝났다
                }
                let kind = if w == WAIT_TIMEOUT { std::io::ErrorKind::TimedOut } else { std::io::ErrorKind::Other };
                return Err(std::io::Error::new(kind, "쓰기 시한 초과"));
            }
            if unsafe { GetOverlappedResult(self.h.0, &ov, &mut n, 0) } == 0 {
                return Err(std::io::Error::last_os_error());
            }
        }
        if n as usize != line.len() {
            return Err(std::io::Error::new(std::io::ErrorKind::WriteZero, "짧은 쓰기"));
        }
        Ok(())
    }
}
impl Drop for PipeSink {
    fn drop(&mut self) {
        // 읽기 스레드의 대기 중인 ReadFile 을 취소해 스레드가 끝나고 핸들이 닫히게 한다(서버가 끊김을 본다).
        // 핸들의 모든 I/O 를 취소하는 것(NULL)은 여기, 연결을 놓을 때뿐이다
        unsafe { CancelIoEx(self.h.0, std::ptr::null()) };
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectError {
    /// 실행 중이 아니거나 믿지 않는 슬롯
    NotRunning,
    /// 파이프를 열지 못함(모듈이 꺼졌는데 이름만 남은 경우 포함)
    Open(String),
    /// 파이프 서버 프로세스의 실행 파일을 알아내지 못함
    ServerUnknown,
    /// 파이프 서버가 등록된 실행 파일이 아님 — 한 바이트도 보내지 않고 놓았다
    ServerMismatch,
    /// 서비스형인데 파이프 서버가 session 0 이 아님
    NotSession0,
}

impl ConnectError {
    pub fn message(&self) -> &'static str {
        match self {
            ConnectError::NotRunning => "실행 중이 아닙니다",
            ConnectError::Open(_) => "파이프를 열지 못했습니다(모듈이 꺼졌을 수 있습니다)",
            ConnectError::ServerUnknown => "파이프의 상대 프로그램을 확인하지 못해 접속하지 않았습니다",
            ConnectError::ServerMismatch => "파이프의 상대가 등록된 프로그램이 아니어서 접속하지 않았습니다",
            ConnectError::NotSession0 => "파이프의 상대가 서비스(session 0)가 아니어서 접속하지 않았습니다",
        }
    }
}

/// 서버 확인이 어디까지 됐는가
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Verified {
    /// 파이프 서버 프로세스의 실행 파일이 등록된 Exe 와 같음을 확인했다.
    /// false 는 서비스형에서만 나온다: 일반 권한으로는 서비스 프로세스를 열지 못할 수 있어, 그때는
    /// "관리자만 쓰는 HKLM 에 공개된 파이프 이름 + session 0" 으로 확인한 것이다.
    pub image_checked: bool,
}

/// 파이프 서버 확인(5.2 의 3번, 8.3 의 3번). 통과하기 전에는 아무것도 보내지 않는다.
///
/// - 앱형: 서버 프로세스의 실행 파일이 등록된 Exe 여야 한다(`names::server_image_ok`). 알아내지 못하면 거부.
///   운영체제가 돌려준 경로 **그대로**가 `launch_roots`(관리자 폴더) 아래여야 한다 — 그 값은 띄울 때의 글자열이라,
///   이름으로 다시 열어 푼 값만 보면 띄운 뒤 폴더를 연결로 바꿔치기한 가짜 서버가 통과한다(러너 측정 M1).
/// - 서비스형: 서버가 session 0 이어야 한다(일반 사용자 프로세스는 session 0 에서 돌 수 없다 — 서비스가 죽은 뒤
///   남은 이름으로 가짜 서버를 세우는 것을 막는다). 실행 파일을 알아낼 수 있으면 등록된 Exe 와 같아야 한다.
pub fn verify_server(h: &Handle, server_pid: u32, kind: ModuleKind, exe_final: &str, launch_roots: &[String]) -> Result<Verified, ConnectError> {
    // (풀기 전의 값, 푼 값). 프로세스를 열지 못했거나 그 경로가 열리지 않으면 None
    let image = process_image(server_pid).and_then(|raw| final_path(&raw).map(|resolved| (raw, resolved)));
    if let Some((raw, resolved)) = &image {
        if !server_image_ok(raw, resolved, exe_final, launch_roots) {
            return Err(ConnectError::ServerMismatch);
        }
    }
    match kind {
        ModuleKind::Service => {
            let mut sess = u32::MAX;
            // 세션은 파이프 핸들에서 바로 얻는다(프로세스를 열 권한이 없어도 된다)
            let known = unsafe { GetNamedPipeServerSessionId(h.0, &mut sess) } != 0;
            if !(known && sess == 0) {
                return Err(ConnectError::NotSession0);
            }
            Ok(Verified { image_checked: image.is_some() })
        }
        _ => {
            if image.is_none() {
                return Err(ConnectError::ServerUnknown);
            }
            Ok(Verified { image_checked: true })
        }
    }
}

/// 슬롯에 붙는다: 열기 → 서버 확인 → 클라이언트 시작. rack.hello 는 부르는 쪽이 한다.
pub fn connect(
    slot: &Slot,
    on_notify: Box<dyn Fn(&str, serde_json::Value) + Send>,
    on_close: Box<dyn FnOnce(CloseReason) + Send>,
) -> Result<(Client, Verified), ConnectError> {
    connect_in(slot, &launch_roots(), on_notify, on_close)
}

/// `connect` 와 같되 서버 프로세스가 떠 있어야 할 폴더들을 받는다(시험에서 시험 실행 파일의 폴더를 넣는 데 쓴다)
pub fn connect_in(
    slot: &Slot,
    launch_roots: &[String],
    on_notify: Box<dyn Fn(&str, serde_json::Value) + Send>,
    on_close: Box<dyn FnOnce(CloseReason) + Send>,
) -> Result<(Client, Verified), ConnectError> {
    let SlotState::Running { pipe } = &slot.state else {
        return Err(ConnectError::NotRunning);
    };
    let (h, spid) = open_unchecked(pipe).map_err(|e| ConnectError::Open(e.to_string()))?;
    let verified = verify_server(&h, spid, slot.kind, &slot.exe_final, launch_roots)?;
    let source = PipeLine::new(h.clone()).map_err(|e| ConnectError::Open(e.to_string()))?;
    let sink = PipeSink::new(h).map_err(|e| ConnectError::Open(e.to_string()))?;
    Ok((Client::start(Box::new(source), Box::new(sink), on_notify, on_close), verified))
}
