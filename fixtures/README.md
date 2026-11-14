# 계약 고정 예시 (Rust·TypeScript 공용)

`crates/epistle` 시험과 `app/tests` 가 같은 파일을 읽는다. 한쪽만 고치면 다른 쪽 시험이 깨진다.

- `widgets_proposed.json` 을 뺀 나머지는 통합프로젝트 설계 문서 0.19판 초안 6.3~6.6, 11.7 의 예시를 옮긴 것이다.
- 한 가지가 다르다: 위젯·목록 항목의 색조 칸은 `state` 가 아니라 **`tone`** 이다(「변경 제안 — 에피스틀 6장 (2판)」, 기획자 확정 2026-11-15). `rack.health` 의 `state` 는 그대로다.
- `widgets_proposed.json` (status·spark·timeline 의 자료 모양)은 설계 문서에 예시가 없어 이 저장소가 정한 **제안**이다.
