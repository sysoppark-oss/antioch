// 섀시의 상태 관리: 슬롯을 찾고(발견), 붙고(hello → describe → health·widgets → subscribe), 사건을 모은다.
// 화면(DOM)을 모른다 — 시험은 이 파일만으로 돈다. 자기 기능은 없다: 받아서 거르고 보관할 뿐이다.

import type { ModuleLink, RackFeed } from "../feed/feed";
import { LIMITS, type Health, type Manifest, type RackEvent, type Widget } from "../epistle/types";
import {
  parseEvent,
  parseHealth,
  parseHelloReply,
  parseManifest,
  parseWidgets,
  validModuleId,
} from "../epistle/validate";

export type SlotStatus =
  | "off" // 등록돼 있으나 실행 중이 아님
  | "connecting"
  | "on"
  | "rejected" // 서버 확인·판 협상·이름 대조에 실패 — 자료를 받지 않는다
  | "lost"; // 연결이 끊김. 다시 붙을 때 처음부터 다시 맞춘다

export interface SlotView {
  id: string;
  builtin: boolean;
  status: SlotStatus;
  /** rejected·lost 일 때 사람에게 보일 까닭 */
  reason?: string;
  manifest?: Manifest;
  health?: Health;
  widgets: Widget[];
  /** 버린 사건 수(형식 위반·초당 상한 초과) */
  dropped: number;
}

/** 섀시가 보관하는 사건. 순서와 보관은 모듈이 적은 시각이 아니라 **받은 순서**로 정한다 */
export interface FeedEvent extends RackEvent {
  /** 섀시가 받은 순번(클수록 나중) */
  recv: number;
  /** 모듈이 적어 보낸 시각. 같은 사건이 다시 온 것인지 가릴 때 쓴다 */
  rawTs: number;
  /** 모듈이 적은 시각이 허용 창 밖이라 받은 시각으로 고쳐 적었다 */
  tsFixed?: true;
}

export const MAX_EVENTS = 200;
/** 한 모듈이 통합 보관함을 다 차지하지 못하게: 넘칠 때는 가장 많이 가진 모듈의 가장 옛것부터 버린다 */
const REJECT_RETRY_MS = 30_000;
const MIN_ATTEMPT_GAP_MS = 2_000;
const MIN_REFRESH_MS = 5_000;
const MANY_WIDGETS = 10;
const MANY_WIDGETS_REFRESH_MS = 15_000;
const STABLE_MS = 60_000;
const MAX_BACKOFF_MS = 60_000;
/** 6.1 의 초당 10건 가운데 info·notice 가 쓸 수 있는 몫. 나머지는 warn·critical 을 위해 남긴다 */
const LOW_PER_SEC = 5;
const TS_FUTURE_SEC = 60;
const TS_PAST_SEC = 30 * 86_400;

interface Live {
  link: ModuleLink;
  /** 연결 세대. 옛 연결에서 늦게 온 콜백을 버리는 데 쓴다 */
  gen: number;
  nextRefresh: number;
  refreshMs: number;
  onAt: number;
  winSec: number;
  low: number;
  high: number;
}

export interface ChassisOptions {
  /** 지금 시각(밀리초). 시험에서 바꿔 끼운다 */
  now?: () => number;
  /** 섀시 안에서 바로 답하는 내장 슬롯(11.1). 파이프를 거치지 않는다 */
  builtins?: Record<string, ModuleLink>;
  /** 응답 시한(밀리초). 넘으면 끊긴 것으로 본다 */
  timeouts?: { hello: number; data: number };
}

class Timeout extends Error {}

export class Chassis {
  slots: SlotView[] = [];
  events: FeedEvent[] = [];

  private readonly now: () => number;
  private readonly builtins: Record<string, ModuleLink>;
  private readonly timeouts: { hello: number; data: number };
  private live = new Map<string, Live>();
  private retryAt = new Map<string, number>();
  private lastAttempt = new Map<string, number>();
  private lastToken = new Map<string, string | undefined>();
  private fails = new Map<string, number>();
  private gen = 0;
  private recv = 0;
  private syncing = false;
  private listeners = new Set<() => void>();

  constructor(
    private readonly feed: RackFeed,
    opts: ChassisOptions = {},
  ) {
    this.now = opts.now ?? Date.now;
    this.builtins = opts.builtins ?? {};
    this.timeouts = opts.timeouts ?? { hello: 3000, data: 5000 };
  }

  onChange(cb: () => void): () => void {
    this.listeners.add(cb);
    return () => this.listeners.delete(cb);
  }
  private changed(): void {
    for (const cb of this.listeners) cb();
  }

  slot(id: string): SlotView | undefined {
    return this.slots.find((s) => s.id === id);
  }

  /** 응답이 시한 안에 오지 않으면 던진다 — 답하지 않는 모듈 때문에 패널이 "불러오는 중"에 갇히지 않게 */
  private call<T>(p: Promise<T>, ms: number): Promise<T> {
    return new Promise<T>((resolve, reject) => {
      const timer = setTimeout(() => reject(new Timeout()), ms);
      p.then(
        (v) => (clearTimeout(timer), resolve(v)),
        (e) => (clearTimeout(timer), reject(e)),
      );
    });
  }

  /** 한 바퀴: 등록부 다시 읽기 → 새로 붙기·끊긴 것 다시 맞추기 → 때가 된 슬롯 새로 고침 */
  async sync(): Promise<void> {
    if (this.syncing) return;
    this.syncing = true;
    try {
      let found: { id: string; running: boolean; token?: string }[] = [];
      try {
        found = await this.call(this.feed.discover(), this.timeouts.data);
      } catch {
        found = []; // 등록부를 못 읽으면 "모듈 없음"으로 본다
      }
      const wanted: { id: string; running: boolean; builtin: boolean; token?: string }[] = [];
      const seen = new Set<string>();
      for (const s of found) {
        // id 형식 위반, 중복, 내장 슬롯과 같은 이름은 받지 않는다
        if (!validModuleId(s.id) || seen.has(s.id) || s.id in this.builtins) continue;
        seen.add(s.id);
        wanted.push({ id: s.id, running: s.running === true, builtin: false, token: typeof s.token === "string" ? s.token : undefined });
      }
      for (const id of Object.keys(this.builtins)) wanted.push({ id, running: true, builtin: true });

      // 등록이 사라진 슬롯 정리
      for (const old of this.slots) {
        if (!wanted.some((w) => w.id === old.id)) {
          this.drop(old.id);
          for (const m of [this.retryAt, this.lastAttempt, this.lastToken, this.fails]) m.delete(old.id);
        }
      }
      this.slots = wanted.map(
        (w) => this.slot(w.id) ?? { id: w.id, builtin: w.builtin, status: "off", widgets: [], dropped: 0 },
      );

      for (const w of wanted) {
        const view = this.slot(w.id)!;
        if (!w.running) {
          this.drop(w.id);
          this.retryAt.delete(w.id);
          this.fails.delete(w.id);
          this.clear(view, "off");
          continue;
        }
        // 공개된 파이프 이름이 바뀌었으면(진짜 모듈이 다시 올렸을 수 있다) 대기를 풀고 바로 다시 본다.
        // 단, 값을 계속 바꿔 대는 프로그램에 끌려다니지 않게 시도 사이는 2초 이상 띄운다
        if (this.lastToken.get(w.id) !== w.token) {
          this.lastToken.set(w.id, w.token);
          if (this.retryAt.has(w.id)) {
            this.retryAt.set(w.id, (this.lastAttempt.get(w.id) ?? 0) + MIN_ATTEMPT_GAP_MS);
          }
        }
        const lv = this.live.get(w.id);
        if (!lv) {
          if ((this.retryAt.get(w.id) ?? 0) > this.now()) continue;
          await this.attach(view);
        } else if (this.now() >= lv.nextRefresh) {
          await this.refresh(view, lv);
        }
      }
    } finally {
      this.syncing = false;
      this.changed();
    }
  }

  private clear(view: SlotView, status: SlotStatus, reason?: string): void {
    view.status = status;
    view.reason = reason;
    view.health = undefined;
    view.widgets = [];
    if (status !== "lost") view.manifest = undefined;
  }

  private drop(id: string): void {
    const lv = this.live.get(id);
    if (lv) {
      this.live.delete(id);
      try {
        lv.link.close();
      } catch {
        /* 닫다가 난 오류는 볼 것이 없다 */
      }
    }
  }

  private reject(view: SlotView, reason: string): void {
    this.drop(view.id);
    this.clear(view, "rejected", reason);
    this.retryAt.set(view.id, this.now() + REJECT_RETRY_MS);
  }

  /** 처음 붙거나 끊긴 뒤 다시 붙는다. 언제나 처음부터 전부 다시 받는다(재동기화). */
  private async attach(view: SlotView): Promise<void> {
    view.status = "connecting";
    this.lastAttempt.set(view.id, this.now());
    this.changed();
    let link: ModuleLink;
    try {
      link = view.builtin ? this.builtins[view.id]! : await this.call(this.feed.connect(view.id), this.timeouts.hello);
    } catch {
      return this.reject(view, "서버 확인에 실패했습니다(비정상 파이프일 수 있습니다)");
    }
    const gen = ++this.gen;
    const lv: Live = { link, gen, nextRefresh: 0, refreshMs: 60_000, onAt: 0, winSec: 0, low: 0, high: 0 };
    this.live.set(view.id, lv);
    try {
      if (!parseHelloReply(await this.call(link.hello(), this.timeouts.hello), view.id)) {
        return this.reject(view, "판이 맞지 않거나 이름이 등록부와 다릅니다");
      }
      const manifest = parseManifest(await this.call(link.describe(), this.timeouts.hello), view.id);
      if (!manifest) return this.reject(view, "명판(rack.describe)을 읽을 수 없습니다");
      view.manifest = manifest;
      view.reason = undefined;
      const least = Math.min(60, ...manifest.widgets.map((w) => w.refresh_sec));
      // 위젯이 많은 모듈은 주기를 늦춘다(규칙 안에서 섀시를 바쁘게 만드는 것 방지)
      lv.refreshMs = Math.max(MIN_REFRESH_MS, least * 1000, manifest.widgets.length >= MANY_WIDGETS ? MANY_WIDGETS_REFRESH_MS : 0);
      if (!(await this.refresh(view, lv))) return;
      view.status = "on";
      lv.onAt = this.now();
      await this.call(
        link.subscribe(
          (params) => this.onEvent(view.id, gen, params),
          () => this.onClose(view.id, gen),
        ),
        this.timeouts.hello,
      );
    } catch {
      this.lose(view);
    }
  }

  private async refresh(view: SlotView, lv: Live): Promise<boolean> {
    try {
      const ids = view.manifest?.widgets.map((w) => w.id) ?? [];
      const health = parseHealth(await this.call(lv.link.health(), this.timeouts.data));
      const widgets = ids.length ? parseWidgets(await this.call(lv.link.widgets(ids), this.timeouts.data), ids) : [];
      if (this.live.get(view.id)?.gen !== lv.gen) return false; // 기다리는 사이 끊겼다
      view.health = health ?? undefined; // 형식이 틀리면 "상태 모름"
      view.widgets = widgets;
      lv.nextRefresh = this.now() + lv.refreshMs;
      return true;
    } catch (e) {
      this.lose(view, e instanceof Timeout ? "응답이 늦어 연결을 끊었습니다" : undefined);
      return false;
    }
  }

  /** 끊김 처리. 붙었다 끊기를 되풀이하면 다시 붙기를 점점 늦춘다(2초 → 4초 → … 최대 60초) */
  private lose(view: SlotView, why?: string): void {
    if (view.status === "rejected") return this.drop(view.id);
    const lv = this.live.get(view.id);
    const stable = !!lv && lv.onAt > 0 && this.now() - lv.onAt >= STABLE_MS;
    const n = stable ? 1 : (this.fails.get(view.id) ?? 0) + 1;
    this.fails.set(view.id, n);
    this.drop(view.id);
    const wait = n <= 1 ? 0 : Math.min(MAX_BACKOFF_MS, 1000 * 2 ** (n - 1));
    if (wait) this.retryAt.set(view.id, this.now() + wait);
    else this.retryAt.delete(view.id);
    this.clear(view, "lost", why ?? (n >= 3 ? "연결이 불안정합니다 — 잠시 뒤 다시 붙습니다" : undefined));
  }

  private onClose(id: string, gen: number): void {
    if (this.live.get(id)?.gen !== gen) return;
    const view = this.slot(id);
    if (view) this.lose(view);
    this.changed();
  }

  private onEvent(id: string, gen: number, params: unknown): void {
    const lv = this.live.get(id);
    const view = this.slot(id);
    if (!lv || lv.gen !== gen || !view) return; // 옛 연결에서 온 것
    const nowSec = Math.floor(this.now() / 1000);
    if (lv.winSec !== nowSec) {
      lv.winSec = nowSec;
      lv.low = lv.high = 0;
    }
    const ev = parseEvent(params, id);
    if (!ev) {
      view.dropped++;
      this.changed();
      return;
    }
    // 초당 상한: 가벼운 사건이 몫을 다 써도 warn·critical 자리는 남는다
    const low = ev.severity === "info" || ev.severity === "notice";
    if (low ? lv.low >= LOW_PER_SEC : lv.low + lv.high >= LIMITS.eventsPerSec) {
      view.dropped++;
      this.changed();
      return;
    }
    if (low) lv.low++;
    else lv.high++;

    // 같은 사건이 다시 온 것(다시 붙은 뒤의 되풀이)만 버린다.
    // id 는 같아도 내용이 다르면 새 사건이다 — 모듈이 다시 켜져 번호를 처음부터 세는 경우
    if (this.events.some((e) => e.id === ev.id && e.rawTs === ev.ts && e.title === ev.title)) return;

    const stored: FeedEvent = { ...ev, recv: ++this.recv, rawTs: ev.ts };
    if (ev.ts > nowSec + TS_FUTURE_SEC || ev.ts < nowSec - TS_PAST_SEC) {
      stored.ts = nowSec; // 먼 미래·먼 과거 시각으로 목록 자리를 차지하거나 숨지 못하게
      stored.tsFixed = true;
    }
    this.events.unshift(stored);
    while (this.events.length > MAX_EVENTS) this.evict();
    this.changed();
  }

  /** 가장 많이 가진 모듈의 가장 옛 사건 하나를 버린다 */
  private evict(): void {
    const count = new Map<string, number>();
    for (const e of this.events) count.set(e.module, (count.get(e.module) ?? 0) + 1);
    let top = "";
    for (const [m, c] of count) if (c > (count.get(top) ?? 0)) top = m;
    for (let i = this.events.length - 1; i >= 0; i--) {
      if (this.events[i]!.module === top) {
        this.events.splice(i, 1);
        return;
      }
    }
  }

  stop(): void {
    for (const id of [...this.live.keys()]) this.drop(id);
  }
}
