// Sensors (PRD 0117; RFC 0217 ports it to the sdk UI): the pure part, with no window, no API and no clock of its own,
// so every rule here is tested in test/sensors-core.test.ts.
//
// A source (a machine, a service, a connection, the coordinator, the asks queue, this Mac) is a planet;
// its sensors orbit it. Health is the colour, the signal rate is the orbit speed, and a sensor that
// missed three of its intervals is silent (critique 11). Nothing here invents a reading: a value the
// source did not send stays null and the page says so.

import type { StreamEvent } from "./event-stream-core.ts";

export type Health = "fine" | "degraded" | "failing" | "silent";
export const HEALTHS: Health[] = ["failing", "degraded", "fine", "silent"];
export const HEALTH_WORD: Record<Health, string> = { fine: "Fine", degraded: "Degraded", failing: "Failing", silent: "Silent" };
/** The CSS colour token each health is drawn with (frame/tokens.css). */
export const HEALTH_TOKEN: Record<Health, string> = { fine: "--success", degraded: "--warning", failing: "--destructive", silent: "--muted-foreground" };

export type SensorState = "active" | "paused" | "revoked";
export type SourceKind = "machine" | "service" | "connection" | "coordinator" | "asks" | "mac";

export type Reading = { at: number; value: number | null; ok: boolean | null; server_at?: number | null };

export type Sensor = {
  id: string;
  name: string;
  source_id: string;
  /** What it measures, in a word: "http", "latency", "jobs", "cpu", … */
  kind: string;
  state: SensorState;
  health: Health;
  interval_secs: number | null;
  /** Signals a minute, from the readings when there are enough, else from the interval. */
  rate_per_min: number;
  last_seen: number | null;
  value: number | null;
  unit: string;
  /** The reading in words when a number would mislead (memory pressure "Normal", not "1"). */
  text?: string;
  readings: readonly Reading[];
  owner?: string;
  producer?: string;
  schema_version?: number;
  /** The health word the service (or the newest reading) last said, before staleness is applied. */
  raw?: string;
  /** Built in (derived by the service from monitors, incidents, the coordinator, the asks): read only. */
  builtin?: boolean;
  /** Only a sensor that is not built in and not already revoked can be revoked. */
  revocable: boolean;
};

export type Source = { id: string; name: string; kind: SourceKind; detail: string; health: Health; sensors: Sensor[] };

/** "api" is what the sensors service answered; "sample" is a labelled stub (never drawn as a measurement). */
export type Origin = "api" | "sample";
export type Snapshot = { origin: Origin; label: string; sources: Source[]; at: number; problems: string[] };

// ---------------------------------------------------------------- health

const FINE = new Set(["fine", "ok", "up", "healthy", "pass", "passing", "green", "good"]);
const DEGRADED = new Set(["degraded", "warn", "warning", "slow", "flapping", "amber", "partial"]);
const FAILING = new Set(["failing", "fail", "failed", "down", "error", "critical", "red", "bad", "outage"]);

/** A health word from any source mapped onto the four colours; unknown, off and empty are silent. */
export function healthWord(raw: string | null | undefined): Health {
  const w = (raw ?? "").trim().toLowerCase();
  if (FINE.has(w)) return "fine";
  if (DEGRADED.has(w)) return "degraded";
  if (FAILING.has(w)) return "failing";
  return "silent";
}

/** Stale after three missed intervals (critique 11). A sensor never seen is stale; one with no interval never goes stale by time. */
export function isStale(lastSeen: number | null, intervalSecs: number | null, now: number): boolean {
  if (lastSeen == null) return true;
  if (!intervalSecs || intervalSecs <= 0) return false;
  return now - lastSeen > 3 * intervalSecs * 1000;
}

/** A sensor's colour: paused, revoked and stale are silent whatever they last said. */
export function healthOf(s: { raw: string | null | undefined; state: SensorState; last_seen: number | null; interval_secs: number | null }, now: number): Health {
  if (s.state !== "active") return "silent";
  if (isStale(s.last_seen, s.interval_secs, now)) return "silent";
  return healthWord(s.raw);
}

/** A planet's colour: its worst speaking sensor; silent only when every sensor is silent. */
export function rollup(healths: readonly Health[]): Health {
  if (healths.includes("failing")) return "failing";
  if (healths.includes("degraded")) return "degraded";
  if (healths.includes("fine")) return "fine";
  return "silent";
}

export function sensorState(raw: string | null | undefined): SensorState {
  const w = (raw ?? "").toLowerCase();
  return w === "paused" ? "paused" : w === "revoked" ? "revoked" : "active";
}

// ---------------------------------------------------------------- rate and motion

/** Signals a minute: counted over the readings of the last hour when there are two or more, else 60 / interval. */
export function ratePerMin(intervalSecs: number | null, readings: readonly Reading[], now: number): number {
  const hour = readings.filter((r) => r.at <= now && now - r.at <= 3_600_000);
  if (hour.length >= 2) {
    const first = Math.min(...hour.map((r) => r.at));
    const span = Math.max(60_000, now - first);
    return (hour.length / span) * 60_000;
  }
  return intervalSecs && intervalSecs > 0 ? 60 / intervalSecs : 0;
}

/**
 * Orbit speed in radians a second. Log-scaled so one signal a minute still visibly moves and sixty a
 * minute does not blur: 0.08 + 0.9 × log10(1 + rate) / log10(61), capped at 0.98. Silent sensors stand
 * still, and with reduced motion nothing orbits (the colour still says everything).
 */
export function orbitSpeed(rate: number, health: Health, reducedMotion: boolean): number {
  if (reducedMotion || health === "silent" || !(rate > 0)) return 0;
  return 0.08 + 0.9 * Math.min(1, Math.log10(1 + rate) / Math.log10(61));
}

/** The rate in words for a tile: "4 a minute", "1 every 5 min", "none lately". */
export function rateText(rate: number): string {
  if (!(rate > 0)) return "none lately";
  if (rate >= 1) return `${rate >= 10 ? Math.round(rate) : Math.round(rate * 10) / 10} a minute`;
  const every = Math.round(1 / rate);
  return every >= 60 ? `1 every ${Math.round(every / 60)} h` : `1 every ${every} min`;
}

export function agoText(at: number | null, now: number): string {
  if (at == null) return "never";
  const s = Math.max(0, Math.round((now - at) / 1000));
  if (s < 5) return "just now";
  if (s < 60) return `${s} s ago`;
  if (s < 3600) return `${Math.round(s / 60)} min ago`;
  if (s < 86_400) return `${Math.round(s / 3600)} h ago`;
  return `${Math.round(s / 86_400)} d ago`;
}

export function valueText(s: Pick<Sensor, "value" | "unit" | "text">): string {
  if (s.text) return s.text;
  if (s.value == null) return "no reading";
  const v = s.value;
  if (s.unit === "ms") return v >= 1000 ? `${(v / 1000).toFixed(1)} s` : `${Math.round(v)} ms`;
  if (s.unit === "%") return `${Math.round(v)}%`;
  if (s.unit === "GB") return `${v.toFixed(v >= 100 ? 0 : 1)} GB`;
  const n = Number.isInteger(v) ? String(v) : v.toFixed(2);
  return s.unit ? `${n} ${s.unit}` : n;
}

// ---------------------------------------------------------------- the solar system layout

export type Placed = { id: string; x: number; y: number; r: number; orbit: number };

/** A planet's own size and the radius of its outermost sensor ring, before scaling. */
export function planetSize(sensors: number): { r: number; orbit: number } {
  const r = 9 + Math.min(13, Math.sqrt(Math.max(1, sensors)) * 3);
  const rings = Math.min(3, Math.ceil(sensors / 8));
  return { r, orbit: r + 12 + Math.max(0, rings - 1) * 10 };
}

/** Which ring a sensor sits on and where on it (rings of 8, inner first). */
export function moonSlot(index: number, count: number): { ring: number; phase: number } {
  const ring = Math.min(2, Math.floor(index / 8));
  const onRing = ring < 2 ? Math.min(8, count - ring * 8) : count - 16;
  const k = index - ring * 8;
  return { ring, phase: (k / Math.max(1, onRing)) * Math.PI * 2 + ring * 0.4 };
}

/**
 * Planets on concentric tracks around the centre (the account), the worst first and nearest, each
 * track holding as many as fit without touching. Everything is scaled down to fit `w × h`.
 */
export function systemLayout(sources: readonly { id: string; count: number }[], w: number, h: number): { placed: Placed[]; scale: number; tracks: number[] } {
  const sizes = sources.map((s) => ({ id: s.id, ...planetSize(s.count) }));
  if (!sizes.length || w <= 0 || h <= 0) return { placed: [], scale: 1, tracks: [] };
  const cell = Math.max(...sizes.map((s) => s.orbit)) * 2 + 34;
  const placed: Placed[] = [];
  const tracks: number[] = [];
  let i = 0;
  let R = cell * 0.95;
  while (i < sizes.length) {
    const fit = Math.max(1, Math.floor((2 * Math.PI * R) / cell));
    const take = Math.min(fit, sizes.length - i);
    tracks.push(R);
    const offset = tracks.length * 0.7;
    for (let k = 0; k < take; k++, i++) {
      const a = offset + (k / take) * Math.PI * 2;
      placed.push({ id: sizes[i].id, x: Math.cos(a) * R, y: Math.sin(a) * R, r: sizes[i].r, orbit: sizes[i].orbit });
    }
    R += cell;
  }
  const outer = tracks[tracks.length - 1] + cell / 2;
  const scale = Math.min(1.6, (Math.min(w, h) / 2 - 8) / outer);
  return {
    placed: placed.map((p) => ({ id: p.id, x: w / 2 + p.x * scale, y: h / 2 + p.y * scale, r: p.r * scale, orbit: p.orbit * scale })),
    scale,
    tracks: tracks.map((t) => t * scale),
  };
}

/** The camera that frames one planet: its orbit fills about 70% of the shorter side. */
export function zoomFor(p: Placed, w: number, h: number): { x: number; y: number; k: number } {
  return { x: p.x, y: p.y, k: Math.min(6, (Math.min(w, h) * 0.35) / Math.max(1, p.orbit)) };
}

/** Smooth step for the zoom (ease in and out); reduced motion jumps straight to the end. */
export function ease(t: number, reducedMotion: boolean): number {
  if (reducedMotion) return 1;
  const x = Math.max(0, Math.min(1, t));
  return x * x * (3 - 2 * x);
}

/** The planet under a point, or null (outside every planet's orbit). */
export function hitPlanet(placed: readonly Placed[], x: number, y: number): string | null {
  let best: { id: string; d: number } | null = null;
  for (const p of placed) {
    const d = Math.hypot(p.x - x, p.y - y);
    if (d <= p.orbit + 6 && (!best || d < best.d)) best = { id: p.id, d };
  }
  return best?.id ?? null;
}

// ---------------------------------------------------------------- order, pins, events

const RANK: Record<Health, number> = { failing: 0, degraded: 1, fine: 2, silent: 3 };
export const byHealth = <T extends { health: Health; name: string }>(a: T, b: T) => RANK[a.health] - RANK[b.health] || a.name.localeCompare(b.name);

/** Control room order: pinned first in pin order, then worst first, then by name. */
export function tileOrder(sensors: readonly Sensor[], pins: readonly string[]): Sensor[] {
  const pinned = pins.map((id) => sensors.find((s) => s.id === id)).filter((s): s is Sensor => !!s);
  const rest = sensors.filter((s) => !pins.includes(s.id)).sort(byHealth);
  return [...pinned, ...rest];
}

export function togglePin(pins: readonly string[], id: string): string[] {
  return pins.includes(id) ? pins.filter((p) => p !== id) : [id, ...pins];
}

export type SignalEvent = { id: string; at: number; sensor_id: string; source_id: string; health: Health; text: string; kind: "signal" | "change" };

/**
 * What happened between two snapshots: a new reading (a signal, drawn as a streak) and a change of
 * health (a line in the stream). The stream is built from what the page saw, newest first.
 */
export function diffEvents(prev: Snapshot | null, next: Snapshot): SignalEvent[] {
  if (!prev) return [];
  const before = new Map(prev.sources.flatMap((s) => s.sensors).map((s) => [s.id, s]));
  const out: SignalEvent[] = [];
  for (const src of next.sources) {
    for (const s of src.sensors) {
      const b = before.get(s.id);
      if (!b) {
        out.push({ id: `${s.id}@new@${next.at}`, at: next.at, sensor_id: s.id, source_id: src.id, health: s.health, text: `${s.name} appeared on ${src.name}`, kind: "change" });
        continue;
      }
      if (b.health !== s.health)
        out.push({ id: `${s.id}@${s.health}@${next.at}`, at: next.at, sensor_id: s.id, source_id: src.id, health: s.health, text: `${s.name} on ${src.name}: ${HEALTH_WORD[b.health].toLowerCase()} → ${HEALTH_WORD[s.health].toLowerCase()}`, kind: "change" });
      if (s.last_seen != null && (b.last_seen == null || s.last_seen > b.last_seen))
        out.push({ id: `${s.id}@sig@${s.last_seen}`, at: s.last_seen, sensor_id: s.id, source_id: src.id, health: s.health, text: `${s.name}: ${valueText(s)}`, kind: "signal" });
    }
  }
  return out.sort((a, b) => b.at - a.at);
}

/** The stream kept on screen: newest first, no repeats, at most `max`. */
export function mergeEvents(old: readonly SignalEvent[], fresh: readonly SignalEvent[], max = 200): SignalEvent[] {
  const seen = new Set<string>();
  return [...fresh, ...old].filter((e) => (seen.has(e.id) ? false : (seen.add(e.id), true))).sort((a, b) => b.at - a.at).slice(0, max);
}

/** A signal event as a line of the shared event stream (components/event-stream.tsx): its health is the tone, its source the group, a change is said in bold. */
export const streamEvent = (e: SignalEvent): StreamEvent<Health> => ({ id: e.id, at: e.at, tone: e.health, text: e.text, group: e.source_id, strong: e.kind === "change" });

// ---------------------------------------------------------------- sparkline

/** SVG polyline points for a sparkline of `w × h`; null when fewer than two readings carry a value. */
export function sparkPoints(readings: readonly Reading[], w: number, h: number): string | null {
  const pts = readings.filter((r) => r.value != null).sort((a, b) => a.at - b.at);
  if (pts.length < 2) return null;
  const t0 = pts[0].at;
  const t1 = pts[pts.length - 1].at;
  const vs = pts.map((p) => p.value as number);
  const lo = Math.min(...vs);
  const hi = Math.max(...vs);
  const x = (t: number) => (t1 === t0 ? 0 : ((t - t0) / (t1 - t0)) * w);
  const y = (v: number) => (hi === lo ? h / 2 : h - 1 - ((v - lo) / (hi - lo)) * (h - 2));
  return pts.map((p) => `${x(p.at).toFixed(1)},${y(p.value as number).toFixed(1)}`).join(" ");
}

// ---------------------------------------------------------------- sessions

export type SessionPreview = {
  id: string;
  /** The thread a view request goes to; null when the session has none. */
  thread_id: string | null;
  title: string;
  state: string;
  last_step: string;
  starter: string;
  at: number | null;
  mine: boolean;
  kind: "session" | "ask";
};

/** Whose it is: the caller's own when they started it (subject, user id or email). */
export function isMine(startedBy: readonly (string | null | undefined)[], me: { subject?: string; user_id?: string; email?: string }): boolean {
  const ids = [me.subject, me.user_id, me.email].filter((x): x is string => !!x).map((x) => x.toLowerCase());
  return startedBy.some((s) => !!s && ids.includes(s.toLowerCase()));
}

/** What someone else's session shows: that it runs, and nothing it says. */
export function redact(s: SessionPreview): SessionPreview {
  return s.mine ? s : { ...s, title: "A session is running", last_step: "", starter: "" };
}

const LIVE_STATES = new Set(["queued", "running", "building", "waiting", "awaiting", "held", "stuck", "open", "started", "in_progress", "pending", "claimed"]);
export const isLiveState = (s: string) => LIVE_STATES.has(s.toLowerCase());
