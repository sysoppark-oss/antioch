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
  | "lost"; // 연결이 끊김. 다음 발견 때 처음부터 다시 맞춘다

export interface SlotView {
  id: string;
  builtin: boolean;
  status: SlotStatus;
  /** rejected 일 때 사람에게 보일 까닭 */
  reason?: string;
  manifest?: Manifest;
  health?: Health;
  widgets: Widget[];
  /** 버린 사건 수(형식 위반·초당 상한 초과) */
  dropped: number;
}

export const MAX_EVENTS = 200;
const RETRY_MS = 30_000;
const MIN_REFRESH_MS = 5_000;

interface Live {
  link: ModuleLink;
  /** 연결 세대. 옛 연결에서 늦게 온 콜백을 버리는 데 쓴다 */
  gen: number;
  nextRefresh: number;
  refreshMs: number;
  winSec: number;
  winCount: number;
}

export interface ChassisOptions {
  /** 지금 시각(밀리초). 시험에서 바꿔 끼운다 */
  now?: () => number;
  /** 섀시 안에서 바로 답하는 내장 슬롯(11.1). 파이프를 거치지 않는다 */
  builtins?: Record<string, ModuleLink>;
}

export class Chassis {
  slots: SlotView[] = [];
  events: RackEvent[] = [];

  private readonly now: () => number;
  private readonly builtins: Record<string, ModuleLink>;
  private live = new Map<string, Live>();
  private retryAt = new Map<string, number>();
  private gen = 0;
  private syncing = false;
  private listeners = new Set<() => void>();

  constructor(
    private readonly feed: RackFeed,
    opts: ChassisOptions = {},
  ) {
    this.now = opts.now ?? Date.now;
    this.builtins = opts.builtins ?? {};
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

  /** 한 바퀴: 등록부 다시 읽기 → 새로 붙기·끊긴 것 다시 맞추기 → 때가 된 슬롯 새로 고침 */
  async sync(): Promise<void> {
    if (this.syncing) return;
    this.syncing = true;
    try {
      let found: { id: string; running: boolean }[] = [];
      try {
        found = await this.feed.discover();
      } catch {
        found = []; // 등록부를 못 읽으면 "모듈 없음"으로 본다
      }
      const wanted: { id: string; running: boolean; builtin: boolean }[] = [];
      const seen = new Set<string>();
      for (const s of found) {
        // id 형식 위반, 중복, 내장 슬롯과 같은 이름은 받지 않는다
        if (!validModuleId(s.id) || seen.has(s.id) || s.id in this.builtins) continue;
        seen.add(s.id);
        wanted.push({ id: s.id, running: s.running === true, builtin: false });
      }
      for (const id of Object.keys(this.builtins)) wanted.push({ id, running: true, builtin: true });

      // 등록이 사라진 슬롯 정리
      for (const old of this.slots) {
        if (!wanted.some((w) => w.id === old.id)) this.drop(old.id);
      }
      this.slots = wanted.map(
        (w) => this.slot(w.id) ?? { id: w.id, builtin: w.builtin, status: "off", widgets: [], dropped: 0 },
      );

      for (const w of wanted) {
        const view = this.slot(w.id)!;
        if (!w.running) {
          this.drop(w.id);
          this.retryAt.delete(w.id);
          this.clear(view, "off");
          continue;
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
    this.retryAt.set(view.id, this.now() + RETRY_MS);
  }

  /** 처음 붙거나 끊긴 뒤 다시 붙는다. 언제나 처음부터 전부 다시 받는다(재동기화). */
  private async attach(view: SlotView): Promise<void> {
    view.status = "connecting";
    this.changed();
    let link: ModuleLink;
    try {
      link = view.builtin ? this.builtins[view.id]! : await this.feed.connect(view.id);
    } catch {
      return this.reject(view, "서버 확인에 실패했습니다");
    }
    const gen = ++this.gen;
    const lv: Live = { link, gen, nextRefresh: 0, refreshMs: 60_000, winSec: 0, winCount: 0 };
    this.live.set(view.id, lv);
    try {
      if (!parseHelloReply(await link.hello(), view.id)) {
        return this.reject(view, "판이 맞지 않거나 이름이 등록부와 다릅니다");
      }
      const manifest = parseManifest(await link.describe(), view.id);
      if (!manifest) return this.reject(view, "명판(rack.describe)을 읽을 수 없습니다");
      view.manifest = manifest;
      view.reason = undefined;
      const least = Math.min(60, ...manifest.widgets.map((w) => w.refresh_sec));
      lv.refreshMs = Math.max(MIN_REFRESH_MS, least * 1000);
      if (!(await this.refresh(view, lv))) return;
      view.status = "on";
      await link.subscribe(
        (params) => this.onEvent(view.id, gen, params),
        () => this.onClose(view.id, gen),
      );
    } catch {
      this.lose(view);
    }
  }

  private async refresh(view: SlotView, lv: Live): Promise<boolean> {
    try {
      const ids = view.manifest?.widgets.map((w) => w.id) ?? [];
      const health = parseHealth(await lv.link.health());
      const widgets = ids.length ? parseWidgets(await lv.link.widgets(ids), ids) : [];
      if (this.live.get(view.id)?.gen !== lv.gen) return false; // 기다리는 사이 끊겼다
      view.health = health ?? undefined; // 형식이 틀리면 "상태 모름"
      view.widgets = widgets;
      lv.nextRefresh = this.now() + lv.refreshMs;
      return true;
    } catch {
      this.lose(view);
      return false;
    }
  }

  private lose(view: SlotView): void {
    this.drop(view.id);
    if (view.status !== "rejected") this.clear(view, "lost");
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
    const sec = Math.floor(this.now() / 1000);
    if (lv.winSec !== sec) {
      lv.winSec = sec;
      lv.winCount = 0;
    }
    if (++lv.winCount > LIMITS.eventsPerSec) {
      view.dropped++;
      return;
    }
    const ev = parseEvent(params, id);
    if (!ev) {
      view.dropped++;
      this.changed();
      return;
    }
    if (this.events.some((e) => e.id === ev.id)) return; // 다시 붙은 뒤 같은 사건이 또 와도 한 번만
    this.events.push(ev);
    this.events.sort((a, b) => b.ts - a.ts || (a.id < b.id ? -1 : 1));
    if (this.events.length > MAX_EVENTS) this.events.length = MAX_EVENTS;
    this.changed();
  }

  stop(): void {
    for (const id of [...this.live.keys()]) this.drop(id);
  }
}
