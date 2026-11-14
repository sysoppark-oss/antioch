// 3D 코버플로우: 드래그/스와이프 → 관성 안착, 옆 패널 누르면 가운데로, 가운데 다시 누르면 확대.
// 승인 맛보기의 동작을 그대로 옮기고, 패널 수가 바뀌어도 돌게 했다.

import type { Chassis } from "../core/chassis";
import { emptyPanel, h, modulePanel, overviewPanel, slotColor, wordPanel } from "./panels";

interface Card {
  key: string;
  el: HTMLElement;
  body: HTMLElement;
}

export class HoloConsole {
  private cards: Card[] = [];
  private current = 0;
  private zoom = false;
  private spacing = 300;
  private raf = 0;
  /** 첫 모듈이 나타났을 때 한 번, 그 패널을 가운데로 가져왔는가 */
  private centered = false;
  private readonly reduce = window.matchMedia?.("(prefers-reduced-motion: reduce)").matches ?? false;

  constructor(
    private readonly chassis: Chassis,
    private readonly vp: HTMLElement,
    private readonly rail: HTMLElement,
    private readonly dots: HTMLElement,
  ) {
    this.measure();
    window.addEventListener("resize", () => {
      this.measure();
      this.place();
    });
    this.bindPointer();
    window.addEventListener("keydown", (e) => {
      if (e.key === "ArrowRight") this.go(Math.round(this.current) + 1, false);
      else if (e.key === "ArrowLeft") this.go(Math.round(this.current) - 1, false);
      else if (e.key === "Enter" || e.key === " ") this.go(Math.round(this.current), !this.zoom);
    });
    chassis.onChange(() => this.update());
    this.update();
  }

  private measure(): void {
    // 좁은 화면은 맛보기 그대로(폭의 64%), 넓은 화면은 옆 패널이 가운데 패널을 가리지 않을 만큼 벌린다
    this.spacing = Math.min(window.innerWidth * 0.64, 430);
  }

  /** 섀시 상태 → 패널. 순서: 시스템 개요, 모듈들, 말씀, 빈 슬롯 */
  update(): void {
    const slots = this.chassis.slots;
    const mods = slots.filter((v) => !v.builtin);
    const word = slots.find((v) => v.id === "word");
    const plan: { key: string; color: string; dim: boolean; fill: () => HTMLElement[] }[] = [
      { key: "@overview", color: "var(--sy)", dim: false, fill: () => overviewPanel(slots, this.chassis.events) },
      ...mods.map((v, i) => ({
        key: v.id,
        color: v.status === "on" ? slotColor(v.id, i) : "var(--none)",
        dim: v.status !== "on",
        fill: () => modulePanel(v, slotColor(v.id, i)),
      })),
      ...(word ? [{ key: "@word", color: "var(--wd)", dim: false, fill: () => wordPanel(word) }] : []),
      { key: "@empty", color: "var(--none)", dim: true, fill: emptyPanel },
    ];

    const keys = plan.map((p) => p.key).join("|");
    if (keys !== this.cards.map((c) => c.key).join("|")) {
      const focusKey = this.cards[Math.round(this.current)]?.key;
      const old = new Map(this.cards.map((c) => [c.key, c]));
      this.cards = plan.map((p) => old.get(p.key) ?? this.makeCard(p.key));
      this.rail.replaceChildren(...this.cards.map((c) => c.el));
      this.dots.replaceChildren(...this.cards.map(() => h("i")));
      const keep = this.cards.findIndex((c) => c.key === focusKey);
      this.current = keep >= 0 ? keep : Math.min(Math.round(this.current), this.cards.length - 1);
      // 처음 모듈이 나타나면 그 패널을 가운데 둔다(그 뒤로는 사용자가 둔 자리를 건드리지 않는다)
      if (!this.centered && mods.length) {
        this.centered = true;
        this.current = 1;
      }
    }
    plan.forEach((p, i) => {
      const c = this.cards[i]!;
      c.el.style.setProperty("--pc", p.color);
      c.el.classList.toggle("dimmed", p.dim);
      c.el.dataset.i = String(i);
      c.body.replaceChildren(...p.fill());
    });
    this.place();
  }

  private makeCard(key: string): Card {
    const el = h("section", "card");
    for (const k of ["tl", "tr", "bl", "br2"]) el.append(h("span", `br ${k}`));
    const body = h("div");
    el.append(body);
    return { key, el, body };
  }

  private place(): void {
    const ci = Math.round(this.current);
    this.cards.forEach((c, i) => {
      const off = i - this.current,
        a = Math.abs(off),
        isC = i === ci;
      const tx = off * this.spacing,
        tz = isC ? 70 : -a * 250 - 50,
        ry = Math.max(-34, Math.min(34, -off * 25)),
        sc = isC ? (this.zoom ? 1.14 : 1) : 0.8;
      const st = c.el.style;
      st.transform = `translate(-50%,-50%) translateX(${tx}px) translateZ(${tz}px) rotateY(${ry}deg) scale(${sc})`;
      st.zIndex = String(100 - Math.round(a * 10));
      st.opacity = String(a > 2.3 ? 0 : 1 - a * 0.26);
      st.filter = isC ? "none" : "brightness(.78) saturate(.85)";
      c.el.classList.toggle("focus", isC);
      c.el.setAttribute("aria-hidden", String(!isC));
    });
    [...this.dots.children].forEach((d, i) => d.classList.toggle("on", i === ci));
  }

  private go(target: number, zoom: boolean): void {
    const t = Math.max(0, Math.min(this.cards.length - 1, target));
    this.zoom = zoom;
    cancelAnimationFrame(this.raf);
    if (this.reduce) {
      this.current = t;
      this.place();
      return;
    }
    const s = this.current,
      ch = t - s,
      st = performance.now(),
      dur = 460;
    const step = (now: number): void => {
      const k = Math.min(1, (now - st) / dur),
        e = 1 - Math.pow(1 - k, 3);
      this.current = k < 1 ? s + ch * e : t;
      this.place();
      if (k < 1) this.raf = requestAnimationFrame(step);
    };
    this.raf = requestAnimationFrame(step);
  }

  private bindPointer(): void {
    let drag = false,
      sx = 0,
      sc0 = 0,
      lx = 0,
      lt = 0,
      vel = 0,
      moved = 0;
    const vp = this.vp;
    vp.addEventListener("pointerdown", (e) => {
      drag = true;
      sx = lx = e.clientX;
      sc0 = this.current;
      lt = performance.now();
      vel = 0;
      moved = 0;
      this.centered = true;
      cancelAnimationFrame(this.raf);
      vp.classList.add("grab");
      try {
        vp.setPointerCapture(e.pointerId);
      } catch {
        /* 캡처를 못 해도 끌기는 된다 */
      }
    });
    vp.addEventListener("pointermove", (e) => {
      if (!drag) return;
      const now = performance.now(),
        dx = e.clientX - lx,
        dt = now - lt || 16;
      moved += Math.abs(dx);
      this.current = Math.max(-0.35, Math.min(this.cards.length - 1 + 0.35, sc0 - (e.clientX - sx) / this.spacing));
      vel = dx / dt;
      lx = e.clientX;
      lt = now;
      this.place();
    });
    const end = (e: PointerEvent): void => {
      if (!drag) return;
      drag = false;
      vp.classList.remove("grab");
      if (moved < 8) {
        // 포인터를 붙잡아 두면 target 이 뷰포트가 되므로 좌표로 패널을 찾는다
        const hit = document.elementFromPoint(e.clientX, e.clientY)?.closest<HTMLElement>(".card");
        if (hit) {
          const i = Number(hit.dataset.i);
          this.go(i, Math.round(this.current) === i ? !this.zoom : false);
        } else this.go(Math.round(this.current), false);
        return;
      }
      this.go(Math.round(this.current - vel * 70), false);
    };
    vp.addEventListener("pointerup", end);
    vp.addEventListener("pointercancel", end);
  }
}
