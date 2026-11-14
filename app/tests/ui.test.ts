// 화면: 모듈에서 온 글자가 요소가 되지 않는지, 말씀이 규칙대로 나오는지.
import { readRoot } from "./paths";
import { describe, expect, it } from "vitest";
import type { SlotView } from "../src/core/chassis";
import { parseEvent, parseManifest, parseWidgets } from "../src/epistle/validate";
import { modulePanel, overviewPanel, wordPanel } from "../src/ui/panels";
import { dayNumber, pickLang, today } from "../src/word/word";

const EVIL = `<img src=x onerror="window.__pwn=1"><script>window.__pwn=1</script>`;

function mount(nodes: HTMLElement[]): HTMLElement {
  const root = document.createElement("div");
  root.append(...nodes);
  document.body.replaceChildren(root);
  return root;
}

describe("화면 주입 방지", () => {
  const manifest = parseManifest(
    {
      id: "netx",
      title: EVIL,
      kind: "app",
      version: EVIL,
      api: [1, 0],
      summary: EVIL,
      widgets: [
        { id: "s", title: EVIL, kind: "stat", refresh_sec: 10 },
        { id: "l", title: EVIL, kind: "list", refresh_sec: 10 },
      ],
      actions: [{ id: "a", title: EVIL, risk: "danger" }],
    },
    "netx",
  )!;
  const view: SlotView = {
    id: "netx",
    builtin: false,
    status: "on",
    manifest,
    health: { state: "ok", since: 1, notes: [EVIL], checks: [] },
    widgets: parseWidgets(
      { widgets: [{ id: "s", kind: "stat", value: 1, unit: EVIL }, { id: "l", kind: "list", items: [{ title: EVIL, sub: EVIL }] }] },
      ["s", "l"],
    ),
    dropped: 0,
  };

  it("모듈 패널: 글자로만 보이고 img·script 요소가 생기지 않는다", () => {
    const root = mount(modulePanel(view, "#fff"));
    expect(root.querySelectorAll("img,script,iframe,a,[onerror]")).toHaveLength(0);
    expect(root.textContent).toContain("<img src=x");
    expect((window as any).__pwn).toBeUndefined();
  });

  it("개요 패널의 사건 제목도 같다", () => {
    const e = parseEvent({ id: "netx:evt:1", ts: 1794355321, module: "netx", severity: "critical", kind: "x", title: EVIL }, "netx")!;
    const root = mount(overviewPanel([view], [e]));
    expect(root.querySelectorAll("img,script")).toHaveLength(0);
    expect(root.querySelector(".frow .m")?.textContent).toContain("<img");
  });

  it("꺼짐·거부 상태에서는 자료 대신 안내만 보인다", () => {
    const root = mount(modulePanel({ ...view, status: "rejected", reason: "이름이 다릅니다", widgets: [] }, "#fff"));
    expect(root.querySelector(".kpi")).toBeNull();
    expect(root.textContent).toContain("자료를 받지 않습니다");
  });
});

describe("오늘의 말씀 (word/README.md 확인 값)", () => {
  it("날짜로 정한다", () => {
    const cases: [number, number, number, number, string][] = [
      [2026, 11, 11, 20768, "jhn.7.37-38"],
      [2026, 11, 12, 20769, "rom.6.23"],
      [2026, 11, 13, 20770, "jhn.8.12"],
      [2026, 11, 14, 20771, "jhn.10.9-11"],
      [2026, 12, 25, 20812, "jhn.14.27"],
      [2027, 1, 1, 20819, "psa.119.105"],
      [2028, 2, 29, 21243, "rom.3.23-24"],
    ];
    for (const [y, m, d, day, id] of cases) {
      expect(dayNumber(y, m, d)).toBe(day);
      expect(today("ko", y, m, d).verse.id).toBe(id);
      expect(today("en", y, m, d).verse.id).toBe(id); // 어느 언어에서나 같은 곳
    }
  });

  it("언어는 내장 목록과 글자 비교만 한다", () => {
    expect(pickLang(["ko-KR"])).toBe("ko");
    expect(pickLang(["..\\..\\x", undefined, "fr"])).toBe("en");
  });

  it("본문은 자료 묶음의 글자 그대로, 자르지 않고 보인다", () => {
    const { verse, version } = today("ko", 2026, 11, 14);
    const bundle = readRoot("word/ko.json");
    expect(verse.text).toBe(bundle.verses.find((v: any) => v.id === verse.id).text);
    const widgets = parseWidgets({ widgets: [{ id: "today", kind: "quote", text: verse.text, ref: verse.ref, version }] }, ["today"]);
    const root = mount(wordPanel({ id: "word", builtin: true, status: "on", widgets, dropped: 0 }));
    expect(root.querySelector(".verse p")?.textContent).toBe(verse.text);
    expect(root.querySelector(".verse cite")?.textContent).toBe(`${verse.ref} · ${version}`);
  });

  it("가장 긴 구절도 위젯 상한에 잘리지 않는다", () => {
    for (const lang of ["ko", "en"]) {
      const b = readRoot(`word/${lang}.json`);
      expect(Math.max(...b.verses.map((v: any) => Array.from(v.text).length))).toBeLessThan(2000);
    }
  });
});
