// 검증 답변(섀시 1차)에서 받아들인 항목을 시험으로 고정한다. 항목 번호는 docs/답변반영_섀시1차.md 와 같다.
import { beforeEach, describe, expect, it } from "vitest";
import { Chassis, MAX_EVENTS, type SlotView } from "../src/core/chassis";
import { MockFeed } from "../src/feed/mock";
import { label, parseManifest, parseWidgets } from "../src/epistle/validate";
import { displayTitles, modulePanel, overviewPanel } from "../src/ui/panels";

let t = 1_794_355_000_000;
const now = (): number => t;
const sec = (): number => Math.floor(t / 1000);
const ev = (id: string, n: number | string, more: object = {}): object => ({
  id: `${id}:evt:${n}`,
  ts: sec(),
  module: id,
  severity: "warn",
  kind: "incident.opened",
  title: `사건 ${n}`,
  ...more,
});

let feed: MockFeed;
let ch: Chassis;
beforeEach(async () => {
  t = 1_794_355_000_000;
  feed = new MockFeed(now);
  ch = new Chassis(feed, { now, timeouts: { hello: 30, data: 30 } });
  await ch.sync();
});

describe("R1·R2·Q1 글자와 수", () => {
  it("이름표: 한 줄로, 폭 없는·방향 전환·C1 문자와 그림 글자를 뺀다", () => {
    expect(label("정상\n작동", 80)).toBe("정상 작동");
    expect(label("a\u200bb\ufeffc\u061cd\u0085e\u2028f", 80)).toBe("abcdef");
    expect(label("\u{1F7E2} 정상 작동 \u2705", 80)).toBe("정상 작동");
  });
  it("겹쳐 쌓은 결합 부호는 둘까지만 남긴다", () => {
    expect(Array.from(label("Z" + "\u0301".repeat(40), 80))).toHaveLength(3);
  });
  it("너무 큰 수는 그 칸을 버리고, 추이선에서는 그 점만 뺀다", () => {
    const ws = parseWidgets(
      { widgets: [{ id: "a", kind: "stat", value: 1e308 }, { id: "b", kind: "spark", points: [1, 1e308, -1e308, 2] }] },
      ["a", "b"],
    );
    expect(ws).toEqual([{ kind: "spark", id: "b", points: [1, 2] }]);
  });
  it("값이 모두 같은 추이선도 그림 좌표가 깨지지 않는다", () => {
    const view: SlotView = {
      id: "netx",
      builtin: false,
      status: "on",
      manifest: parseManifest(
        { id: "netx", title: "NetX", kind: "app", api: [1, 0], widgets: [{ id: "s", title: "추이", kind: "spark", refresh_sec: 10 }] },
        "netx",
      )!,
      widgets: parseWidgets({ widgets: [{ id: "s", kind: "spark", points: [42, 42, 42] }] }, ["s"]),
      dropped: 0,
    };
    const root = document.createElement("div");
    root.append(...modulePanel(view, "#fff"));
    expect(root.querySelector("polyline")?.getAttribute("points")).not.toMatch(/NaN|Infinity/);
  });
});

describe("R10 조작 표시", () => {
  it("조작 이름은 24자까지, 섀시가 '읽기 전용' 표시를 붙인다", () => {
    const m = parseManifest(
      { id: "netx", title: "NetX", kind: "app", api: [1, 0], actions: [{ id: "a", title: "지금 즉시 클릭하여 복구 (관리자 승인됨) 반드시 누르세요", risk: "safe" }] },
      "netx",
    )!;
    expect(Array.from(m.actions[0]!.title).length).toBeLessThanOrEqual(24);
    const root = document.createElement("div");
    root.append(...modulePanel({ id: "netx", builtin: false, status: "on", manifest: m, widgets: [], dropped: 0 }, "#fff"));
    expect(root.querySelector(".chip.ro")?.textContent).toContain("읽기 전용");
  });
});

describe("R5·R6·Q3 사건의 시각과 보관", () => {
  it("먼 미래·먼 과거 시각은 받은 시각으로 고쳐 적고 표시한다", () => {
    feed.emit("netx", ev("netx", 1, { ts: 4_102_444_799 }));
    feed.emit("netx", ev("netx", 2, { ts: 1 }));
    feed.emit("netx", ev("netx", 3, { ts: sec() + 30 }));
    expect(ch.events.map((e) => [e.id, e.ts, e.tsFixed ?? false])).toEqual([
      ["netx:evt:3", sec() + 30, false],
      ["netx:evt:2", sec(), true],
      ["netx:evt:1", sec(), true],
    ]);
  });
  it("순서는 받은 순서다 — 미래 시각을 단 사건이 맨 위에 눌러앉지 못한다", () => {
    feed.emit("netx", ev("netx", "future", { ts: 4_102_444_799 }));
    t += 1000;
    feed.emit("storyfirewall", ev("storyfirewall", "real"));
    expect(ch.events[0]?.id).toBe("storyfirewall:evt:real");
  });
  it("한 모듈이 쏟아 내도 다른 모듈의 사건은 밀려나지 않는다", () => {
    feed.emit("storyfirewall", ev("storyfirewall", "keep"));
    for (let i = 0; i < MAX_EVENTS + 100; i++) {
      if (i % 10 === 0) t += 1000;
      feed.emit("netx", ev("netx", i));
    }
    expect(ch.events).toHaveLength(MAX_EVENTS);
    expect(ch.events.some((e) => e.id === "storyfirewall:evt:keep")).toBe(true);
  });
});

describe("R7 초당 상한", () => {
  it("가벼운 사건이 몰려도 warn·critical 자리는 남는다", () => {
    for (let i = 0; i < 10; i++) feed.emit("netx", ev("netx", `i${i}`, { severity: "info" }));
    feed.emit("netx", ev("netx", "c", { severity: "critical" }));
    expect(ch.events.filter((e) => e.severity === "info")).toHaveLength(5);
    expect(ch.events[0]).toMatchObject({ id: "netx:evt:c", severity: "critical" });
    expect(ch.slot("netx")?.dropped).toBe(5);
  });
});

describe("Q4 끊김·다시 붙기", () => {
  it("모듈이 다시 켜져 번호를 처음부터 세도 새 사건을 버리지 않는다", async () => {
    feed.emit("netx", ev("netx", 1, { title: "어제 일" }));
    feed.kill("netx");
    await ch.sync();
    t += 5000;
    feed.emit("netx", ev("netx", 1, { title: "오늘 일" }));
    feed.emit("netx", ev("netx", 1, { title: "오늘 일" })); // 글자 그대로 다시 온 것은 한 번만
    expect(ch.events.map((e) => e.title)).toEqual(["오늘 일", "어제 일"]);
  });
  it("답하지 않는 모듈은 시한 뒤 끊고 까닭을 보인다", async () => {
    feed.kill("netx");
    feed.modules.get("netx")!.health = () => new Promise(() => {});
    await ch.sync();
    expect(ch.slot("netx")).toMatchObject({ status: "lost", reason: "응답이 늦어 연결을 끊었습니다" });
  });
  it("붙었다 끊기를 되풀이하면 다시 붙기를 2초·4초로 늦추고 '불안정'을 보인다", async () => {
    feed.kill("netx"); // 1번째: 바로 다시 붙는다
    await ch.sync();
    expect(ch.slot("netx")?.status).toBe("on");
    feed.kill("netx"); // 2번째: 2초 뒤
    await ch.sync();
    expect(ch.slot("netx")?.status).toBe("lost");
    t += 2000;
    await ch.sync();
    expect(ch.slot("netx")?.status).toBe("on");
    feed.kill("netx"); // 3번째: 4초 뒤
    expect(ch.slot("netx")?.reason).toContain("불안정");
    t += 2000;
    await ch.sync();
    expect(ch.slot("netx")?.status).toBe("lost");
    t += 2000;
    await ch.sync();
    expect(ch.slot("netx")?.status).toBe("on");
  });
  it("1분 넘게 잘 붙어 있었으면 끊겨도 바로 다시 붙는다", async () => {
    feed.kill("netx");
    await ch.sync();
    feed.kill("netx");
    t += 2000;
    await ch.sync();
    t += 61_000;
    feed.kill("netx");
    await ch.sync();
    expect(ch.slot("netx")?.status).toBe("on");
  });
});

describe("R3·Q5 확인 실패 뒤 되살아나기", () => {
  it("파이프 이름이 바뀌면 30초를 기다리지 않고 2초 뒤 다시 본다", async () => {
    feed.setRunning("netx", false);
    await ch.sync();
    feed.modules.get("netx")!.running = true;
    feed.tokens.set("netx", "fake");
    const real = feed.connect.bind(feed);
    feed.connect = async () => {
      throw new Error("서버 Exe 경로 불일치");
    };
    await ch.sync();
    expect(ch.slot("netx")?.status).toBe("rejected");
    feed.connect = real;
    t += 5000;
    await ch.sync();
    expect(ch.slot("netx")?.status).toBe("rejected"); // 이름이 그대로면 30초 대기
    feed.tokens.set("netx", "real");
    await ch.sync();
    expect(ch.slot("netx")?.status).toBe("on");
  });
  it("이름을 계속 바꿔 대도 시도는 2초에 한 번을 넘지 않는다", async () => {
    feed.setRunning("netx", false);
    await ch.sync();
    feed.modules.get("netx")!.running = true;
    let tries = 0;
    feed.connect = async () => {
      tries++;
      throw new Error("가짜");
    };
    for (let i = 0; i < 20; i++) {
      feed.tokens.set("netx", `fake${i}`);
      await ch.sync();
      t += 500;
    }
    expect(tries).toBeLessThanOrEqual(6);
  });
});

describe("Q2 표시 이름", () => {
  it("같은 이름을 쓰는 뒤쪽 모듈에는 슬롯 id 를 붙이고, 통합 피드의 출처는 따로 둔 칸에 보인다", async () => {
    feed.modules.get("netx")!.manifest.title = "스토리방화벽";
    feed.kill("netx");
    await ch.sync();
    expect([...displayTitles(ch.slots).values()]).toEqual(["스토리방화벽", "스토리방화벽 (netx)"]);
    feed.emit("netx", ev("netx", 1, { title: "스토리방화벽 · 차단 해제됨" }));
    const root = document.createElement("div");
    root.append(...overviewPanel(ch.slots, ch.events));
    expect(root.querySelector(".frow .src")?.textContent).toBe("스토리방화벽 (netx)");
    expect(root.querySelector(".frow .m")?.textContent).toBe("스토리방화벽 · 차단 해제됨");
  });
});

describe("변경 제안 2판 1장: 위젯 색조 칸은 tone", () => {
  it("tone 만 읽는다 — health 값(down)이나 옛 칸 이름(state)은 색조 없음으로 두고 ok 로 바꾸지 않는다", () => {
    const ws = parseWidgets(
      {
        widgets: [
          { id: "a", kind: "stat", value: 1, tone: "danger" },
          { id: "b", kind: "stat", value: 1, tone: "down" },
          { id: "c", kind: "stat", value: 1, state: "danger" },
        ],
      },
      ["a", "b", "c"],
    );
    expect(ws).toEqual([
      { kind: "stat", id: "a", value: 1, tone: "danger" },
      { kind: "stat", id: "b", value: 1 },
      { kind: "stat", id: "c", value: 1 },
    ]);
  });
});
