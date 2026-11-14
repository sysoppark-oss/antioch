# 안디옥 (Antioch)

스토리방화벽·NetX 등을 모듈로 꽂는 통합 화면(섀시)과 통신 규격 에피스틀(Epistle)의 저장소.
기획·설계 원본은 구글 드라이브 `05_개발/통합프로젝트` 폴더다("00_목차"만 먼저 읽는다). 작업 원칙은 스토리방화벽 저장소의 `CLAUDE.md` 를 따른다.

섀시는 일반 권한으로 돌고 자기 기능이 없다. 모듈이 준 **자료만** 받아 그린다(모듈이 HTML·스크립트를 보내는 길은 없다).
변경은 언제나 주인 모듈이 자기 확인 창으로 승인받는다.

## 구조

```
crates/epistle/   Epistle 1.0 rack.* 타입과 검사 (Rust, 운영체제 호출 없음)
fixtures/         계약 고정 예시 — Rust·화면 시험이 같은 파일을 읽는다
app/              섀시 화면 (Vite + TypeScript, 3단계에서 Tauri 2 껍데기를 씌운다)
  src/epistle/    화면 쪽 타입과 걸러 내기(validate.ts)
  src/feed/       랙을 보는 창구(feed.ts)와 로컬 목업 피드(mock.ts)
  src/core/       섀시 상태 관리: 발견 → 연결 → 구독, 끊김 뒤 재동기화
  src/ui/         홀로그램 콘솔(3D 코버플로우)과 패널
  src/word/       오늘의 말씀 내장 슬롯
word/             말씀 자료 묶음 (글자를 고치지 않는다 — word/README.md)
docs/             검증 의뢰서 사본 (기준은 드라이브)
```

## 돌려 보기

```
cargo test                      # 계약 시험 (Linux 에서 전부 돈다)
cd app && npm install
npm test                        # 화면 쪽 계약·섀시·화면 시험
npm run dev                     # http://localhost:1420 — 목업 피드에 붙은 콘솔
```

## 단계

1. **(지금)** rack.* 자료 계약 고정 + 목업 피드에 붙은 홀로그램 콘솔
2. epistle 크레이트에 전송(네임드 파이프 + 한 줄 JSON-RPC 2.0)·등록부 발견·구성원/서버 확인
3. 목업 피드를 epistle 실클라이언트로 교체 (Tauri 2)
4. 스토리방화벽에 읽기 전용 rack.* 송출 → 섀시에 실데이터

## 라이선스

AGPL-3.0-only. (`epistle` 크레이트를 따로 허용적 라이선스로 둘지는 미정 — 설계 문서 10장 6번)
