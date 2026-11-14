// "오늘의 말씀" 내장 슬롯 (설계 문서 11장 — 확정). 규칙은 저장소 word/README.md 가 기준이다.
// 본문은 자료 묶음 파일을 그대로 읽는다. 여기서 글자를 고치거나 자르지 않는다. 네트워크를 쓰지 않는다.

import ko from "../../../word/ko.json";
import en from "../../../word/en.json";
import type { ModuleLink } from "../feed/feed";

export interface Verse {
  id: string;
  ref: string;
  group: string;
  long: boolean;
  text: string;
}
interface Edition {
  lang: string;
  version: string;
  verses: Verse[];
}

const EDITIONS: Record<string, Edition> = { ko: ko as Edition, en: en as Edition };
export const WORD_LANGS = Object.keys(EDITIONS);

/** 현지 날짜가 1970-01-01 부터 며칠째인가 (1970-01-01 이 0) */
export function dayNumber(y: number, m: number, d: number): number {
  return Math.floor(Date.UTC(y, m - 1, d) / 86_400_000);
}

/** 내장 언어 목록과 글자 비교만 한다. 없으면 en → ko */
export function pickLang(candidates: readonly (string | undefined)[]): string {
  for (const c of [...candidates, "en", "ko"]) {
    const base = c?.toLowerCase().split("-")[0];
    if (base && WORD_LANGS.includes(base)) return base;
  }
  return "ko";
}

/** 오늘의 말씀: 날짜로 정한다. 무작위를 쓰지 않는다. */
export function today(lang: string, y: number, m: number, d: number): { verse: Verse; version: string } {
  const ed = EDITIONS[lang] ?? EDITIONS.ko!;
  const day = dayNumber(y, m, d);
  const g = ed.verses.filter((v) => v.group === "gospel");
  const o = ed.verses.filter((v) => v.group !== "gospel");
  const t = Math.floor(day / 3);
  const verse = day % 3 === 0 ? g[t % g.length]! : o[(day - t - 1) % o.length]!;
  return { verse, version: ed.version };
}

/** 섀시 안에서 바로 답하는 내장 슬롯. 모양은 rack.describe / rack.widgets 와 같다 (11.7) */
export function wordLink(lang: string, clock: () => Date = () => new Date()): ModuleLink {
  return {
    hello: async () => ({ module: "word", module_version: "0.1.0", api: [1, 0], granted: ["read"] }),
    describe: async () => ({
      id: "word",
      title: "오늘의 말씀",
      kind: "builtin",
      version: "0.1.0",
      api: [1, 0],
      summary: "오늘의 성경 구절",
      capabilities: ["widgets"],
      namespaces: [],
      widgets: [{ id: "today", title: "오늘의 말씀", kind: "quote", sizes: ["2x1", "2x2"], refresh_sec: 3600 }],
      intents_in: [],
      intents_out: [],
      actions: [],
    }),
    health: async () => ({ state: "ok", since: 1, notes: [], checks: [] }),
    widgets: async () => {
      const n = clock();
      const { verse, version } = today(lang, n.getFullYear(), n.getMonth() + 1, n.getDate());
      return {
        widgets: [
          { id: "today", kind: "quote", text: verse.text, ref: verse.ref, version, open: { view: "word", ref: verse.id } },
        ],
      };
    },
    subscribe: async () => {}, // 말씀은 rack.event 를 내지 않는다 (11.2 넣지 않는 자리)
    close: () => {},
  };
}
