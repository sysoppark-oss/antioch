//! 등록부 자리, 파이프 이름 형식, 경로 비교. 순수 함수 — Linux 에서 시험한다.
//! 경로 비교·형식 검사는 스토리방화벽 `crates/rack`(실기기·러너에서 확인된 것)과 같은 규칙이다.

/// 설치 프로그램(관리자)이 쓰는 자리: `HKLM\SOFTWARE\Antioch\Modules\<id>` — 값 Exe, ClientExe, Kind, Api, (서비스형) PipeName
pub const MODULES_KEY: &str = r"SOFTWARE\Antioch\Modules";
/// 앱형 모듈이 실행 중일 때 쓰는 자리: `HKCU\SOFTWARE\Antioch\Running\<id>` — 값 PipeName
pub const RUNNING_KEY: &str = r"SOFTWARE\Antioch\Running";

/// 파이프 이름: `\\.\pipe\<id>-<128비트 hex>` (설계 5.1). 다른 모양은 쓰지 않는다 —
/// 등록부 값으로 남의 파이프나 다른 PC(`\\server\pipe\…`)에 붙게 만들 수 없다.
pub fn valid_pipe_name(id: &str, name: &str) -> bool {
    let Some(rest) = name.strip_prefix(r"\\.\pipe\") else {
        return false;
    };
    let Some(hex) = rest.strip_prefix(id).and_then(|r| r.strip_prefix('-')) else {
        return false;
    };
    hex.len() == 32 && hex.bytes().all(|c| c.is_ascii_hexdigit())
}

/// 비교용 모양: `\\?\` 접두사 제거, `/` → `\`, 소문자, 끝의 `\` 제거.
/// 짧은 이름·링크는 여기서 풀지 않는다 — 운영체제의 최종 경로(GetFinalPathNameByHandle)를 받은 뒤에 부른다.
pub fn norm_path(p: &str) -> String {
    let p = p.trim();
    let p = p
        .strip_prefix(r"\\?\UNC\")
        .map(|r| format!(r"\\{r}"))
        .unwrap_or_else(|| p.strip_prefix(r"\\?\").unwrap_or(p).to_string());
    let mut s: String = p.replace('/', r"\").to_lowercase();
    while s.len() > 3 && s.ends_with('\\') {
        s.pop();
    }
    s
}

/// 드라이브 문자로 시작하는 절대 경로이고 `.`·`..` 마디와 대체 데이터 스트림(`:`)이 없다
fn plain_absolute(p: &str) -> bool {
    let b = p.as_bytes();
    b.len() > 3
        && b[0].is_ascii_alphabetic()
        && b[1] == b':'
        && b[2] == b'\\'
        && !p[2..].contains(':')
        && !p.split('\\').any(|seg| seg == ".." || seg == ".")
}

/// 두 경로가 같은 파일을 가리키는가(둘 다 최종 경로일 때). 빈 값·절대 경로가 아닌 값은 같다고 하지 않는다
pub fn same_path(a: &str, b: &str) -> bool {
    let (a, b) = (norm_path(a), norm_path(b));
    plain_absolute(&a) && a == b
}

/// 실행 파일이 관리자만 쓸 수 있는 폴더(Program Files 등) 아래인가 (8.3 의 4번)
pub fn under_admin_root(exe: &str, roots: &[String]) -> bool {
    let e = norm_path(exe);
    if !plain_absolute(&e) {
        return false;
    }
    roots
        .iter()
        .map(|r| norm_path(r))
        .filter(|r| plain_absolute(r))
        .any(|r| e.len() > r.len() + 1 && e.starts_with(&r) && e.as_bytes()[r.len()] == b'\\')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 파이프_이름() {
        let ok = r"\\.\pipe\netx-0123456789abcdef0123456789ABCDEF";
        assert!(valid_pipe_name("netx", ok));
        assert!(!valid_pipe_name("storyfirewall", ok), "다른 모듈의 이름");
        for bad in [
            r"\\.\pipe\netx",
            r"\\.\pipe\netx-0123",
            r"\\.\pipe\netx-0123456789abcdef0123456789abcdeg",
            r"\\evil\pipe\netx-0123456789abcdef0123456789abcdef",
            r"\\.\pipe\..\netx-0123456789abcdef0123456789abcdef",
            r"\\.\pipe\netx-0123456789abcdef0123456789abcdef\x",
            r"netx-0123456789abcdef0123456789abcdef",
            "",
        ] {
            assert!(!valid_pipe_name("netx", bad), "{bad}");
        }
    }

    #[test]
    fn 경로_비교() {
        assert!(same_path(r"C:\Program Files\NetX\netx.exe", r"\\?\c:\program files\netx\NETX.EXE"));
        assert!(same_path(r"C:/Program Files/NetX/netx.exe", r"C:\Program Files\NetX\netx.exe"));
        assert!(!same_path(r"C:\Program Files\NetX\netx.exe", r"C:\Program Files\NetX\netx2.exe"));
        assert!(!same_path("", ""));
        assert!(!same_path(r"netx.exe", r"netx.exe"), "절대 경로가 아니면 같다고 하지 않는다");
        assert!(!same_path(r"\\server\share\netx.exe", r"\\server\share\netx.exe"), "다른 PC의 경로");
        assert!(!same_path(r"C:\a\..\b\x.exe", r"C:\a\..\b\x.exe"));
        assert!(!same_path(r"C:\a\x.exe:evil", r"C:\a\x.exe:evil"), "대체 데이터 스트림");
    }

    #[test]
    fn 관리자_폴더_아래() {
        let roots = vec![r"C:\Program Files".to_string(), r"C:\Program Files (x86)".to_string()];
        assert!(under_admin_root(r"C:\Program Files\NetX\netx.exe", &roots));
        assert!(under_admin_root(r"\\?\C:\Program Files (x86)\A\a.exe", &roots));
        for bad in [
            r"C:\Users\me\AppData\Local\NetX\netx.exe",
            r"C:\Program Files",
            r"C:\Program Files2\x.exe",
            r"C:\Program Files\..\Users\me\x.exe",
            r"D:\Program Files\x.exe",
            r"\\server\Program Files\x.exe",
            r"Program Files\x.exe",
        ] {
            assert!(!under_admin_root(bad, &roots), "{bad}");
        }
        assert!(!under_admin_root(r"C:\Program Files\x.exe", &[]));
    }
}
