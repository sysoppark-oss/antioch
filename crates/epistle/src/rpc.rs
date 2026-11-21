//! 봉투: 한 줄 JSON-RPC 2.0 (설계 6.1 확정). 접속하는 쪽(섀시)이 쓰는 부분만 있다.
//! 받은 줄은 해석하기 전에 길이와 중첩 깊이를 본다 — 모듈이 보낸 것은 자료이고, 해석기를 지치게 하는 입력일 수 있다.

use serde_json::Value;

/// 한 줄(메시지 하나)의 상한. 6.1 의 "요청 1MB 상한"을 응답·알림에도 같이 쓴다 [제안 — 6.1 은 요청만 적었다]
pub const MAX_LINE: usize = 1024 * 1024;
/// 중첩 깊이 상한. rack.* 응답은 깊이 6을 넘지 않는다
pub const MAX_DEPTH: usize = 32;

pub const VERSION_MISMATCH: i64 = -32006;
pub const UNAUTHORIZED: i64 = -32001;
pub const METHOD_NOT_FOUND: i64 = -32601;

#[derive(Debug, Clone, PartialEq)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Incoming {
    /// 요청에 대한 답
    Response { id: i64, result: Result<Value, RpcError> },
    /// 서버가 밀어 주는 알림(id 없음)
    Notification { method: String, params: Value },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BadLine {
    TooLong,
    TooDeep,
    NotJson,
    /// JSON 이지만 JSON-RPC 2.0 응답·알림 모양이 아니다(우리에게 온 요청 포함 — 섀시는 요청을 받지 않는다)
    NotRpc,
}

/// 요청 한 줄(끝에 개행). params 는 객체여야 한다(6.1)
pub fn request_line(id: i64, method: &str, params: Value) -> String {
    debug_assert!(params.is_object());
    let mut s = serde_json::json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string();
    s.push('\n');
    s
}

/// 문자열 밖의 `{`·`[` 중첩이 상한 안인가. 해석 전에 바이트만 훑는다
pub fn depth_ok(line: &[u8], max: usize) -> bool {
    let (mut depth, mut in_str, mut esc) = (0usize, false, false);
    for &b in line {
        if in_str {
            if esc {
                esc = false;
            } else if b == b'\\' {
                esc = true;
            } else if b == b'"' {
                in_str = false;
            }
            continue;
        }
        match b {
            b'"' => in_str = true,
            b'{' | b'[' => {
                depth += 1;
                if depth > max {
                    return false;
                }
            }
            b'}' | b']' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    true
}

/// 받은 한 줄을 응답 또는 알림으로 읽는다
pub fn parse_incoming(line: &[u8]) -> Result<Incoming, BadLine> {
    if line.len() > MAX_LINE {
        return Err(BadLine::TooLong);
    }
    if !depth_ok(line, MAX_DEPTH) {
        return Err(BadLine::TooDeep);
    }
    let v: Value = serde_json::from_slice(line).map_err(|_| BadLine::NotJson)?;
    let obj = v.as_object().ok_or(BadLine::NotRpc)?;
    if obj.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Err(BadLine::NotRpc);
    }
    match (obj.get("id"), obj.get("method")) {
        // 알림: method 있고 id 없음
        (None, Some(m)) => {
            let method = m.as_str().ok_or(BadLine::NotRpc)?;
            if method.is_empty() || method.len() > 64 || !method.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'.' || c == b'_') {
                return Err(BadLine::NotRpc);
            }
            // 알림의 params 는 객체다. 없거나 객체가 아니면 틀린 줄로 센다(규격 보강 2판 3장 2번)
            let params = obj.get("params").filter(|p| p.is_object()).cloned().ok_or(BadLine::NotRpc)?;
            Ok(Incoming::Notification { method: method.to_string(), params })
        }
        // 응답: id 있고 method 없음, result 와 error 가운데 하나
        (Some(id), None) => {
            let id = id.as_i64().ok_or(BadLine::NotRpc)?;
            match (obj.get("result"), obj.get("error")) {
                (Some(r), None) => Ok(Incoming::Response { id, result: Ok(r.clone()) }),
                (None, Some(e)) => {
                    let code = e.get("code").and_then(Value::as_i64).ok_or(BadLine::NotRpc)?;
                    let message: String = e.get("message").and_then(Value::as_str).unwrap_or("").chars().take(200).collect();
                    Ok(Incoming::Response { id, result: Err(RpcError { code, message }) })
                }
                _ => Err(BadLine::NotRpc),
            }
        }
        _ => Err(BadLine::NotRpc),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 고정_예시를_읽는다() {
        let r = parse_incoming(include_bytes!("../../../fixtures/hello_response.json")).unwrap();
        assert!(matches!(r, Incoming::Response { id: 1, result: Ok(_) }));
        let n = parse_incoming(include_bytes!("../../../fixtures/event_notification.json")).unwrap();
        assert!(matches!(n, Incoming::Notification { ref method, .. } if method == "rack.event"));
    }

    #[test]
    fn 알림의_params_는_객체여야_한다() {
        let ok = br#"{"jsonrpc":"2.0","method":"rack.event","params":{}}"#;
        assert!(matches!(parse_incoming(ok), Ok(Incoming::Notification { .. })));
        for bad in [
            &br#"{"jsonrpc":"2.0","method":"rack.event"}"#[..],
            br#"{"jsonrpc":"2.0","method":"rack.event","params":null}"#,
            br#"{"jsonrpc":"2.0","method":"rack.event","params":[1]}"#,
            br#"{"jsonrpc":"2.0","method":"rack.event","params":"x"}"#,
        ] {
            assert_eq!(parse_incoming(bad).err(), Some(BadLine::NotRpc), "{}", String::from_utf8_lossy(bad));
        }
    }

    #[test]
    fn 응답의_id_는_정수만_받는다() {
        assert!(matches!(parse_incoming(br#"{"jsonrpc":"2.0","id":7,"result":{}}"#), Ok(Incoming::Response { id: 7, .. })));
        for bad in [
            &br#"{"jsonrpc":"2.0","id":"7","result":{}}"#[..],
            br#"{"jsonrpc":"2.0","id":null,"result":{}}"#,
            br#"{"jsonrpc":"2.0","id":7.5,"result":{}}"#,
        ] {
            assert_eq!(parse_incoming(bad).err(), Some(BadLine::NotRpc), "{}", String::from_utf8_lossy(bad));
        }
    }

    #[test]
    fn 요청_한_줄() {
        let l = request_line(7, "rack.health", serde_json::json!({}));
        assert!(l.ends_with('\n') && !l[..l.len() - 1].contains('\n'));
        let v: Value = serde_json::from_str(&l).unwrap();
        assert_eq!(v["id"], 7);
        assert_eq!(v["method"], "rack.health");
    }

    #[test]
    fn 오류_응답() {
        let r = parse_incoming(br#"{"jsonrpc":"2.0","id":3,"error":{"code":-32006,"message":"version_mismatch"}}"#).unwrap();
        assert_eq!(r, Incoming::Response { id: 3, result: Err(RpcError { code: VERSION_MISMATCH, message: "version_mismatch".into() }) });
    }

    #[test]
    fn 모양이_틀린_줄() {
        for (line, want) in [
            (&b"hello"[..], BadLine::NotJson),
            (b"[1,2]", BadLine::NotRpc),
            (br#"{"id":1,"result":{}}"#, BadLine::NotRpc),
            (br#"{"jsonrpc":"1.0","id":1,"result":{}}"#, BadLine::NotRpc),
            (br#"{"jsonrpc":"2.0","id":"1","result":{}}"#, BadLine::NotRpc),
            (br#"{"jsonrpc":"2.0","id":1,"result":{},"error":{"code":1}}"#, BadLine::NotRpc),
            (br#"{"jsonrpc":"2.0","id":1}"#, BadLine::NotRpc),
            // 서버가 우리에게 요청을 보냄 — 섀시는 받지 않는다
            (br#"{"jsonrpc":"2.0","id":1,"method":"shell.exec","params":{}}"#, BadLine::NotRpc),
            (br#"{"jsonrpc":"2.0","method":"rack event"}"#, BadLine::NotRpc),
        ] {
            assert_eq!(parse_incoming(line), Err(want), "{}", String::from_utf8_lossy(line));
        }
    }

    #[test]
    fn 길이와_깊이_상한() {
        let deep = format!(r#"{{"jsonrpc":"2.0","id":1,"result":{}{}}}"#, "[".repeat(5000), "]".repeat(5000));
        assert_eq!(parse_incoming(deep.as_bytes()), Err(BadLine::TooDeep));
        // 문자열 안의 괄호는 깊이가 아니다
        let fine = format!(r#"{{"jsonrpc":"2.0","id":1,"result":{{"t":"{}\"{}"}}}}"#, "[".repeat(100), "{".repeat(100));
        assert!(parse_incoming(fine.as_bytes()).is_ok());
        let long = format!(r#"{{"jsonrpc":"2.0","id":1,"result":"{}"}}"#, "a".repeat(MAX_LINE));
        assert_eq!(parse_incoming(long.as_bytes()), Err(BadLine::TooLong));
    }

    #[test]
    fn 아무_바이트에도_죽지_않는다() {
        // 고정 씨앗의 간단한 난수 — 어떤 입력에도 panic 없이 Ok/Err 를 돌려준다
        let mut x: u64 = 0x9E3779B97F4A7C15;
        for len in 0..600usize {
            let bytes: Vec<u8> = (0..len)
                .map(|_| {
                    x ^= x << 13;
                    x ^= x >> 7;
                    x ^= x << 17;
                    b"{}[]\":,\\ \n0a\xff"[(x % 13) as usize]
                })
                .collect();
            let _ = parse_incoming(&bytes);
        }
    }
}
