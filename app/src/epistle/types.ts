// Epistle 1.0 rack.* 자료 계약 — 화면 쪽 타입.
// Rust 쪽 짝: crates/epistle/src/lib.rs. 두 쪽이 fixtures/ 의 같은 예시로 시험한다.

export const EPISTLE_API: readonly [number, number] = [1, 0];

export const LIMITS = {
  title: 80, // 6.5
  sub: 200, // 6.5
  listItems: 50,
  sparkPoints: 240,
  widgets: 24,
  checks: 16,
  notes: 8,
  actions: 16,
  /** 조작 이름은 짧게 — 긴 유도 문구를 실을 자리를 주지 않는다 */
  actionTitle: 24,
  /** 수치의 절댓값 상한. 넘는 값은 그 칸을 버린다(그림·표기가 깨지지 않게) */
  maxAbsNumber: 1e15,
  quote: 2000,
  /** 시각은 epoch 초. 밀리초가 들어오면 이 상한에 걸린다 (2100-01-01) */
  maxTs: 4_102_444_800,
  /** 6.1: 모듈당 초당 10건 */
  eventsPerSec: 10,
} as const;

export type Grant = "read" | "operate" | "admin";
export type State = "ok" | "degraded" | "down" | "starting";
export type Severity = "info" | "notice" | "warn" | "critical";
export type Risk = "safe" | "caution" | "danger" | "forbidden";
export type Tone = "ok" | "caution" | "danger" | "off";
export type Trend = "up" | "down" | "flat";
export type ModuleKind = "service" | "app" | "builtin";
export type WidgetKind = "stat" | "status" | "list" | "spark" | "timeline" | "quote";

export interface HelloReply {
  module: string;
  module_version: string;
  api: [number, number];
  granted: Grant[];
}

export interface WidgetSpec {
  id: string;
  title: string;
  kind: WidgetKind;
  refresh_sec: number;
}

export interface ActionSpec {
  id: string;
  title: string;
  risk: Risk;
}

export interface Manifest {
  id: string;
  title: string;
  kind: ModuleKind;
  version: string;
  api: [number, number];
  summary: string;
  capabilities: string[];
  widgets: WidgetSpec[];
  actions: ActionSpec[];
}

export interface Health {
  state: State;
  /** epoch 초 */
  since: number;
  notes: string[];
  checks: { id: string; state: State }[];
}

export interface OpenRef {
  view: string;
  ref?: string;
}

export interface ListItem {
  title: string;
  sub?: string;
  state?: Tone;
  open?: OpenRef;
}

export interface TimelineItem {
  ts: number;
  title: string;
  state?: Tone;
}

export type Widget =
  | { kind: "stat"; id: string; value: number; unit?: string; trend?: Trend; state?: Tone; open?: OpenRef }
  | { kind: "status"; id: string; state: Tone; label?: string; open?: OpenRef }
  | { kind: "list"; id: string; state?: Tone; items: ListItem[] }
  | { kind: "spark"; id: string; points: number[]; unit?: string; state?: Tone; open?: OpenRef }
  | { kind: "timeline"; id: string; items: TimelineItem[] }
  | { kind: "quote"; id: string; text: string; ref: string; version: string; open?: OpenRef };

export interface RackEvent {
  /** `<module>:evt:<모듈 안에서 유일한 값>` */
  id: string;
  /** epoch 초 */
  ts: number;
  module: string;
  severity: Severity;
  kind: string;
  title: string;
  open?: OpenRef;
}
