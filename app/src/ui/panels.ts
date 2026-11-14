// 패널 그리기. 모듈에서 온 글자는 모두 textContent 로만 넣는다 — innerHTML 을 쓰지 않는다(화면 주입 방지, 6.5).

import type { SlotView } from "../core/chassis";
import type { RackEvent, Severity, State, Tone, Widget } from "../epistle/types";

const SVG = "http://www.w3.org/2000/svg";

export function h<K extends keyof HTMLElementTagNameMap>(tag: K, cls?: string, text?: string): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text !== undefined) e.textContent = text;
  return e;
}
function s(tag: string, attrs: Record<string, string | number>): SVGElement {
  const e = document.createElementNS(SVG, tag);
  for (const [k, v] of Object.entries(attrs)) e.setAttribute(k, String(v));
  return e;
}

const STATE_TONE: Record<State, Tone> = { ok: "ok", degraded: "caution", down: "danger", starting: "off" };
const STATE_LABEL: Record<State, string> = { ok: "정상", degraded: "일부 문제", down: "멈춤", starting: "시작 중" };
const SEV_TONE: Record<Severity, string> = { info: "info", notice: "ok", warn: "caution", critical: "danger" };

/** 슬롯의 상태등: 색조와 문구 */
export function slotLamp(v: SlotView): { tone: Tone; label: string } {
  switch (v.status) {
    case "off":
      return { tone: "off", label: "꺼짐" };
    case "connecting":
      return { tone: "off", label: "연결 중" };
    case "rejected":
      return { tone: "danger", label: "확인 실패" };
    case "lost":
      return { tone: "caution", label: "연결 끊김" };
    case "on":
      return v.health
        ? { tone: STATE_TONE[v.health.state], label: STATE_LABEL[v.health.state] }
        : { tone: "off", label: "상태 모름" };
  }
}

const PALETTE: Record<string, string> = { storyfirewall: "var(--sf)", netx: "var(--nx)", word: "var(--wd)" };
const SPARE = ["var(--m3)", "var(--m4)", "var(--sy)"];
export function slotColor(id: string, index: number): string {
  return PALETTE[id] ?? SPARE[index % SPARE.length]!;
}

export function hhmm(ts: number): string {
  const d = new Date(ts * 1000);
  return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
}

function fmt(n: number): string {
  return Number.isInteger(n) ? n.toLocaleString("ko-KR") : n.toFixed(1);
}

function valueNode(cls: string, value: number, unit?: string): HTMLElement {
  const b = h("b", cls, fmt(value));
  if (unit) b.append(h("small", "", unit));
  return b;
}

function caption(title: string, lamp: { tone: Tone; label: string }): HTMLElement {
  const cap = h("div", "cap");
  const st = h("span", `st ${lamp.tone}`);
  st.append(h("i"), document.createTextNode(lamp.label));
  cap.append(h("span", "tag"), h("b", "", title), st);
  return cap;
}

function row(time: string | null, tone: string | undefined, main: string, sub?: string): HTMLElement {
  const r = h("div", "frow");
  if (time) r.append(h("span", "t", time));
  r.append(h("span", `lamp ${tone ?? "info"}`), h("span", "m", main));
  if (sub) r.append(h("span", "s", sub));
  return r;
}

function box(title: string, rows: HTMLElement[], emptyText: string): HTMLElement {
  const b = h("div", "box");
  b.append(h("div", "ttl", title));
  if (rows.length) b.append(...rows);
  else b.append(h("div", "frow none", emptyText));
  return b;
}

function spark(points: number[], color: string): SVGElement {
  const W = 400,
    H = 84,
    P = 6;
  const svg = s("svg", { class: "viz", viewBox: `0 0 ${W} ${H}`, role: "img" });
  if (points.length < 2) return svg;
  const min = Math.min(...points),
    max = Math.max(...points),
    span = max - min || 1;
  const xy = points.map((p, i) => [P + (i * (W - 2 * P)) / (points.length - 1), H - P - ((p - min) / span) * (H - 2 * P)] as const);
  const line = xy.map(([x, y]) => `${x.toFixed(1)},${y.toFixed(1)}`).join(" ");
  svg.append(
    s("polygon", { points: `${P},${H - P} ${line} ${W - P},${H - P}`, fill: color, opacity: 0.12 }),
    s("polyline", { points: line, fill: "none", stroke: color, "stroke-width": 1.8, "stroke-linejoin": "round" }),
  );
  const last = xy[xy.length - 1]!;
  svg.append(s("circle", { cx: last[0], cy: last[1], r: 3.2, fill: "#e4f1ff" }));
  return svg;
}

function titleOf(v: SlotView, widgetId: string): string {
  return v.manifest?.widgets.find((w) => w.id === widgetId)?.title ?? widgetId;
}

/** 모듈 한 대의 운영 대시보드: 헤더·상태·지표·그래프·목록·조작 */
export function modulePanel(v: SlotView, color: string): HTMLElement[] {
  const title = v.manifest?.title ?? v.id;
  const out: HTMLElement[] = [caption(title, slotLamp(v))];
  const dash = h("div", "dash");
  out.push(dash);

  if (v.status !== "on") {
    const msg =
      v.status === "off"
        ? "꺼져 있습니다\n모듈을 켜면 이 자리에 운영 대시보드가 떠오릅니다"
        : v.status === "rejected"
          ? `랙 구성원 확인에 실패해 자료를 받지 않습니다\n${v.reason ?? ""}`
          : v.status === "lost"
            ? "연결이 끊겼습니다\n다시 붙으면 처음부터 새로 받습니다"
            : "연결하는 중입니다";
    const blank = h("div", "blank", msg);
    blank.style.whiteSpace = "pre-line";
    dash.append(blank);
    return out;
  }

  if (v.manifest?.summary) dash.append(h("div", "sum", v.manifest.summary));

  const stats = v.widgets.filter((w): w is Extract<Widget, { kind: "stat" }> => w.kind === "stat").slice(0, 3);
  if (stats.length) {
    const rowEl = h("div", "kpirow");
    for (const w of stats) {
      const k = h("div", `kpi ${w.state ?? ""}`);
      k.append(valueNode("", w.value, w.unit), h("span", "", titleOf(v, w.id)));
      rowEl.append(k);
    }
    dash.append(rowEl);
  }

  for (const w of v.widgets) {
    if (w.kind === "spark") {
      const b = h("div", "box");
      const last = w.points[w.points.length - 1];
      b.append(h("div", "ttl", `${titleOf(v, w.id)}${last !== undefined ? ` · ${fmt(last)}${w.unit ?? ""}` : ""}`));
      b.append(spark(w.points, color));
      dash.append(b);
    } else if (w.kind === "list") {
      dash.append(
        box(
          titleOf(v, w.id),
          w.items.slice(0, 4).map((it) => row(null, it.state, it.title, it.sub)),
          "없음",
        ),
      );
    } else if (w.kind === "timeline") {
      dash.append(
        box(
          titleOf(v, w.id),
          w.items.slice(0, 4).map((it) => row(hhmm(it.ts), it.state, it.title)),
          "없음",
        ),
      );
    }
  }

  const chips = h("div", "chips");
  for (const w of v.widgets) {
    if (w.kind !== "status") continue;
    const c = h("span", "chip");
    c.append(h("span", `lamp ${w.state}`), document.createTextNode(`${titleOf(v, w.id)}${w.label ? ` · ${w.label}` : ""}`));
    chips.append(c);
  }
  // 조작: 이 단계는 읽기 전용이다. 누를 수 없는 표시로만 둔다(확정은 언제나 주인 모듈의 확인 창, 6.8)
  for (const a of v.manifest?.actions ?? []) {
    chips.append(h("span", `chip ${a.risk === "danger" || a.risk === "forbidden" ? "danger" : a.risk === "caution" ? "caution" : ""}`, a.title));
  }
  if (chips.childElementCount) dash.append(chips);

  const notes = v.health?.notes ?? [];
  const bad = v.health?.checks.filter((c) => c.state !== "ok").map((c) => c.id) ?? [];
  const foot = [
    v.manifest ? `판 ${v.manifest.version}` : "",
    bad.length ? `점검 필요: ${bad.join(", ")}` : "",
    ...notes,
    v.dropped ? `버린 알림 ${v.dropped}건` : "",
  ].filter(Boolean);
  if (foot.length) dash.append(h("div", "foot", foot.join(" · ")));
  return out;
}

/** 시스템 개요: 모듈 상태등 + 통합 이벤트 피드 */
export function overviewPanel(slots: SlotView[], events: RackEvent[]): HTMLElement[] {
  const mods = slots.filter((v) => !v.builtin);
  const on = mods.filter((v) => v.status === "on");
  const worst: Tone = mods.some((v) => slotLamp(v).tone === "danger")
    ? "danger"
    : mods.some((v) => slotLamp(v).tone === "caution")
      ? "caution"
      : on.length
        ? "ok"
        : "off";
  const label = { ok: "정상", caution: "주의", danger: "확인 필요", off: "모듈 없음" }[worst];
  const out: HTMLElement[] = [caption("안디옥 · 통합 관제", { tone: worst, label })];
  const dash = h("div", "dash");
  out.push(dash);

  if (mods.length) {
    const tiles = h("div", "tiles");
    mods.slice(0, 4).forEach((v, i) => {
      const t = h("div", "tile");
      t.style.setProperty("--tc", slotColor(v.id, i));
      const lamp = slotLamp(v);
      const stat = v.widgets.find((w): w is Extract<Widget, { kind: "stat" }> => w.kind === "stat");
      const mv = stat ? valueNode("mv", stat.value, stat.unit) : h("b", "mv", lamp.label);
      mv.className = "mv";
      const nm = h("div", "nm");
      nm.append(h("span", `lamp ${lamp.tone}`), document.createTextNode(stat ? titleOf(v, stat.id) : (v.manifest?.title ?? v.id)));
      t.append(h("div", "ic", (v.manifest?.title ?? v.id).slice(0, 2).toUpperCase()), mv, nm);
      tiles.append(t);
    });
    dash.append(tiles);
  }

  const titles = new Map(slots.map((v) => [v.id, v.manifest?.title ?? v.id]));
  dash.append(
    box(
      "실시간 이벤트",
      events.slice(0, 6).map((e) => row(hhmm(e.ts), SEV_TONE[e.severity], `${titles.get(e.module) ?? e.module} · ${e.title}`)),
      "아직 사건이 없습니다",
    ),
  );
  const off = mods.length - on.length;
  dash.append(h("div", "foot", mods.length ? `${on.length} 모듈 가동${off ? ` · ${off} 모듈 쉬는 중` : ""}` : "등록된 모듈이 없습니다"));
  return out;
}

/** 오늘의 말씀. 본문을 자르거나 줄이지 않는다 */
export function wordPanel(v: SlotView): HTMLElement[] {
  const q = v.widgets.find((w): w is Extract<Widget, { kind: "quote" }> => w.kind === "quote");
  const out: HTMLElement[] = [caption("오늘의 말씀", { tone: "ok", label: "표시 중" })];
  if (!q) return out;
  const bq = h("blockquote", "verse");
  bq.append(h("p", "", q.text), h("cite", "", `${q.ref} · ${q.version}`));
  out.push(bq);
  return out;
}

export function emptyPanel(): HTMLElement[] {
  return [caption("빈 슬롯", { tone: "off", label: "미설치" }), h("div", "blank", "모듈을 설치하면\n이 자리에 운영 대시보드가 떠오릅니다")];
}
