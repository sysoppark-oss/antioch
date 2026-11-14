// 로컬 목업 rack 피드. 실제 모듈 없이 화면이 규격대로 도는지 보는 용도다. 여기 수치는 모두 예시다.
// 응답 모양은 설계 문서 6.3~6.6 의 예시를 따른다. 3단계에서 epistle 실클라이언트로 바뀐다.

import type { ModuleLink, RackFeed, SlotInfo } from "./feed";

interface MockModule {
  running: boolean;
  manifest: Record<string, unknown>;
  health: () => unknown;
  widgets: () => unknown[];
  listeners: Set<{ onEvent: (p: unknown) => void; onClose: () => void }>;
}

export class MockFeed implements RackFeed {
  readonly modules = new Map<string, MockModule>();
  /** 시험용: 슬롯별 파이프 이름 표식 */
  readonly tokens = new Map<string, string>();
  private seq = 90210;
  private blocked = 37;
  private latency = [88, 91, 95, 90, 92, 97, 93, 92];
  private traffic = [3, 5, 4, 9, 12, 7, 6, 8];

  constructor(private readonly now: () => number = Date.now) {
    const started = Math.floor(this.now() / 1000) - 6 * 86400;
    this.modules.set("storyfirewall", {
      running: true,
      listeners: new Set(),
      manifest: {
        id: "storyfirewall",
        title: "스토리방화벽",
        kind: "service",
        version: "0.1.6",
        api: [1, 0],
        summary: "이 PC의 연결을 이야기로 보여 주고 차단 규칙을 만든다",
        capabilities: ["events", "widgets"],
        namespaces: ["connections", "story", "policy"],
        widgets: [
          { id: "blocked_today", title: "오늘 차단", kind: "stat", sizes: ["1x1"], refresh_sec: 10 },
          { id: "rules", title: "적용 규칙", kind: "stat", sizes: ["1x1"], refresh_sec: 60 },
          { id: "collector", title: "수집", kind: "status", sizes: ["1x1"], refresh_sec: 10 },
          { id: "out_traffic", title: "바깥 접속 추이", kind: "spark", sizes: ["2x1"], refresh_sec: 10 },
          { id: "open_incidents", title: "열린 사건", kind: "list", sizes: ["2x2"], refresh_sec: 10 },
        ],
        intents_in: [],
        intents_out: ["diagnose.target"],
        actions: [{ id: "emergency.start", title: "긴급 차단", risk: "danger" }],
      },
      health: () => ({
        state: "ok",
        since: started,
        notes: [],
        checks: [
          { id: "collector", state: "ok" },
          { id: "wfp", state: "ok" },
        ],
      }),
      widgets: () => [
        { id: "blocked_today", kind: "stat", value: this.blocked, unit: "건", trend: "up", state: "caution", open: { view: "blocked" } },
        { id: "rules", kind: "stat", value: 128, unit: "개" },
        { id: "collector", kind: "status", state: "ok", label: "수집 중" },
        { id: "out_traffic", kind: "spark", points: [...this.traffic], unit: "건/분" },
        {
          id: "open_incidents",
          kind: "list",
          state: "danger",
          items: [
            {
              title: "처음 보는 프로그램이 해외로 접속",
              sub: "updater.exe → 203.0.113.9:443",
              state: "danger",
              open: { view: "incident", ref: "4812" },
            },
          ],
        },
      ],
    });
    this.modules.set("netx", {
      running: true,
      listeners: new Set(),
      manifest: {
        id: "netx",
        title: "NetX",
        kind: "app",
        version: "0.2.0",
        api: [1, 0],
        summary: "네트워크 도구 통합 작업대",
        capabilities: ["events", "widgets"],
        namespaces: [],
        widgets: [
          { id: "latency", title: "지연", kind: "stat", sizes: ["1x1"], refresh_sec: 10 },
          { id: "hops", title: "홉", kind: "stat", sizes: ["1x1"], refresh_sec: 30 },
          { id: "latency_trend", title: "지연 추이", kind: "spark", sizes: ["2x1"], refresh_sec: 10 },
          { id: "recent", title: "최근 진단", kind: "timeline", sizes: ["2x2"], refresh_sec: 30 },
        ],
        intents_in: ["diagnose.target"],
        intents_out: [],
        actions: [],
      },
      health: () => ({ state: "ok", since: Math.floor(this.now() / 1000) - 3600, notes: [], checks: [] }),
      widgets: () => [
        { id: "latency", kind: "stat", value: this.latency[this.latency.length - 1], unit: "ms", state: "ok" },
        { id: "hops", kind: "stat", value: 11, unit: "홉" },
        { id: "latency_trend", kind: "spark", points: [...this.latency], unit: "ms" },
        {
          id: "recent",
          kind: "timeline",
          items: [
            { ts: Math.floor(this.now() / 1000) - 60, title: "203.0.113.9 경로 11홉", state: "ok" },
            { ts: Math.floor(this.now() / 1000) - 900, title: "192.168.0.1 응답 1ms", state: "ok" },
          ],
        },
      ],
    });
  }

  async discover(): Promise<SlotInfo[]> {
    return [...this.modules].map(([id, m]) => ({ id, running: m.running, token: this.tokens.get(id) }));
  }

  async connect(id: string): Promise<ModuleLink> {
    const m = this.modules.get(id);
    if (!m || !m.running) throw new Error("module_unavailable");
    let mine: { onEvent: (p: unknown) => void; onClose: () => void } | undefined;
    return {
      hello: async () => ({ module: id, module_version: m.manifest.version, api: [1, 0], granted: ["read"] }),
      describe: async () => m.manifest,
      health: async () => m.health(),
      widgets: async (ids) => ({ widgets: m.widgets().filter((w) => ids.includes((w as { id: string }).id)) }),
      subscribe: async (onEvent, onClose) => {
        mine = { onEvent, onClose };
        m.listeners.add(mine);
      },
      close: () => {
        if (mine) m.listeners.delete(mine);
      },
    };
  }

  /** 알림 한 건을 그대로 흘려보낸다(시험에서 틀린 입력을 넣을 때도 쓴다) */
  emit(id: string, params: unknown): void {
    for (const l of [...(this.modules.get(id)?.listeners ?? [])]) l.onEvent(params);
  }

  /** 모듈이 죽은 것처럼 연결을 끊는다 */
  kill(id: string): void {
    const m = this.modules.get(id);
    if (!m) return;
    const ls = [...m.listeners];
    m.listeners.clear();
    for (const l of ls) l.onClose();
  }

  setRunning(id: string, running: boolean): void {
    const m = this.modules.get(id);
    if (!m) return;
    m.running = running;
    if (!running) this.kill(id);
  }

  /** 예시 수치를 조금씩 움직이고 사건을 낸다 */
  tick(): void {
    const ts = Math.floor(this.now() / 1000);
    const last = this.latency[this.latency.length - 1] ?? 92;
    this.latency = [...this.latency.slice(-23), Math.max(60, Math.min(140, last + Math.round(Math.random() * 10 - 5)))];
    this.traffic = [...this.traffic.slice(-23), Math.round(Math.random() * 12) + 2];
    if (Math.random() < 0.5) {
      this.blocked++;
      this.emit("storyfirewall", {
        id: `storyfirewall:evt:${++this.seq}`,
        ts,
        module: "storyfirewall",
        severity: "warn",
        kind: "incident.opened",
        title: "처음 보는 프로그램이 해외로 접속",
        entities: [{ type: "endpoint", ip: "203.0.113.9", port: 443, proto: "tcp" }],
        open: { view: "incident", ref: "4812" },
        suggest: [{ intent: "diagnose.target", to: "netx", label: "NetX로 정밀 진단" }],
      });
    } else {
      this.emit("netx", {
        id: `netx:evt:${++this.seq}`,
        ts,
        module: "netx",
        severity: "info",
        kind: "diagnose.done",
        title: `203.0.113.9 경로 11홉 ${this.latency[this.latency.length - 1]}ms`,
      });
    }
  }
}
