//! 계약 고정 시험: `fixtures/` 의 예시(설계 문서 6.3~6.6, 11.7)를 그대로 읽는다.
//! 같은 파일을 화면 쪽 `app/tests/contract.test.ts` 도 읽는다.

use epistle::*;
use serde_json::Value;

macro_rules! fx {
    ($name:literal) => {
        include_str!(concat!("../../../fixtures/", $name))
    };
}

fn params_or_result(line: &str, key: &str) -> Value {
    let v: Value = serde_json::from_str(line).unwrap();
    assert_eq!(v["jsonrpc"], "2.0");
    v[key].clone()
}

#[test]
fn hello_요청과_응답() {
    let h: Hello = serde_json::from_value(params_or_result(fx!("hello_request.json"), "params")).unwrap();
    assert_eq!(h.client, "antioch");
    assert_eq!(h.api, EPISTLE_API);

    let r: HelloReply =
        serde_json::from_value(params_or_result(fx!("hello_response.json"), "result")).unwrap();
    assert_eq!(r.granted, vec![Grant::Read, Grant::Operate]);
    assert_eq!(r.check("storyfirewall"), Ok(()));
    // 등록부의 슬롯과 다른 이름으로 답하면 거부
    assert_eq!(r.check("netx"), Err(Violation::ModuleMismatch));
}

#[test]
fn 판_협상() {
    assert!(api_compatible((1, 0)));
    assert!(api_compatible((1, 3)));
    assert!(!api_compatible((2, 0)));
    assert!(!api_compatible((0, 9)));
}

#[test]
fn 매니페스트() {
    let m: Manifest = serde_json::from_str(fx!("describe_storyfirewall.json")).unwrap();
    assert_eq!(m.kind, ModuleKind::Service);
    assert_eq!(m.widgets.len(), 2);
    assert_eq!(m.widgets[0].kind, WidgetKind::Stat);
    assert_eq!(m.actions[0].risk, Risk::Danger);
    assert_eq!(m.check("storyfirewall"), Ok(()));

    let w: Manifest = serde_json::from_str(fx!("describe_word.json")).unwrap();
    assert_eq!(w.kind, ModuleKind::Builtin);
    assert_eq!(w.widgets[0].kind, WidgetKind::Quote);
}

#[test]
fn 상태() {
    let h: Health = serde_json::from_str(fx!("health.json")).unwrap();
    assert_eq!(h.state, State::Ok);
    assert!(ts_is_seconds(h.since));
    assert_eq!(h.checks.len(), 2);
}

#[test]
fn 위젯_자료() {
    let w: Widgets = serde_json::from_str(fx!("widgets.json")).unwrap();
    match &w.widgets[0] {
        Widget::Stat { id, value, unit, trend, state, open } => {
            assert_eq!(id, "blocked_today");
            assert_eq!(*value, 37.0);
            assert_eq!(unit.as_deref(), Some("건"));
            assert_eq!(*trend, Some(Trend::Up));
            assert_eq!(*state, Some(Tone::Caution));
            assert_eq!(open.as_ref().unwrap().view, "blocked");
        }
        other => panic!("stat 이 아님: {other:?}"),
    }
    match &w.widgets[1] {
        Widget::List { items, state, .. } => {
            assert_eq!(*state, Some(Tone::Danger));
            assert_eq!(items[0].open.as_ref().unwrap().reference.as_deref(), Some("4812"));
        }
        other => panic!("list 가 아님: {other:?}"),
    }

    let q: Widgets = serde_json::from_str(fx!("widgets_word.json")).unwrap();
    assert!(matches!(&q.widgets[0], Widget::Quote { reference, .. } if reference == "요한복음 3:16"));

    let p: Widgets = serde_json::from_str(fx!("widgets_proposed.json")).unwrap();
    assert!(matches!(p.widgets[0], Widget::Status { state: Tone::Ok, .. }));
    assert!(matches!(&p.widgets[1], Widget::Spark { points, .. } if points.len() == 6));
    assert!(matches!(&p.widgets[2], Widget::Timeline { items, .. } if ts_is_seconds(items[0].ts)));
}

#[test]
fn 위젯에_html을_싣는_칸이_없다() {
    // 모르는 kind 는 타입으로 읽히지 않는다 — 섀시는 그 칸을 그리지 않는다
    let bad = r#"{"widgets":[{"id":"x","kind":"html","html":"<script>1</script>"}]}"#;
    assert!(serde_json::from_str::<Widgets>(bad).is_err());
}

#[test]
fn 사건_알림() {
    let v: Value = serde_json::from_str(fx!("event_notification.json")).unwrap();
    assert_eq!(v["method"], "rack.event");
    assert!(v.get("id").is_none(), "알림에는 봉투 id 가 없다");
    let e: Event = serde_json::from_value(v["params"].clone()).unwrap();
    assert_eq!(e.severity, Severity::Warn);
    assert_eq!(e.entities.len(), 2);
    assert!(matches!(e.entities[1], Entity::Endpoint { port: Some(443), .. }));
    assert_eq!(e.suggest[0].to, "netx");
    assert_eq!(e.check("storyfirewall"), Ok(()));

    // 접점: 다른 모듈의 연결로 들어온 사건, id 접두사 위조, 밀리초 시각
    assert_eq!(e.check("netx"), Err(Violation::ModuleMismatch));
    let mut forged = e.clone();
    forged.id = "netx:evt:1".into();
    assert_eq!(forged.check("storyfirewall"), Err(Violation::BadEventId));
    let mut ms = e.clone();
    ms.ts = 1_794_355_321_000;
    assert_eq!(ms.check("storyfirewall"), Err(Violation::BadTimestamp));
    let mut odd = e.clone();
    odd.id = "storyfirewall:evt:<b>".into();
    assert_eq!(odd.check("storyfirewall"), Err(Violation::BadEventId));
}

#[test]
fn 모듈_id와_자르기() {
    assert!(valid_module_id("storyfirewall"));
    assert!(valid_module_id("net_x2"));
    for bad in ["", "NetX", "a-b", r"..\x", "a:b", &"a".repeat(33)] {
        assert!(!valid_module_id(bad), "{bad}");
    }
    assert_eq!(clip("가나다라", 2), "가나");
    assert_eq!(clip("ab", 80), "ab");
}

#[test]
fn 다시_적어도_같은_모양() {
    for (name, text) in [
        ("describe", fx!("describe_storyfirewall.json")),
        ("health", fx!("health.json")),
    ] {
        let v: Value = serde_json::from_str(text).unwrap();
        let back = match name {
            "describe" => serde_json::to_value(serde_json::from_value::<Manifest>(v.clone()).unwrap()),
            _ => serde_json::to_value(serde_json::from_value::<Health>(v.clone()).unwrap()),
        }
        .unwrap();
        assert_eq!(v, back, "{name}");
    }
}
