// 계약 고정 시험: fixtures/ 의 예시(설계 문서 6.3~6.6, 11.7)를 그대로 읽는다.
// 같은 파일을 Rust 쪽 crates/epistle/tests/contract.rs 도 읽는다.
import { describe, expect, it } from "vitest";
import { LIMITS } from "../src/epistle/types";
import {
  apiCompatible,
  clip,
  parseEvent,
  parseHealth,
  parseHelloReply,
  parseManifest,
  parseWidgets,
  tsIsSeconds,
  validModuleId,
} from "../src/epistle/validate";

import { readRoot } from "./paths";

const fx = (name: string): any => readRoot(`fixtures/${name}`);

describe("rack.hello", () => {
  const reply = fx("hello_response.json").result;
  it("예시 응답을 읽는다", () => {
    expect(parseHelloReply(reply, "storyfirewall")).toEqual({
      module: "storyfirewall",
      module_version: "0.1.6",
      api: [1, 0],
      granted: ["read", "operate"],
    });
  });
  it("등록부의 슬롯과 다른 이름으로 답하면 거부", () => {
    expect(parseHelloReply(reply, "netx")).toBeNull();
  });
  it("판 협상: 메이저가 같고 모듈 마이너가 같거나 높아야", () => {
    expect(apiCompatible([1, 0])).toBe(true);
    expect(apiCompatible([1, 3])).toBe(true);
    expect(apiCompatible([2, 0])).toBe(false);
    expect(parseHelloReply({ ...reply, api: [2, 0] }, "storyfirewall")).toBeNull();
    expect(parseHelloReply({ ...reply, api: "1.0" }, "storyfirewall")).toBeNull();
  });
  it("모르는 권한 이름은 버린다(권한을 넓히지 않는다)", () => {
    expect(parseHelloReply({ ...reply, granted: ["read", "root"] }, "storyfirewall")?.granted).toEqual(["read"]);
  });
});

describe("rack.describe", () => {
  it("예시 매니페스트를 읽는다", () => {
    const m = parseManifest(fx("describe_storyfirewall.json"), "storyfirewall")!;
    expect(m.title).toBe("스토리방화벽");
    expect(m.kind).toBe("service");
    expect(m.widgets.map((w) => [w.id, w.kind, w.refresh_sec])).toEqual([
      ["blocked_today", "stat", 10],
      ["open_incidents", "list", 10],
    ]);
    expect(m.actions).toEqual([{ id: "emergency.start", title: "긴급 차단", risk: "danger" }]);
    expect(parseManifest(fx("describe_word.json"), "word")?.widgets[0]?.kind).toBe("quote");
  });
  it("id 가 슬롯과 다르면 거부, 모르는 위젯 종류·중복 id 는 버린다", () => {
    const m = fx("describe_storyfirewall.json");
    expect(parseManifest(m, "netx")).toBeNull();
    const odd = { ...m, widgets: [...m.widgets, { id: "x", title: "x", kind: "html", refresh_sec: 1 }, m.widgets[0]] };
    expect(parseManifest(odd, "storyfirewall")?.widgets).toHaveLength(2);
  });
  it("새로 고침 주기는 5초~1시간으로 가둔다", () => {
    const m = fx("describe_storyfirewall.json");
    m.widgets[0].refresh_sec = 0;
    m.widgets[1].refresh_sec = 1e9;
    expect(parseManifest(m, "storyfirewall")?.widgets.map((w) => w.refresh_sec)).toEqual([5, 3600]);
  });
});

describe("rack.health / rack.widgets", () => {
  it("예시 상태를 읽는다", () => {
    const hl = parseHealth(fx("health.json"))!;
    expect(hl.state).toBe("ok");
    expect(hl.checks).toHaveLength(2);
    expect(parseHealth({ state: "great", since: 1 })).toBeNull();
    expect(parseHealth({ state: "ok", since: 1794355200000 })).toBeNull(); // 밀리초
  });
  it("예시 위젯 자료를 읽는다", () => {
    const ws = parseWidgets(fx("widgets.json"), ["blocked_today", "open_incidents"]);
    expect(ws[0]).toEqual({
      kind: "stat",
      id: "blocked_today",
      value: 37,
      trend: "up",
      unit: "건",
      state: "caution",
      open: { view: "blocked" },
    });
    expect(ws[1]).toMatchObject({ kind: "list", state: "danger" });
    expect((ws[1] as any).items[0].open).toEqual({ view: "incident", ref: "4812" });
    expect(parseWidgets(fx("widgets_word.json"), ["today"])[0]).toMatchObject({ kind: "quote", ref: "요한복음 3:16" });
    expect(parseWidgets(fx("widgets_proposed.json"), ["service", "uplink", "recent"]).map((w) => w.kind)).toEqual([
      "status",
      "spark",
      "timeline",
    ]);
  });
  it("요청하지 않은 칸, 모르는 종류, 틀린 값은 그리지 않는다", () => {
    const raw = {
      widgets: [
        { id: "a", kind: "stat", value: "37" },
        { id: "b", kind: "html", html: "<script>alert(1)</script>" },
        { id: "c", kind: "stat", value: 1 },
        { id: "ok", kind: "stat", value: 1 },
        { id: "ok", kind: "stat", value: 2 },
      ],
    };
    expect(parseWidgets(raw, ["a", "b", "ok"])).toEqual([{ kind: "stat", id: "ok", value: 1 }]);
    expect(parseWidgets("x", ["a"])).toEqual([]);
  });
  it("길이 상한: 제목 80자, 보조 200자, 목록 50건", () => {
    const long = "가".repeat(500);
    const items = Array.from({ length: 300 }, () => ({ title: long, sub: long }));
    const w = parseWidgets({ widgets: [{ id: "l", kind: "list", items }] }, ["l"])[0] as any;
    expect(w.items).toHaveLength(LIMITS.listItems);
    expect(Array.from(w.items[0].title)).toHaveLength(80);
    expect(Array.from(w.items[0].sub)).toHaveLength(200);
  });
});

describe("rack.event", () => {
  const ev = fx("event_notification.json").params;
  it("예시 알림을 읽는다", () => {
    expect(parseEvent(ev, "storyfirewall")).toEqual({
      id: "storyfirewall:evt:90211",
      ts: 1794355321,
      module: "storyfirewall",
      severity: "warn",
      kind: "incident.opened",
      title: "처음 보는 프로그램이 해외로 접속",
      open: { view: "incident", ref: "4812" },
    });
  });
  it("접점: 남의 이름, id 접두사 위조, 밀리초 시각, 모르는 심각도는 버린다", () => {
    expect(parseEvent(ev, "netx")).toBeNull();
    expect(parseEvent({ ...ev, id: "netx:evt:1" }, "storyfirewall")).toBeNull();
    expect(parseEvent({ ...ev, id: "storyfirewall:evt:<b>" }, "storyfirewall")).toBeNull();
    expect(parseEvent({ ...ev, ts: 1794355321000 }, "storyfirewall")).toBeNull();
    expect(parseEvent({ ...ev, ts: "1794355321" }, "storyfirewall")).toBeNull();
    expect(parseEvent({ ...ev, severity: "fatal" }, "storyfirewall")).toBeNull();
    expect(parseEvent(null, "storyfirewall")).toBeNull();
  });
});

describe("기본 검사", () => {
  it("모듈 id", () => {
    for (const ok of ["storyfirewall", "net_x2"]) expect(validModuleId(ok)).toBe(true);
    for (const bad of ["", "NetX", "a-b", "..\\x", "a:b", "a".repeat(33), 3]) expect(validModuleId(bad)).toBe(false);
  });
  it("시각은 epoch 초", () => {
    expect(tsIsSeconds(1794355321)).toBe(true);
    for (const bad of [0, -1, 1.5, 1794355321000, NaN, "1"]) expect(tsIsSeconds(bad)).toBe(false);
  });
  it("자르기는 글자 단위이고 제어·방향 전환 문자를 뺀다", () => {
    expect(clip("가나다라", 2)).toBe("가나");
    expect(clip("a\u202eb\u0000c", 10)).toBe("abc");
    expect(clip("😀😀😀", 2)).toBe("😀😀");
  });
});
