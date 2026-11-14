// 섀시가 목업 피드에서 규격대로 도는지: 발견 → 연결 → 구독, 거부, 끊김 뒤 재동기화, 사건 접점.
import { beforeEach, describe, expect, it } from "vitest";
import { Chassis, MAX_EVENTS } from "../src/core/chassis";
import { MockFeed } from "../src/feed/mock";
import { wordLink } from "../src/word/word";

let t = 1_794_355_000_000;
const now = (): number => t;
const ev = (id: string, n: number | string, ts = Math.floor(t / 1000), more: object = {}): object => ({
  id: `${id}:evt:${n}`,
  ts,
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
  ch = new Chassis(feed, { now, builtins: { word: wordLink("ko", () => new Date(2026, 10, 14)) } });
  await ch.sync();
});

describe("발견과 연결", () => {
  it("등록부의 슬롯마다 hello → describe → health·widgets 를 받아 켜짐으로 둔다", () => {
    expect(ch.slots.map((s) => [s.id, s.status])).toEqual([
      ["storyfirewall", "on"],
      ["netx", "on"],
      ["word", "on"],
    ]);
    const sf = ch.slot("storyfirewall")!;
    expect(sf.manifest?.title).toBe("스토리방화벽");
    expect(sf.health?.state).toBe("ok");
    expect(sf.widgets.map((w) => w.kind)).toEqual(["stat", "stat", "status", "spark", "list"]);
  });

  it("내장 말씀 슬롯은 파이프 없이 답하고 그날의 구절을 낸다", () => {
    const w = ch.slot("word")!;
    expect(w.builtin).toBe(true);
    expect(w.widgets[0]).toMatchObject({ kind: "quote", open: { view: "word", ref: "jhn.10.9-11" } });
  });

  it("실행 중이 아니면 꺼짐, 다시 켜지면 붙는다", async () => {
    feed.setRunning("netx", false);
    await ch.sync();
    expect(ch.slot("netx")).toMatchObject({ status: "off", widgets: [] });
    feed.setRunning("netx", true);
    await ch.sync();
    expect(ch.slot("netx")?.status).toBe("on");
  });

  it("id 형식 위반·중복·내장 슬롯과 같은 이름의 등록은 받지 않는다", async () => {
    feed.discover = async () => [
      { id: "netx", running: true },
      { id: "netx", running: true },
      { id: "..\\evil", running: true },
      { id: "word", running: true },
    ];
    await ch.sync();
    expect(ch.slots.map((s) => s.id)).toEqual(["netx", "word"]);
    expect(ch.slot("word")?.builtin).toBe(true);
  });
});

describe("거부", () => {
  it("서버 확인 실패(connect 가 던짐)면 자료를 받지 않는다", async () => {
    feed.setRunning("netx", false);
    await ch.sync();
    feed.modules.get("netx")!.running = true;
    feed.connect = async () => {
      throw new Error("서버 Exe 경로 불일치");
    };
    await ch.sync();
    expect(ch.slot("netx")).toMatchObject({ status: "rejected", widgets: [] });
    expect(ch.slot("netx")?.manifest).toBeUndefined();
  });

  it("다른 이름으로 답하는 모듈(슬롯 가로채기)은 거부하고 30초 뒤에야 다시 본다", async () => {
    feed.setRunning("netx", false);
    await ch.sync();
    feed.modules.get("netx")!.running = true;
    feed.modules.get("netx")!.manifest.id = "storyfirewall";
    await ch.sync();
    expect(ch.slot("netx")?.status).toBe("rejected");
    feed.modules.get("netx")!.manifest.id = "netx";
    await ch.sync();
    expect(ch.slot("netx")?.status).toBe("rejected"); // 아직 재시도 전
    t += 31_000;
    await ch.sync();
    expect(ch.slot("netx")?.status).toBe("on");
  });
});

describe("사건(rack.event)", () => {
  it("여러 모듈의 사건을 시각 내림차순 한 목록으로 모은다", () => {
    const s = Math.floor(t / 1000);
    feed.emit("storyfirewall", ev("storyfirewall", 1, s - 10));
    feed.emit("netx", ev("netx", 2, s));
    expect(ch.events.map((e) => e.id)).toEqual(["netx:evt:2", "storyfirewall:evt:1"]);
  });

  it("같은 id 는 한 번만, 틀린 사건은 버리고 센다", () => {
    feed.emit("netx", ev("netx", 1));
    feed.emit("netx", ev("netx", 1));
    feed.emit("netx", ev("storyfirewall", 9)); // 남의 이름으로 냄
    feed.emit("netx", ev("netx", 2, t)); // 밀리초 시각
    feed.emit("netx", "<script>");
    expect(ch.events).toHaveLength(1);
    expect(ch.slot("netx")?.dropped).toBe(3);
  });

  it("모듈당 초당 10건을 넘으면 버린다", () => {
    for (let i = 0; i < 25; i++) feed.emit("netx", ev("netx", i));
    expect(ch.events).toHaveLength(10);
    expect(ch.slot("netx")?.dropped).toBe(15);
    t += 1000;
    feed.emit("netx", ev("netx", 100));
    expect(ch.events).toHaveLength(11);
  });

  it("보관은 최근 200건까지", () => {
    for (let i = 0; i < MAX_EVENTS + 50; i++) {
      if (i % 10 === 0) t += 1000;
      feed.emit("netx", ev("netx", i, Math.floor(t / 1000)));
    }
    expect(ch.events).toHaveLength(MAX_EVENTS);
    const ids = ch.events.map((e) => e.id);
    expect(ids).toContain(`netx:evt:${MAX_EVENTS + 49}`); // 가장 새것은 남고
    expect(ids).not.toContain("netx:evt:0"); // 가장 옛것이 밀려난다
  });
});

describe("끊김과 재동기화", () => {
  it("끊기면 '연결 끊김'으로 두고 자료를 비운다(옛 수치를 지금 값처럼 보이지 않게)", () => {
    feed.kill("storyfirewall");
    expect(ch.slot("storyfirewall")).toMatchObject({ status: "lost", widgets: [], health: undefined });
    expect(ch.slot("storyfirewall")?.manifest?.title).toBe("스토리방화벽"); // 이름은 남긴다
  });

  it("다시 붙으면 처음부터 다 받고, 다시 온 같은 사건은 겹치지 않는다", async () => {
    feed.emit("storyfirewall", ev("storyfirewall", 7));
    feed.kill("storyfirewall");
    await ch.sync();
    expect(ch.slot("storyfirewall")?.status).toBe("on");
    expect(ch.slot("storyfirewall")?.widgets).toHaveLength(5);
    feed.emit("storyfirewall", ev("storyfirewall", 7));
    feed.emit("storyfirewall", ev("storyfirewall", 8));
    expect(ch.events.map((e) => e.id).sort()).toEqual(["storyfirewall:evt:7", "storyfirewall:evt:8"]);
  });

  it("끊긴 연결에 남아 있던 구독으로는 사건이 들어오지 않는다", async () => {
    const stale = [...feed.modules.get("netx")!.listeners][0]!;
    feed.kill("netx");
    await ch.sync();
    stale.onEvent(ev("netx", 55));
    stale.onClose();
    expect(ch.events).toHaveLength(0);
    expect(ch.slot("netx")?.status).toBe("on");
  });

  it("새로 고침 주기가 되면 위젯 값을 다시 받는다", async () => {
    const before = ch.slot("netx")!.widgets;
    feed.tick();
    await ch.sync();
    expect(ch.slot("netx")!.widgets).toBe(before); // 주기 전
    t += 10_000;
    await ch.sync();
    expect(ch.slot("netx")!.widgets).not.toBe(before);
  });

  it("등록이 사라진 슬롯은 목록에서 빠진다", async () => {
    feed.modules.delete("netx");
    await ch.sync();
    expect(ch.slots.map((s) => s.id)).toEqual(["storyfirewall", "word"]);
  });
});
