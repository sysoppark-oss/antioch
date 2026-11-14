//! epistle: Epistle 1.0 의 rack.* 자료 계약.
//! 기준: 통합프로젝트 「안디옥 플랫폼 설계와 에피스틀 규격 (0.19판 초안)」 6.1·6.3(확정), 6.4~6.6·7장(초안).
//!
//! 타입과 검사(이 파일), 봉투(`rpc`), 클라이언트(`client`), 슬롯 발견(`discover`), 이름·경로 규칙(`names`)은
//! 운영체제를 부르지 않아 Linux 에서 시험한다. 등록부·네임드 파이프·서버 확인은 `win`(Windows 전용)에 있다.
//! 화면 쪽 짝은 `app/src/epistle/` 이고, 두 쪽이 `fixtures/` 의 같은 예시 파일로 시험한다.
//!
//! 모듈에서 온 값은 모두 **자료**다. 여기 검사를 통과해도 글자로만 표시한다.

pub mod client;
pub mod discover;
pub mod names;
pub mod rpc;
#[cfg(windows)]
pub mod win;

use serde::{Deserialize, Serialize};
use std::net::IpAddr;

/// Epistle 판 (메이저, 마이너)
pub const EPISTLE_API: (u32, u32) = (1, 0);

/// 6.5: 제목 80자, 보조 200자. 넘으면 받는 쪽이 자른다(거부하지 않는다).
pub const MAX_TITLE: usize = 80;
pub const MAX_SUB: usize = 200;
/// 6.1: 목록 한 쪽 최대 1,000건 — 위젯 목록은 화면용이라 더 좁게 둔다 [제안]
pub const MAX_LIST_ITEMS: usize = 50;
pub const MAX_SPARK_POINTS: usize = 240;
/// 조작 이름은 짧게 — 긴 유도 문구를 실을 자리를 주지 않는다
pub const MAX_ACTION_TITLE: usize = 24;
/// 수치의 절댓값 상한. 넘는 값은 그 칸을 버린다
pub const MAX_ABS_NUMBER: f64 = 1e15;
/// 사건 시각의 허용 창: 지금보다 60초 넘게 앞서거나 30일 넘게 지났으면 받은 시각으로 고쳐 적는다
pub const TS_FUTURE_SEC: i64 = 60;
pub const TS_PAST_SEC: i64 = 30 * 86_400;
/// 6.1: 알림은 모듈당 초당 10건 이하
pub const MAX_EVENTS_PER_SEC: u32 = 10;
/// 시각은 epoch **초**다. 밀리초(지금 기준 약 1.8e12)가 들어오면 이 상한에 걸린다 (2100-01-01)
pub const MAX_TS: i64 = 4_102_444_800;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Grant {
    Read,
    Operate,
    Admin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Ok,
    Degraded,
    Down,
    Starting,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Notice,
    Warn,
    Critical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Risk {
    Safe,
    Caution,
    Danger,
    Forbidden,
}

/// 위젯·항목의 색조(칸 이름 `tone`). `rack.health` 의 `state` 와 값 집합이 달라 이름을 나눴다 [확정 2026-11-15 — 변경 제안 2판 1장]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tone {
    Ok,
    Caution,
    Danger,
    Off,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Trend {
    Up,
    Down,
    Flat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModuleKind {
    Service,
    App,
    Builtin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WidgetKind {
    Stat,
    Status,
    List,
    Spark,
    Timeline,
    Quote,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Hello {
    pub client: String,
    pub client_version: String,
    pub api: (u32, u32),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HelloReply {
    pub module: String,
    pub module_version: String,
    pub api: (u32, u32),
    pub granted: Vec<Grant>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub id: String,
    pub title: String,
    pub kind: ModuleKind,
    pub version: String,
    pub api: (u32, u32),
    pub summary: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub namespaces: Vec<String>,
    #[serde(default)]
    pub widgets: Vec<WidgetSpec>,
    #[serde(default)]
    pub intents_in: Vec<String>,
    #[serde(default)]
    pub intents_out: Vec<String>,
    #[serde(default)]
    pub actions: Vec<ActionSpec>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WidgetSpec {
    pub id: String,
    pub title: String,
    pub kind: WidgetKind,
    #[serde(default)]
    pub sizes: Vec<String>,
    pub refresh_sec: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionSpec {
    pub id: String,
    pub title: String,
    pub risk: Risk,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Health {
    pub state: State,
    /// epoch 초
    pub since: i64,
    #[serde(default)]
    pub notes: Vec<String>,
    #[serde(default)]
    pub checks: Vec<Check>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Check {
    pub id: String,
    pub state: State,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpenRef {
    pub view: String,
    #[serde(default, rename = "ref", skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
}

/// rack.widgets 응답
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Widgets {
    pub widgets: Vec<Widget>,
}

/// 위젯 한 칸의 자료. 모듈은 자료만 주고 그리기는 섀시가 한다(6.5). HTML·스크립트를 싣는 칸은 없다.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Widget {
    Stat {
        id: String,
        value: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        unit: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        trend: Option<Trend>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tone: Option<Tone>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        open: Option<OpenRef>,
    },
    /// [제안] 자료 모양
    Status {
        id: String,
        tone: Tone,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        open: Option<OpenRef>,
    },
    List {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tone: Option<Tone>,
        #[serde(default)]
        items: Vec<ListItem>,
    },
    /// [제안] 자료 모양
    Spark {
        id: String,
        points: Vec<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        unit: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tone: Option<Tone>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        open: Option<OpenRef>,
    },
    /// [제안] 자료 모양
    Timeline {
        id: String,
        #[serde(default)]
        items: Vec<TimelineItem>,
    },
    Quote {
        id: String,
        text: String,
        #[serde(rename = "ref")]
        reference: String,
        version: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        open: Option<OpenRef>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ListItem {
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sub: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tone: Option<Tone>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open: Option<OpenRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimelineItem {
    /// epoch 초
    pub ts: i64,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tone: Option<Tone>,
}

/// 모듈끼리 주고받는 대상의 공용어 (6.6)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Entity {
    Process {
        pid: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        image: Option<String>,
    },
    Endpoint {
        ip: IpAddr,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        port: Option<u16>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        proto: Option<String>,
    },
    Host {
        name: String,
    },
    Domain {
        name: String,
    },
    File {
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sha256: Option<String>,
    },
    Rule {
        owner: String,
        id: String,
    },
    Device {
        ip: IpAddr,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mac: Option<String>,
    },
}

/// 알림 rack.event 의 params (6.6)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    /// `<module>:evt:<모듈 안에서 유일한 값>`
    pub id: String,
    /// epoch 초
    pub ts: i64,
    pub module: String,
    pub severity: Severity,
    pub kind: String,
    pub title: String,
    #[serde(default)]
    pub entities: Vec<Entity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open: Option<OpenRef>,
    #[serde(default)]
    pub suggest: Vec<Suggest>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Suggest {
    pub intent: String,
    pub to: String,
    pub label: String,
}

/// 계약 위반의 종류. 화면 쪽 `validate.ts` 와 같은 규칙이다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Violation {
    BadModuleId,
    /// 응답·사건의 모듈 이름이 등록부의 슬롯 id 와 다름(남의 이름으로 말함)
    ModuleMismatch,
    BadEventId,
    /// 시각이 epoch 초 범위가 아님(밀리초 등)
    BadTimestamp,
    ApiMismatch,
}

/// 모듈 id: 영문 소문자·숫자·`_`, 1~32자. 등록부 키 이름·파이프 이름·사건 id 에 그대로 쓰인다.
pub fn valid_module_id(id: &str) -> bool {
    (1..=32).contains(&id.len())
        && id
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_')
}

/// 시각이 epoch 초인가
pub fn ts_is_seconds(ts: i64) -> bool {
    ts > 0 && ts < MAX_TS
}

/// 사건 시각이 허용 창 안인가. 밖이면 받는 쪽이 받은 시각으로 고쳐 적는다(버리지 않는다)
pub fn ts_in_window(ts: i64, now: i64) -> bool {
    ts <= now + TS_FUTURE_SEC && ts >= now - TS_PAST_SEC
}

/// 그림·표기에 써도 되는 수인가
pub fn number_ok(v: f64) -> bool {
    v.is_finite() && v.abs() <= MAX_ABS_NUMBER
}

/// 판 협상(접속자 쪽): 메이저가 같고 모듈의 마이너가 내 것 이상이어야 한다 (6.3)
pub fn api_compatible(module_api: (u32, u32)) -> bool {
    module_api.0 == EPISTLE_API.0 && module_api.1 >= EPISTLE_API.1
}

/// 글자 수(유니코드 스칼라) 기준으로 자른다. 바이트 경계에서 죽지 않는다.
pub fn clip(s: &str, max_chars: usize) -> String {
    s.chars().take(max_chars).collect()
}

impl HelloReply {
    /// `slot_id`: 섀시가 등록부에서 읽은 슬롯 id
    pub fn check(&self, slot_id: &str) -> Result<(), Violation> {
        if !valid_module_id(slot_id) {
            return Err(Violation::BadModuleId);
        }
        if self.module != slot_id {
            return Err(Violation::ModuleMismatch);
        }
        if !api_compatible(self.api) {
            return Err(Violation::ApiMismatch);
        }
        Ok(())
    }
}

impl Manifest {
    pub fn check(&self, slot_id: &str) -> Result<(), Violation> {
        if self.id != slot_id {
            return Err(Violation::ModuleMismatch);
        }
        Ok(())
    }
}

impl Event {
    /// `slot_id`: 이 사건을 보낸 연결의 슬롯 id. 모듈은 자기 이름으로만 사건을 낼 수 있다.
    pub fn check(&self, slot_id: &str) -> Result<(), Violation> {
        if !valid_module_id(slot_id) {
            return Err(Violation::BadModuleId);
        }
        if self.module != slot_id {
            return Err(Violation::ModuleMismatch);
        }
        let rest = self
            .id
            .strip_prefix(slot_id)
            .and_then(|r| r.strip_prefix(":evt:"))
            .ok_or(Violation::BadEventId)?;
        if rest.is_empty()
            || rest.len() > 64
            || !rest
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_' || c == b'.')
        {
            return Err(Violation::BadEventId);
        }
        if !ts_is_seconds(self.ts) {
            return Err(Violation::BadTimestamp);
        }
        Ok(())
    }
}
