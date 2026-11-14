import "./ui/style.css";
import { Chassis } from "./core/chassis";
import { MockFeed } from "./feed/mock";
import { HoloConsole } from "./ui/console";
import { pickLang, wordLink } from "./word/word";

// 1단계: 로컬 목업 피드. 3단계에서 epistle 실클라이언트(등록부 발견 → 파이프 연결)로 바꾼다.
const feed = new MockFeed();
const badge = document.getElementById("badge")!;
badge.textContent = "목업 피드 · 수치는 예시";
badge.hidden = false;

// 말씀 언어: 3단계에서 HKCU → HKLM(WordLang) 값을 앞에 둔다. 지금은 표시 언어 → en → ko
const chassis = new Chassis(feed, { builtins: { word: wordLink(pickLang([navigator.language])) } });

new HoloConsole(
  chassis,
  document.getElementById("vp")!,
  document.getElementById("rail")!,
  document.getElementById("dots")!,
);

void chassis.sync();
setInterval(() => void chassis.sync(), 2000);
setInterval(() => feed.tick(), 4000);
