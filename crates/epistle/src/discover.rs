//! 슬롯 발견 (설계 5장, 8.3 확정). 등록부를 읽는 일과 판단을 나눴다:
//! 읽기는 `RegistryView`/`Host`(Windows 구현은 `win`), 판단은 여기 — 순수 함수라 Linux 에서 시험한다.
//!
//! 등록부 값은 모두 **자료**다. 여기서 형식을 보고, 통과한 값도 경로·명령으로 실행하지 않는다.

use crate::names::{under_admin_root, valid_pipe_name};
use crate::{valid_module_id, ModuleKind, EPISTLE_API};

/// 한 번에 다루는 슬롯 수의 상한
pub const MAX_SLOTS: usize = 32;

/// `HKLM\SOFTWARE\Antioch\Modules\<id>` 의 값들(읽은 그대로)
#[derive(Debug, Clone, Default)]
pub struct RawModule {
    /// 하위 키 이름
    pub id: String,
    pub exe: Option<String>,
    pub kind: Option<String>,
    pub api: Option<String>,
    /// 서비스형이 올리는 실행 중 파이프 이름
    pub pipe_name: Option<String>,
}

pub trait RegistryView {
    /// HKLM 의 모듈 등록(관리자만 쓸 수 있는 자리). **슬롯은 여기서만 생긴다**
    fn modules(&self) -> Vec<RawModule>;
    /// HKCU 의 `Running\<id>\PipeName` (앱형 모듈이 실행 중에 쓰는 값. 일반 권한 프로그램이 고칠 수 있다)
    fn running_pipe(&self, id: &str) -> Option<String>;
}

pub trait Host {
    /// 운영체제가 알려 주는 최종 경로(짧은 이름·링크를 푼 것). 파일이 없으면 None
    fn final_path(&self, path: &str) -> Option<String>;
    /// 관리자만 쓸 수 있는 설치 폴더들(Program Files 등) — 환경 변수가 아니라 운영체제에 물은 값
    fn admin_roots(&self) -> Vec<String>;
}

/// 붙지 않는 까닭. 화면에 그대로 보인다
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Distrust {
    /// Exe 값이 없거나 그 파일이 없다
    ExeMissing,
    /// 실행 파일이 관리자 전용 폴더 아래가 아니다(8.3 의 4번)
    NotAdminFolder,
    /// Kind 값이 service·app 이 아니다
    BadKind,
    /// 판(메이저)이 다르다
    ApiMismatch,
    /// 공개된 파이프 이름의 모양이 틀리다(누가 값을 고쳤을 수 있다)
    BadPipeName,
}

impl Distrust {
    pub fn message(self) -> &'static str {
        match self {
            Distrust::ExeMissing => "등록된 실행 파일을 찾지 못했습니다",
            Distrust::NotAdminFolder => "등록된 실행 파일이 Program Files 아래에 있지 않습니다(PC 전체 설치가 필요합니다)",
            Distrust::BadKind => "등록 정보의 종류(Kind) 값이 올바르지 않습니다",
            Distrust::ApiMismatch => "이 모듈의 규격 판이 맞지 않습니다",
            Distrust::BadPipeName => "공개된 파이프 이름이 올바르지 않습니다(비정상 값)",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlotState {
    /// 등록돼 있고 실행 중이 아니다
    Off,
    /// 실행 중: 이 파이프에 붙는다
    Running { pipe: String },
    /// 붙지 않는다
    Untrusted(Distrust),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Slot {
    pub id: String,
    pub kind: ModuleKind,
    /// 등록된 실행 파일의 최종 경로. 파이프 서버 프로세스가 이 파일인지 대조한다. Untrusted 면 빈 값일 수 있다
    pub exe_final: String,
    pub state: SlotState,
}

fn parse_api(s: &str) -> Option<(u32, u32)> {
    let (a, b) = s.trim().split_once('.')?;
    Some((a.parse().ok()?, b.parse().ok()?))
}

/// 등록부를 읽어 슬롯 목록을 만든다.
/// - id 형식이 틀린 등록(대문자 포함)과 중복은 뺀다.
/// - 서비스형의 파이프 이름은 HKLM 값만, 앱형은 HKCU 값만 본다. HKLM 에 등록되지 않은 id 의 HKCU 값은 읽지도 않는다.
pub fn discover(reg: &dyn RegistryView, host: &dyn Host) -> Vec<Slot> {
    let roots = host.admin_roots();
    let mut out: Vec<Slot> = Vec::new();
    for m in reg.modules() {
        if out.len() >= MAX_SLOTS {
            break;
        }
        if !valid_module_id(&m.id) || out.iter().any(|s| s.id == m.id) {
            continue;
        }
        let kind = match m.kind.as_deref().map(str::trim) {
            Some("service") => ModuleKind::Service,
            Some("app") => ModuleKind::App,
            _ => {
                out.push(Slot { id: m.id, kind: ModuleKind::App, exe_final: String::new(), state: SlotState::Untrusted(Distrust::BadKind) });
                continue;
            }
        };
        let untrusted = |why, exe_final: String| Slot { id: m.id.clone(), kind, exe_final, state: SlotState::Untrusted(why) };

        let Some(exe_final) = m.exe.as_deref().and_then(|e| host.final_path(e)) else {
            out.push(untrusted(Distrust::ExeMissing, String::new()));
            continue;
        };
        if !under_admin_root(&exe_final, &roots) {
            out.push(untrusted(Distrust::NotAdminFolder, exe_final));
            continue;
        }
        // Api 값이 있으면 메이저를 본다. 없으면 rack.hello 의 판 협상에 맡긴다
        if let Some(api) = m.api.as_deref() {
            if parse_api(api).map(|a| a.0) != Some(EPISTLE_API.0) {
                out.push(untrusted(Distrust::ApiMismatch, exe_final));
                continue;
            }
        }
        let pipe = match kind {
            ModuleKind::Service => m.pipe_name.clone(),
            _ => reg.running_pipe(&m.id),
        };
        let state = match pipe {
            None => SlotState::Off,
            Some(p) if valid_pipe_name(&m.id, &p) => SlotState::Running { pipe: p },
            Some(_) => SlotState::Untrusted(Distrust::BadPipeName),
        };
        out.push(Slot { id: m.id, kind, exe_final, state });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct Reg {
        hklm: Vec<RawModule>,
        hkcu: HashMap<String, String>,
        asked: std::cell::RefCell<Vec<String>>,
    }
    impl RegistryView for Reg {
        fn modules(&self) -> Vec<RawModule> {
            self.hklm.clone()
        }
        fn running_pipe(&self, id: &str) -> Option<String> {
            self.asked.borrow_mut().push(id.to_string());
            self.hkcu.get(id).cloned()
        }
    }
    struct Pc;
    impl Host for Pc {
        fn final_path(&self, p: &str) -> Option<String> {
            // 흉내: 짧은 이름을 풀어 준다. "없음" 이 들어간 경로는 없는 파일
            (!p.contains("없음")).then(|| p.replace("PROGRA~1", "Program Files"))
        }
        fn admin_roots(&self) -> Vec<String> {
            vec![r"C:\Program Files".into()]
        }
    }

    const SF_PIPE: &str = r"\\.\pipe\storyfirewall-00112233445566778899aabbccddeeff";
    const NX_PIPE: &str = r"\\.\pipe\netx-00112233445566778899aabbccddeeff";

    fn module(id: &str, kind: &str, exe: &str) -> RawModule {
        RawModule { id: id.into(), exe: Some(exe.into()), kind: Some(kind.into()), api: Some("1.0".into()), pipe_name: None }
    }
    fn reg(hklm: Vec<RawModule>, hkcu: &[(&str, &str)]) -> Reg {
        Reg { hklm, hkcu: hkcu.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect(), asked: Default::default() }
    }
    fn state(slots: &[Slot], id: &str) -> SlotState {
        slots.iter().find(|s| s.id == id).unwrap().state.clone()
    }

    #[test]
    fn 서비스형은_hklm_앱형은_hkcu의_파이프_이름() {
        let mut sf = module("storyfirewall", "service", r"C:\PROGRA~1\StoryFirewall\svc.exe");
        sf.pipe_name = Some(SF_PIPE.into());
        let r = reg(vec![sf, module("netx", "app", r"C:\Program Files\NetX\netx.exe")], &[("netx", NX_PIPE)]);
        let slots = discover(&r, &Pc);
        assert_eq!(state(&slots, "storyfirewall"), SlotState::Running { pipe: SF_PIPE.into() });
        assert_eq!(slots[0].exe_final, r"C:\Program Files\StoryFirewall\svc.exe", "최종 경로로 맞춘다");
        assert_eq!(slots[0].kind, ModuleKind::Service);
        assert_eq!(state(&slots, "netx"), SlotState::Running { pipe: NX_PIPE.into() });
    }

    #[test]
    fn hkcu만으로는_슬롯이_생기지_않고_서비스형은_hkcu를_보지_않는다() {
        // 공격: 일반 권한 프로그램이 HKCU 에 가짜 모듈과, 서비스형 모듈의 가짜 파이프 이름을 써 둔다
        let r = reg(
            vec![module("storyfirewall", "service", r"C:\Program Files\StoryFirewall\svc.exe")],
            &[("storyfirewall", SF_PIPE), ("fake_fw", r"\\.\pipe\fake_fw-00112233445566778899aabbccddeeff")],
        );
        let slots = discover(&r, &Pc);
        assert_eq!(slots.len(), 1);
        assert_eq!(state(&slots, "storyfirewall"), SlotState::Off, "HKLM 에 파이프 이름이 없으면 꺼짐");
        assert!(r.asked.borrow().is_empty(), "HKCU 를 읽지도 않는다");
    }

    #[test]
    fn 꺼짐과_비정상_파이프_이름() {
        let nx = module("netx", "app", r"C:\Program Files\NetX\netx.exe");
        assert_eq!(state(&discover(&reg(vec![nx.clone()], &[]), &Pc), "netx"), SlotState::Off);
        for bad in [r"\\evil\pipe\netx-00112233445566778899aabbccddeeff", r"\\.\pipe\storyfirewall-00112233445566778899aabbccddeeff", "x"] {
            let slots = discover(&reg(vec![nx.clone()], &[("netx", bad)]), &Pc);
            assert_eq!(state(&slots, "netx"), SlotState::Untrusted(Distrust::BadPipeName), "{bad}");
        }
    }

    #[test]
    fn 믿지_않는_등록() {
        let cases = [
            (module("a", "app", r"C:\Users\me\AppData\Local\A\a.exe"), Distrust::NotAdminFolder),
            (module("b", "app", r"C:\Program Files\없음\b.exe"), Distrust::ExeMissing),
            (RawModule { id: "c".into(), kind: Some("app".into()), ..Default::default() }, Distrust::ExeMissing),
            (module("d", "driver", r"C:\Program Files\D\d.exe"), Distrust::BadKind),
            (RawModule { api: Some("2.0".into()), ..module("e", "app", r"C:\Program Files\E\e.exe") }, Distrust::ApiMismatch),
            (RawModule { api: Some("최신".into()), ..module("f", "app", r"C:\Program Files\F\f.exe") }, Distrust::ApiMismatch),
        ];
        for (m, why) in cases {
            let id = m.id.clone();
            // 파이프 이름이 멀쩡해도 붙지 않는다
            let pipe = format!(r"\\.\pipe\{id}-00112233445566778899aabbccddeeff");
            let slots = discover(&reg(vec![m], &[(&id, &pipe)]), &Pc);
            assert_eq!(state(&slots, &id), SlotState::Untrusted(why), "{id}");
        }
    }

    #[test]
    fn id_형식과_중복과_개수_상한() {
        let exe = r"C:\Program Files\X\x.exe";
        let r = reg(vec![module("netx", "app", exe), module("NetX", "app", exe), module("a-b", "app", exe), module(r"..\x", "app", exe), module("netx", "service", exe)], &[]);
        let slots = discover(&r, &Pc);
        assert_eq!(slots.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["netx"]);
        assert_eq!(slots[0].kind, ModuleKind::App, "먼저 읽은 등록이 남는다");

        let many: Vec<RawModule> = (0..100).map(|i| module(&format!("m{i}"), "app", exe)).collect();
        assert_eq!(discover(&reg(many, &[]), &Pc).len(), MAX_SLOTS);
    }
}
