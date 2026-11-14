// 모듈에서 온 값(unknown)을 계약대로 걸러 내는 곳. 섀시는 이 문을 지난 값만 그린다.
// 규칙: 형식이 틀린 필수 칸 → 그 항목을 버린다(null). 길이는 자른다. 모르는 칸·모르는 위젯 종류는 무시한다.
// 여기서 나온 문자열도 **자료**다 — 화면은 textContent 로만 넣는다.

import {
  EPISTLE_API,
  LIMITS,
  type ActionSpec,
  type Grant,
  type Health,
  type HelloReply,
  type ListItem,
  type Manifest,
  type ModuleKind,
  type OpenRef,
  type RackEvent,
  type Risk,
  type Severity,
  type State,
  type TimelineItem,
  type Tone,
  type Trend,
  type Widget,
  type WidgetKind,
  type WidgetSpec,
} from "./types";

type Obj = Record<string, unknown>;

const isObj = (v: unknown): v is Obj => typeof v === "object" && v !== null && !Array.isArray(v);
const arr = (v: unknown): unknown[] => (Array.isArray(v) ? v : []);

// 빼는 글자: C0·C1 제어 문자(탭·줄바꿈 제외), 폭 없는 문자(U+200B~200F, U+FEFF), 줄·문단 구분(U+2028·2029),
// 글자 방향을 바꾸는 문자(U+061C, U+202A~202E, U+2066~2069). 소스에는 이스케이프로만 적는다
// eslint-disable-next-line no-control-regex
const STRIP = /[\u0000-\u0008\u000b-\u001f\u007f-\u009f\u061c\u200b-\u200f\u2028-\u202e\u2066-\u2069\ufeff]/g;
/** 한 글자에 겹쳐 쌓는 결합 부호는 둘까지만 둔다(옆 칸을 덮는 글자 방지) */
const STACKED = /(\p{M}{2})\p{M}+/gu;
/** 그림 글자. 이름표에서는 뺀다 — 상태등 흉내(초록 동그라미 등)를 못 하게 */
const PICTO = /[\p{Extended_Pictographic}\ufe0f\u20e3]/gu;

/** 글자 수(코드 포인트) 기준으로 자른다. 제어·폭 없는·방향 전환 문자를 빼고 겹친 결합 부호를 줄인다. 줄바꿈은 둔다. */
export function clip(s: string, max: number): string {
  const clean = s.replace(STRIP, "").replace(STACKED, "$1");
  const cps = Array.from(clean);
  return cps.length > max ? cps.slice(0, max).join("") : clean;
}

/** 이름표(제목·보조·단위)용: 한 줄로 만들고 그림 글자를 뺀 뒤 자른다 */
export function label(s: string, max: number): string {
  return clip(s.replace(/[\t\n]+/g, " ").replace(PICTO, ""), max).trim();
}

function str(v: unknown, max: number): string | null {
  return typeof v === "string" ? label(v, max) : null;
}
function optStr(v: unknown, max: number): string | undefined {
  return typeof v === "string" ? label(v, max) : undefined;
}
function oneOf<T extends string>(v: unknown, set: readonly T[]): T | null {
  return typeof v === "string" && (set as readonly string[]).includes(v) ? (v as T) : null;
}
function num(v: unknown): number | null {
  return typeof v === "number" && Number.isFinite(v) && Math.abs(v) <= LIMITS.maxAbsNumber ? v : null;
}

const STATES: readonly State[] = ["ok", "degraded", "down", "starting"];
const SEVERITIES: readonly Severity[] = ["info", "notice", "warn", "critical"];
const RISKS: readonly Risk[] = ["safe", "caution", "danger", "forbidden"];
const TONES: readonly Tone[] = ["ok", "caution", "danger", "off"];
const TRENDS: readonly Trend[] = ["up", "down", "flat"];
const KINDS: readonly ModuleKind[] = ["service", "app", "builtin"];
const WIDGET_KINDS: readonly WidgetKind[] = ["stat", "status", "list", "spark", "timeline", "quote"];
const GRANTS: readonly Grant[] = ["read", "operate", "admin"];

/** 모듈 id: 영문 소문자·숫자·`_`, 1~32자 */
export function validModuleId(id: unknown): id is string {
  return typeof id === "string" && /^[a-z0-9_]{1,32}$/.test(id);
}
const ID_RE = /^[A-Za-z0-9_.-]{1,64}$/;
const localId = (v: unknown): string | null => (typeof v === "string" && ID_RE.test(v) ? v : null);

/** 시각이 epoch 초인가 (밀리초·음수·소수는 아님) */
export function tsIsSeconds(v: unknown): v is number {
  return typeof v === "number" && Number.isInteger(v) && v > 0 && v < LIMITS.maxTs;
}

function api(v: unknown): [number, number] | null {
  if (!Array.isArray(v) || v.length !== 2) return null;
  const [a, b] = v as unknown[];
  return Number.isInteger(a) && Number.isInteger(b) && (a as number) >= 0 && (b as number) >= 0
    ? [a as number, b as number]
    : null;
}

/** 판 협상(접속자 쪽): 메이저가 같고 모듈의 마이너가 내 것 이상 (6.3) */
export function apiCompatible(a: [number, number]): boolean {
  return a[0] === EPISTLE_API[0] && a[1] >= EPISTLE_API[1];
}

function openRef(v: unknown): OpenRef | undefined {
  if (!isObj(v)) return undefined;
  const view = localId(v.view);
  if (!view) return undefined;
  const ref = localId(v.ref);
  return ref ? { view, ref } : { view };
}

/** rack.hello 응답. `slotId` 는 등록부에서 읽은 슬롯 id — 다른 이름으로 답하면 거부한다. */
export function parseHelloReply(v: unknown, slotId: string): HelloReply | null {
  if (!isObj(v) || !validModuleId(slotId)) return null;
  const a = api(v.api);
  const ver = str(v.module_version, 40);
  if (v.module !== slotId || !a || ver === null || !apiCompatible(a)) return null;
  const granted = arr(v.granted)
    .map((g) => oneOf(g, GRANTS))
    .filter((g): g is Grant => g !== null);
  return { module: slotId, module_version: ver, api: a, granted };
}

export function parseManifest(v: unknown, slotId: string): Manifest | null {
  if (!isObj(v) || v.id !== slotId) return null;
  const title = str(v.title, LIMITS.title);
  const kind = oneOf(v.kind, KINDS);
  const a = api(v.api);
  if (!title || !kind || !a) return null;
  const seen = new Set<string>();
  const widgets: WidgetSpec[] = [];
  for (const w of arr(v.widgets).slice(0, LIMITS.widgets)) {
    if (!isObj(w)) continue;
    const id = localId(w.id);
    const k = oneOf(w.kind, WIDGET_KINDS);
    const t = str(w.title, LIMITS.title);
    if (!id || !k || t === null || seen.has(id)) continue; // 모르는 종류·중복 id 는 그리지 않는다
    seen.add(id);
    const r = num(w.refresh_sec) ?? 60;
    widgets.push({ id, kind: k, title: t, refresh_sec: Math.min(3600, Math.max(5, Math.round(r))) });
  }
  const actions: ActionSpec[] = [];
  for (const x of arr(v.actions).slice(0, LIMITS.actions)) {
    if (!isObj(x)) continue;
    const id = localId(x.id);
    const t = str(x.title, LIMITS.actionTitle);
    const risk = oneOf(x.risk, RISKS);
    if (id && t && risk) actions.push({ id, title: t, risk });
  }
  return {
    id: slotId,
    title,
    kind,
    version: str(v.version, 40) ?? "",
    api: a,
    summary: str(v.summary, LIMITS.sub) ?? "",
    capabilities: arr(v.capabilities)
      .map(localId)
      .filter((c): c is string => c !== null)
      .slice(0, 16),
    widgets,
    actions,
  };
}

export function parseHealth(v: unknown): Health | null {
  if (!isObj(v)) return null;
  const state = oneOf(v.state, STATES);
  if (!state || !tsIsSeconds(v.since)) return null;
  const checks: Health["checks"] = [];
  for (const c of arr(v.checks).slice(0, LIMITS.checks)) {
    if (!isObj(c)) continue;
    const id = localId(c.id);
    const st = oneOf(c.state, STATES);
    if (id && st) checks.push({ id, state: st });
  }
  const notes = arr(v.notes)
    .slice(0, LIMITS.notes)
    .map((n) => str(n, LIMITS.sub))
    .filter((n): n is string => n !== null);
  return { state, since: v.since, notes, checks };
}

function listItem(v: unknown): ListItem | null {
  if (!isObj(v)) return null;
  const title = str(v.title, LIMITS.title);
  if (!title) return null;
  const it: ListItem = { title };
  const sub = optStr(v.sub, LIMITS.sub);
  const state = oneOf(v.state, TONES);
  const open = openRef(v.open);
  if (sub) it.sub = sub;
  if (state) it.state = state;
  if (open) it.open = open;
  return it;
}

function timelineItem(v: unknown): TimelineItem | null {
  if (!isObj(v) || !tsIsSeconds(v.ts)) return null;
  const title = str(v.title, LIMITS.title);
  if (!title) return null;
  const state = oneOf(v.state, TONES);
  return state ? { ts: v.ts, title, state } : { ts: v.ts, title };
}

function widget(v: unknown): Widget | null {
  if (!isObj(v)) return null;
  const id = localId(v.id);
  const kind = oneOf(v.kind, WIDGET_KINDS);
  if (!id || !kind) return null;
  const state = oneOf(v.state, TONES) ?? undefined;
  const open = openRef(v.open);
  const unit = optStr(v.unit, 12);
  const extra = { ...(unit ? { unit } : {}), ...(state ? { state } : {}), ...(open ? { open } : {}) };
  switch (kind) {
    case "stat": {
      const value = num(v.value);
      if (value === null) return null;
      const trend = oneOf(v.trend, TRENDS);
      return { kind, id, value, ...(trend ? { trend } : {}), ...extra };
    }
    case "status": {
      if (!state) return null;
      const label = optStr(v.label, LIMITS.title);
      return { kind, id, state, ...(label ? { label } : {}), ...(open ? { open } : {}) };
    }
    case "list":
      return {
        kind,
        id,
        ...(state ? { state } : {}),
        items: arr(v.items)
          .slice(0, LIMITS.listItems)
          .map(listItem)
          .filter((i): i is ListItem => i !== null),
      };
    case "spark": {
      const points = arr(v.points)
        .slice(-LIMITS.sparkPoints)
        .map(num)
        .filter((n): n is number => n !== null);
      return { kind, id, points, ...extra };
    }
    case "timeline":
      return {
        kind,
        id,
        items: arr(v.items)
          .slice(0, LIMITS.listItems)
          .map(timelineItem)
          .filter((i): i is TimelineItem => i !== null),
      };
    case "quote": {
      const text = typeof v.text === "string" ? clip(v.text, LIMITS.quote) : null; // 본문은 줄바꿈을 둔다
      const ref = str(v.ref, LIMITS.title);
      const version = str(v.version, LIMITS.title);
      if (!text || !ref || version === null) return null;
      return { kind, id, text, ref, version, ...(open ? { open } : {}) };
    }
  }
}

/** rack.widgets 응답. `asked`: 섀시가 요청한 위젯 id — 요청하지 않은 칸과 중복은 버린다. */
export function parseWidgets(v: unknown, asked: readonly string[]): Widget[] {
  if (!isObj(v)) return [];
  const out: Widget[] = [];
  const seen = new Set<string>();
  for (const raw of arr(v.widgets).slice(0, LIMITS.widgets)) {
    const w = widget(raw);
    if (!w || !asked.includes(w.id) || seen.has(w.id)) continue;
    seen.add(w.id);
    out.push(w);
  }
  return out;
}

/** 알림 rack.event 의 params. `slotId`: 이 알림이 들어온 연결의 슬롯 — 모듈은 자기 이름으로만 사건을 낸다. */
export function parseEvent(v: unknown, slotId: string): RackEvent | null {
  if (!isObj(v) || !validModuleId(slotId) || v.module !== slotId) return null;
  const id = typeof v.id === "string" ? v.id : "";
  const prefix = `${slotId}:evt:`;
  if (!id.startsWith(prefix) || !ID_RE.test(id.slice(prefix.length))) return null;
  if (!tsIsSeconds(v.ts)) return null;
  const severity = oneOf(v.severity, SEVERITIES);
  const kind = localId(v.kind);
  const title = str(v.title, LIMITS.title);
  if (!severity || !kind || !title) return null;
  const open = openRef(v.open);
  return { id, ts: v.ts, module: slotId, severity, kind, title, ...(open ? { open } : {}) };
}
