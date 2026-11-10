// The orbit canvas, shared by Sensors (sources and their sensors) and Workload (agents and their
// sessions, workers and jobs): every planet has moons in orbit, colour is the planet's tone (health
// or state), orbit speed is the moon's own, and a fresh signal draws a streak from the moon to its
// planet. Clicking a planet (or Tab to it and Enter) zooms in smoothly and shows the caller's panel
// beside it; Esc, the back button or a click on empty space zooms out.
//
// One canvas, one requestAnimationFrame loop, no allocation per moon per frame. The loop stops while
// the window is hidden. With reduced motion nothing orbits, the zoom jumps, there are no streaks,
// and the canvas is drawn only when something changes: colour carries the meaning. A list for
// screen readers says everything the drawing does. The layout maths is in lib/sensors-core.ts.

import { ArrowLeft } from "lucide-react";
import { type ReactNode, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { ease, hitPlanet, moonSlot, type Placed, systemLayout, zoomFor } from "../lib/sensors-core.ts";
import { useReducedMotion } from "../lib/use-reduced-motion.ts";
import { Button, cn } from "./kit.tsx";

const ZOOM_MS = 520;
const STREAK_MS = 900;
const MAX_STREAKS = 80;
// Room kept free at the bottom for the legend, which wraps to two lines on a narrow window.
const legendH = (w: number) => (w < 640 ? 56 : 36);

export type OrbitMoon<T extends string> = {
  id: string;
  name: string;
  tone: T;
  /** Orbit speed in radians a second; 0 stands still. */
  speed: number;
  /** What the screen-reader list says after the name; the tone's word when absent. */
  say?: string;
};
export type OrbitPlanet<T extends string> = {
  id: string;
  name: string;
  tone: T;
  moons: readonly OrbitMoon<T>[];
  /** A second line under the name, in the muted colour: "core 12 · ai-kit 3". */
  badge?: string;
  /** The planet's button for assistive tech: "Coordinator: working, 4 sessions. Open." */
  say: string;
};
/** How a tone is drawn: the CSS token it reads (and a fallback), its word, and whether the name under the planet stays muted. */
export type ToneSpec = { token: string; fallback: string; word: string; quiet?: boolean };

type Moon = { id: string; planet: number; ring: number; phase: number; speed: number; tone: string };
type Cam = { x: number; y: number; k: number };
type Palette = Record<string, string> & { fg: string; muted: string; border: string; card: string; font: string };

function readPalette(el: Element, tones: Record<string, ToneSpec>): Palette {
  const cs = getComputedStyle(el);
  const v = (n: string, fallback: string) => cs.getPropertyValue(n).trim() || fallback;
  const p: Palette = {
    fg: v("--foreground", "#111"),
    muted: v("--muted-foreground", "#888"),
    border: v("--border", "#ddd"),
    card: v("--card", "#fff"),
    font: getComputedStyle(document.body).fontFamily || "system-ui",
  };
  for (const [k, t] of Object.entries(tones)) p[k] = v(t.token, t.fallback);
  return p;
}

/** Moons laid out once per data change: which planet, which ring, where on it, how fast. */
export function moonsOf<T extends string>(planets: readonly OrbitPlanet<T>[], reduced: boolean): Moon[] {
  const out: Moon[] = [];
  planets.forEach((p, planet) => {
    p.moons.forEach((m, i) => {
      const { ring, phase } = moonSlot(i, p.moons.length);
      out.push({ id: m.id, planet, ring, phase, speed: reduced ? 0 : m.speed * (ring % 2 ? -1 : 1), tone: m.tone });
    });
  });
  return out;
}

export function OrbitCanvas<T extends string>({
  planets,
  tones,
  focus,
  onFocus,
  fresh,
  legend,
  panel,
  listLabel,
  backLabel,
  testid,
}: {
  /** In the order wanted, the worst first: the first sits nearest the centre. */
  planets: readonly OrbitPlanet<T>[];
  tones: Record<T, ToneSpec>;
  focus: string | null;
  onFocus: (id: string | null) => void;
  /** Moon ids that just signalled: each draws a streak to its planet. */
  fresh: readonly string[];
  /** The legend's content, bottom right of the canvas. */
  legend: ReactNode;
  /** What shows beside the canvas while a planet is in focus. */
  panel: ReactNode;
  /** The screen-reader list's name: "Sources and sensors, as a list". */
  listLabel: string;
  /** The back button's words: "All sources". */
  backLabel: string;
  testid?: string;
}) {
  const reduced = useReducedMotion();
  const wrap = useRef<HTMLDivElement>(null);
  const canvas = useRef<HTMLCanvasElement>(null);
  const [size, setSize] = useState({ w: 0, h: 0 });
  const layout = useMemo(
    () =>
      systemLayout(
        planets.map((p) => ({ id: p.id, count: p.moons.length })),
        size.w,
        Math.max(0, size.h - legendH(size.w)),
      ),
    [planets, size.w, size.h],
  );
  const moons = useMemo(() => moonsOf(planets, reduced), [planets, reduced]);
  const focused = focus ? (planets.find((p) => p.id === focus) ?? null) : null;

  // Everything the frame reads lives in one ref, so the loop never restarts on a data change.
  const st = useRef({
    layout,
    moons,
    planets,
    reduced,
    tones: tones as Record<string, ToneSpec>,
    palette: null as Palette | null,
    cam: { x: 0, y: 0, k: 1 } as Cam,
    from: { x: 0, y: 0, k: 1 } as Cam,
    to: { x: 0, y: 0, k: 1 } as Cam,
    zoomAt: 0,
    streaks: [] as { moon: number; t0: number }[],
    labels: [] as [number, number, number, number][],
    raf: 0,
    drawMs: 0,
    frames: 0,
    statAt: 0,
  });

  const draw = useCallback((t: number) => {
    const c = canvas.current;
    const s = st.current;
    if (!c || !s.palette) return false;
    const ctx = c.getContext("2d");
    if (!ctx) return false;
    const t0 = performance.now();
    const dpr = window.devicePixelRatio || 1;
    const w = c.width / dpr;
    const h = c.height / dpr;
    const P = s.palette;
    // The camera eases from where it was to where it is going.
    const z = s.zoomAt ? ease((t - s.zoomAt) / ZOOM_MS, s.reduced) : 1;
    const cam: Cam = {
      x: s.from.x + (s.to.x - s.from.x) * z,
      y: s.from.y + (s.to.y - s.from.y) * z,
      k: s.from.k + (s.to.k - s.from.k) * z,
    };
    s.cam = cam;
    const zooming = z < 1;
    ctx.setTransform(1, 0, 0, 1, 0, 0);
    ctx.clearRect(0, 0, c.width, c.height);
    ctx.setTransform(dpr * cam.k, 0, 0, dpr * cam.k, dpr * (w / 2 - cam.x * cam.k), dpr * (h / 2 - cam.y * cam.k));
    const px = 1 / cam.k;
    const { placed, scale, tracks } = s.layout;

    // The tracks planets sit on, and the centre (the account, or you).
    ctx.strokeStyle = P.border;
    ctx.lineWidth = px;
    for (const r of tracks) {
      ctx.beginPath();
      ctx.arc(w / 2, h / 2, r, 0, Math.PI * 2);
      ctx.stroke();
    }
    ctx.fillStyle = P.fg;
    ctx.globalAlpha = 0.85;
    ctx.beginPath();
    ctx.arc(w / 2, h / 2, Math.max(3, 6 * scale), 0, Math.PI * 2);
    ctx.fill();
    ctx.globalAlpha = 1;

    // Planets: a soft body in their tone's colour and a ring for each orbit.
    for (let i = 0; i < placed.length; i++) {
      const p = placed[i];
      const src = s.planets[i];
      if (!src) continue;
      const col = P[src.tone];
      const rings = Math.min(3, Math.ceil(src.moons.length / 8));
      ctx.strokeStyle = P.border;
      ctx.lineWidth = px;
      for (let r = 0; r < rings; r++) {
        ctx.beginPath();
        ctx.arc(p.x, p.y, p.r + (12 + r * 10) * scale, 0, Math.PI * 2);
        ctx.stroke();
      }
      ctx.fillStyle = col;
      ctx.globalAlpha = 0.22;
      ctx.beginPath();
      ctx.arc(p.x, p.y, p.r, 0, Math.PI * 2);
      ctx.fill();
      ctx.globalAlpha = 1;
      ctx.strokeStyle = col;
      ctx.lineWidth = 2 * px;
      ctx.stroke();
    }

    // Moons: one each, on its ring, at its speed.
    const sec = t / 1000;
    const mr = Math.max(1.6, Math.min(4, 2.8 * scale));
    const moonR = Math.min(mr, 7 / cam.k); // never bigger than 7 screen pixels
    for (let i = 0; i < s.moons.length; i++) {
      const m = s.moons[i];
      const p = placed[m.planet];
      if (!p) continue;
      const R = p.r + (12 + m.ring * 10) * scale;
      const a = m.phase + m.speed * sec;
      ctx.fillStyle = P[m.tone];
      ctx.beginPath();
      ctx.arc(p.x + Math.cos(a) * R, p.y + Math.sin(a) * R, Math.max(moonR, 2 * px), 0, Math.PI * 2);
      ctx.fill();
    }

    // Streaks: a fading line from the moon that just signalled to its planet, with a spark on it.
    let live = 0;
    if (!s.reduced) {
      for (let i = 0; i < s.streaks.length; i++) {
        const k = s.streaks[i];
        const age = (t - k.t0) / STREAK_MS;
        if (age < 0 || age >= 1) continue;
        live++;
        const m = s.moons[k.moon];
        const p = m && placed[m.planet];
        if (!p) continue;
        const R = p.r + (12 + m.ring * 10) * scale;
        const a = m.phase + m.speed * sec;
        const mx = p.x + Math.cos(a) * R;
        const my = p.y + Math.sin(a) * R;
        ctx.strokeStyle = P[m.tone];
        ctx.globalAlpha = 0.7 * (1 - age);
        ctx.lineWidth = 1.5 * px;
        ctx.beginPath();
        ctx.moveTo(mx, my);
        ctx.lineTo(p.x, p.y);
        ctx.stroke();
        ctx.globalAlpha = 1 - age;
        ctx.fillStyle = P[m.tone];
        ctx.beginPath();
        ctx.arc(mx + (p.x - mx) * age, my + (p.y - my) * age, 2.2 * px, 0, Math.PI * 2);
        ctx.fill();
      }
      ctx.globalAlpha = 1;
      if (!live && s.streaks.length) s.streaks.length = 0;
    }

    // Names (and badges) under the planets, in screen space so they stay readable at any zoom.
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.textAlign = "center";
    ctx.textBaseline = "top";
    // Worst first (the order is the caller's), kept inside the box, and a name that would land on one
    // already drawn is left out: on a narrow window the planet's button and the list still name it.
    const taken = s.labels;
    taken.length = 0;
    for (let i = 0; i < placed.length; i++) {
      const p = placed[i];
      const src = s.planets[i];
      if (!src) continue;
      ctx.font = `500 11.5px ${P.font}`;
      const text = src.name.length > 22 ? `${src.name.slice(0, 21)}…` : src.name;
      const badge = src.badge && src.badge.length > 30 ? `${src.badge.slice(0, 29)}…` : src.badge;
      const tw = Math.max(ctx.measureText(text).width, badge ? ctx.measureText(badge).width * 0.95 : 0);
      const th = badge ? 27 : 14;
      let sx = w / 2 + (p.x - cam.x) * cam.k;
      const sy = h / 2 + (p.y - cam.y) * cam.k + p.orbit * cam.k + 4;
      if (sx < -80 || sx > w + 80 || sy < -20 || sy > h + 20) continue;
      sx = Math.min(Math.max(sx, tw / 2 + 4), w - tw / 2 - 4);
      const x0 = sx - tw / 2 - 3;
      const x1 = sx + tw / 2 + 3;
      if (taken.some((r) => x0 < r[1] && x1 > r[0] && sy < r[3] && sy + th > r[2])) continue;
      taken.push([x0, x1, sy, sy + th]);
      ctx.fillStyle = s.tones[src.tone]?.quiet ? P.muted : P[src.tone];
      ctx.fillText(text, sx, sy);
      if (badge) {
        ctx.font = `400 10.5px ${P.font}`;
        ctx.fillStyle = P.muted;
        ctx.fillText(badge, sx, sy + 13);
      }
    }

    // Draw time, kept for the 200-moon check (scripts/sensors-shots.mjs reads data-draw-ms).
    s.drawMs += performance.now() - t0;
    s.frames++;
    if (t - s.statAt > 1000) {
      c.dataset.drawMs = (s.drawMs / Math.max(1, s.frames)).toFixed(3);
      c.dataset.fps = String(Math.round((s.frames * 1000) / Math.max(1, t - s.statAt)));
      s.drawMs = 0;
      s.frames = 0;
      s.statAt = t;
    }
    return !s.reduced || zooming || live > 0;
  }, []);

  // The loop: runs while anything moves; with reduced motion only when something changed.
  const kick = useCallback(() => {
    const s = st.current;
    if (s.raf || document.hidden) return;
    const frame = (t: number) => {
      s.raf = 0;
      if (document.hidden) return;
      if (draw(t)) s.raf = requestAnimationFrame(frame);
    };
    s.raf = requestAnimationFrame(frame);
  }, [draw]);

  useEffect(() => {
    const onVis = () => {
      const s = st.current;
      if (document.hidden) {
        cancelAnimationFrame(s.raf);
        s.raf = 0;
      } else kick();
    };
    document.addEventListener("visibilitychange", onVis);
    const held = st.current;
    return () => {
      document.removeEventListener("visibilitychange", onVis);
      cancelAnimationFrame(held.raf);
      held.raf = 0;
    };
  }, [kick]);

  // Size: the canvas follows its box at the screen's pixel density.
  useEffect(() => {
    const el = wrap.current;
    if (!el) return;
    const ro = new ResizeObserver(([e]) => {
      const w = Math.round(e.contentRect.width);
      const h = Math.round(e.contentRect.height);
      setSize((o) => (o.w === w && o.h === h ? o : { w, h }));
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, []);
  useEffect(() => {
    const c = canvas.current;
    if (!c || !size.w) return;
    const dpr = window.devicePixelRatio || 1;
    c.width = Math.round(size.w * dpr);
    c.height = Math.round(size.h * dpr);
    kick();
  }, [size, kick]);

  // Theme: the colours are the page's tokens, read again when light and dark switch.
  useEffect(() => {
    const read = () => {
      if (wrap.current) st.current.palette = readPalette(wrap.current, st.current.tones);
      kick();
    };
    read();
    const q = window.matchMedia("(prefers-color-scheme: dark)");
    q.addEventListener("change", read);
    const mo = new MutationObserver(read);
    mo.observe(document.documentElement, { attributes: true, attributeFilter: ["class", "style", "data-theme"] });
    return () => {
      q.removeEventListener("change", read);
      mo.disconnect();
    };
  }, [kick]);

  // The camera: home, or framing the focused planet.
  const home = useMemo<Cam>(() => ({ x: size.w / 2, y: size.h / 2, k: 1 }), [size.w, size.h]);
  const target = useMemo<Cam>(() => {
    const p = focus ? layout.placed.find((x) => x.id === focus) : null;
    return p ? zoomFor(p, size.w, size.h) : home;
  }, [focus, layout, size.w, size.h, home]);
  const first = useRef(true);
  useEffect(() => {
    const s = st.current;
    if (first.current || !size.w) {
      // The first frame starts where it should be, with no fly-in.
      s.cam = s.from = s.to = target;
      s.zoomAt = 0;
      if (size.w) first.current = false;
    } else {
      s.from = { ...s.cam };
      s.to = target;
      s.zoomAt = performance.now();
    }
    kick();
  }, [target, size.w, kick]);

  // Data changed: hand it to the frame and draw it (this matters with reduced motion, when nothing else asks for a frame).
  useEffect(() => {
    const s = st.current;
    s.layout = layout;
    s.moons = moons;
    s.planets = planets;
    s.reduced = reduced;
    s.tones = tones;
    kick();
  }, [layout, moons, planets, reduced, tones, kick]);

  // Streaks for fresh signals.
  useEffect(() => {
    const s = st.current;
    if (s.reduced || !fresh.length) return;
    const at = performance.now();
    const index = new Map(s.moons.map((m, i) => [m.id, i]));
    for (const id of fresh) {
      const i = index.get(id);
      // Spread a burst over the first half second so one poll doesn't flash all at once.
      if (i != null && s.streaks.length < MAX_STREAKS) s.streaks.push({ moon: i, t0: at + Math.random() * 500 });
    }
    kick();
  }, [fresh, kick]);

  // Esc zooms back out.
  useEffect(() => {
    if (!focus) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || e.defaultPrevented) return;
      e.preventDefault();
      onFocus(null);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [focus, onFocus]);

  // Back on the planet's own button after zooming out, so Tab carries on from there.
  const lastFocus = useRef<string | null>(null);
  useEffect(() => {
    if (focus) lastFocus.current = focus;
    else if (lastFocus.current) {
      const id = lastFocus.current;
      lastFocus.current = null;
      requestAnimationFrame(() =>
        wrap.current?.querySelector<HTMLButtonElement>(`[data-planet="${CSS.escape(id)}"]`)?.focus(),
      );
    }
  }, [focus]);

  const onClick = (e: React.MouseEvent<HTMLCanvasElement>) => {
    const r = e.currentTarget.getBoundingClientRect();
    const cam = st.current.cam;
    const x = (e.clientX - r.left - size.w / 2) / cam.k + cam.x;
    const y = (e.clientY - r.top - size.h / 2) / cam.k + cam.y;
    const hit = hitPlanet(layout.placed, x, y);
    onFocus(hit && hit !== focus ? hit : null);
  };

  return (
    <div
      className={focused ? "grid grid-cols-1 gap-4 @3xl/page:grid-cols-[minmax(0,1fr)_340px]" : ""}
      data-testid={testid}
      data-focus={focus ?? ""}
    >
      <div
        ref={wrap}
        className="bg-card relative h-[min(64vh,600px)] min-h-[300px] overflow-hidden rounded-xl border shadow-xs"
      >
        <canvas
          ref={canvas}
          aria-hidden
          onClick={onClick}
          className="absolute inset-0 size-full cursor-pointer"
          data-reduced={reduced ? "1" : "0"}
        />
        {!focused &&
          layout.placed.map((p: Placed, i) => {
            const src = planets[i];
            if (!src) return null;
            const d = Math.max(28, p.orbit * 2);
            return (
              <button
                key={p.id}
                type="button"
                data-planet={p.id}
                aria-label={src.say}
                onClick={() => onFocus(p.id)}
                className="focus-visible:ring-ring absolute rounded-full outline-none focus-visible:ring-2"
                style={{ left: p.x - d / 2, top: p.y - d / 2, width: d, height: d }}
              />
            );
          })}
        {focused && (
          <Button
            size="sm"
            variant="outline"
            className="bg-card/90 absolute top-3 left-3 backdrop-blur"
            onClick={() => onFocus(null)}
          >
            <ArrowLeft /> {backLabel}
          </Button>
        )}
        <div
          className={cn(
            "bg-card/85 text-muted-foreground pointer-events-none absolute right-3 bottom-3 flex flex-wrap items-center gap-x-3 gap-y-1 rounded-md px-2 py-1 text-[11px] backdrop-blur max-sm:left-3",
          )}
          aria-hidden
        >
          {legend}
        </div>
      </div>
      {focused && panel}
      <section className="sr-only" aria-label={listLabel}>
        <ul>
          {planets.map((p) => (
            <li key={p.id}>
              <button type="button" tabIndex={-1} onClick={() => onFocus(p.id)}>
                {p.name}: {tones[p.tone].word}
              </button>
              <ul>
                {p.moons.map((m) => (
                  <li key={m.id}>
                    {m.name}: {m.say ?? tones[m.tone].word}
                  </li>
                ))}
              </ul>
            </li>
          ))}
        </ul>
      </section>
    </div>
  );
}
