// What the sensors service and the coordinator answer, mapped onto the page's own shapes (PRD 0117;
// RFC 0217 ports the Mac's Sensors to the sdk UI). Pure: no window, no clock of its own, so
// test/sensors-wire.test.ts checks every rule. Nothing is invented: a value the service did not send
// stays null and the page says so.
//
// Wire shapes are the REST form of proto/iohr/sensors/v1/sensors.proto: enum words are lower case
// strings (state active, paused, revoked; health ok, stale, silent, up, down), times are RFC 3339.

import {
  healthOf,
  isLiveState,
  isMine,
  rollup,
  sensorState,
  type Health,
  type Reading,
  type Sensor,
  type SessionPreview,
  type Source,
  type SourceKind,
} from "./sensors-core.ts";

const ms = (iso: string | null | undefined): number | null => {
  if (!iso) return null;
  const t = Date.parse(iso);
  return Number.isFinite(t) ? t : null;
};

const source = (id: string, name: string, kind: SourceKind, detail: string, sensors: Sensor[]): Source => ({
  id,
  name,
  kind,
  detail,
  sensors,
  health: rollup(sensors.map((s) => s.health)),
});

// ---------------------------------------------------------------- the sensors service

/** One sensor as GET /v1/atlas/sensors answers it. The list carries no value: the newest reading supplies it. */
export type WireSensor = {
  sensor_id: string;
  name?: string;
  producer_kind?: string;
  producer_ref?: string;
  kind?: string;
  unit?: string;
  /** mac, server, agent, phone, connection, monitors, incidents, coordinator or asks. */
  source?: string;
  state?: string;
  health?: string;
  interval_seconds?: number | null;
  rate_per_minute?: number | null;
  last_seen_at?: string | null;
  owner?: string;
  schema_version?: number;
  builtin?: boolean;
};

/** The list answers by source: `groups[].sensors[]`. */
export type WireSensors = { groups?: { source?: string; sensors?: WireSensor[] }[]; next_page_token?: string };

export const SOURCE_NAME: Record<string, string> = {
  mac: "Macs",
  server: "Servers",
  agent: "Agents",
  phone: "Phones",
  connection: "Connections",
  monitors: "Monitors",
  incidents: "Incidents",
  coordinator: "Coordinator",
  asks: "Asks",
};

export const SOURCE_KIND: Record<string, SourceKind> = {
  mac: "machine",
  server: "machine",
  phone: "machine",
  agent: "service",
  monitors: "service",
  incidents: "service",
  connection: "connection",
  coordinator: "coordinator",
  asks: "asks",
};

/** How many sensors a list answer holds. */
export const wireCount = (wire: WireSensors): number => (wire.groups ?? []).reduce((n, g) => n + (g.sensors?.length ?? 0), 0);

export function fromApi(wire: WireSensors, now: number): Source[] {
  const out: Source[] = [];
  for (const grp of wire.groups ?? []) {
    const sensors: Sensor[] = [];
    for (const w of grp.sensors ?? []) {
      if (!w.sensor_id) continue;
      const state = sensorState(w.state);
      const lastSeen = ms(w.last_seen_at);
      const interval = w.interval_seconds || null;
      sensors.push({
        id: w.sensor_id,
        name: w.name || w.sensor_id,
        source_id: grp.source || w.source || "other",
        kind: w.kind || "sensor",
        state,
        health: healthOf({ raw: w.health, state, last_seen: lastSeen, interval_secs: interval }, now),
        raw: w.health,
        interval_secs: interval,
        rate_per_min: w.rate_per_minute ?? (interval ? 60 / interval : 0),
        last_seen: lastSeen,
        value: null,
        unit: w.unit && w.unit !== "count" ? w.unit : "",
        readings: [],
        owner: w.owner,
        producer: [w.producer_kind, w.producer_ref].filter(Boolean).join(" · ") || undefined,
        schema_version: w.schema_version,
        builtin: !!w.builtin,
        revocable: state !== "revoked" && !w.builtin,
      });
    }
    if (!sensors.length) continue;
    const id = grp.source || "other";
    out.push(source(id, SOURCE_NAME[id] ?? id, SOURCE_KIND[id] ?? "service", sensors.length === 1 ? "1 sensor" : `${sensors.length} sensors`, sensors));
  }
  return out;
}

/** A reading as the readings route and the signal stream carry it: producer time, server time, a number or a state. */
export type WireReading = { sensor_id?: string; produced_at?: string; received_at?: string; value?: number | null; state?: string };

const FAILED = /^(fail|failed|down|error|critical)/i;

export function readingOf(r: WireReading): Reading | null {
  const at = ms(r.produced_at) ?? ms(r.received_at) ?? 0;
  if (at <= 0) return null;
  return { at, server_at: ms(r.received_at), value: r.value ?? null, ok: r.state ? !FAILED.test(r.state) : null };
}

/** Readings as GET /v1/atlas/sensors/{id}/readings answers them, oldest first. */
export function readingsOf(rows: readonly WireReading[] | undefined): Reading[] {
  return (rows ?? [])
    .map(readingOf)
    .filter((r): r is Reading => r !== null)
    .sort((a, b) => a.at - b.at);
}

/** The signal stream's data (GET /v1/atlas/signals/events): a `reading` or a `summary` (a digest of what was missed). */
export type WireSignalEvent = { sse_id?: string; type?: string; reading?: WireReading; missed?: number | string };

/** A live reading for one sensor, or null when the event is not one. */
export function streamReading(e: WireSignalEvent): { sensor_id: string; reading: Reading; state: string | undefined } | null {
  if (e.type !== "reading" || !e.reading?.sensor_id) return null;
  const reading = readingOf(e.reading);
  return reading ? { sensor_id: e.reading.sensor_id, reading, state: e.reading.state } : null;
}

export const READINGS_KEPT = 120;

/** The newest readings per sensor, with one more appended: oldest first, no repeats by time, at most READINGS_KEPT. */
export function pushReading(by: Readonly<Record<string, readonly Reading[]>>, id: string, r: Reading, max = READINGS_KEPT): Record<string, readonly Reading[]> {
  const cur = by[id] ?? [];
  if (cur.some((x) => x.at === r.at && x.value === r.value)) return by as Record<string, readonly Reading[]>;
  const next = [...cur, r].sort((a, b) => a.at - b.at).slice(-max);
  return { ...by, [id]: next };
}

/**
 * The service's list laid over the readings the page holds: the newest reading is the value, and it
 * moves last_seen forward and the health with it (a stale sensor that speaks is no longer silent).
 */
export function withReadings(sources: readonly Source[], by: Readonly<Record<string, readonly Reading[]>>, live: Readonly<Record<string, string>>, now: number): Source[] {
  return sources.map((src) => {
    const sensors = src.sensors.map((s) => {
      const readings = by[s.id] ?? s.readings;
      const last = readings.length ? readings[readings.length - 1] : null;
      const last_seen = last && (s.last_seen == null || last.at > s.last_seen) ? last.at : s.last_seen;
      const raw = live[s.id] ?? s.raw;
      return {
        ...s,
        readings,
        value: last?.value ?? s.value,
        last_seen,
        raw,
        health: healthOf({ raw, state: s.state, last_seen, interval_secs: s.interval_secs }, now),
      };
    });
    return { ...src, sensors, health: rollup(sensors.map((s) => s.health)) };
  });
}

/** The health word a live reading's state implies, for a sensor that says nothing else. */
export function liveHealthWord(state: string | undefined): string | undefined {
  if (!state) return undefined;
  return FAILED.test(state) ? "down" : /^(warn|degraded|slow)/i.test(state) ? "warning" : "ok";
}

/** The one-line count of a health among planets and sensors, for the header. */
export function healthCounts(sources: readonly Source[]): Record<Health, number> {
  const out: Record<Health, number> = { failing: 0, degraded: 0, fine: 0, silent: 0 };
  for (const s of sources) for (const x of s.sensors) out[x.health] += 1;
  return out;
}

// ---------------------------------------------------------------- the coordinator (sessions)

/** A coordinator job as GET /v1/coordinator/jobs answers it (proto iohr/coordinator/v1, Job). */
export type Job = {
  job_id: string;
  thread_id?: string;
  user?: string;
  kind?: string;
  brief?: string;
  state?: string;
  stage?: string;
  await_kind?: string;
  hold_reason?: string;
  created_by?: string;
  created_at?: string;
  updated_at?: string;
};
/** A thread (proto Thread): `state` is its handoff text. */
export type Thread = { thread_id: string; title?: string; purpose?: string; created_by?: string; status?: string; state?: string; created_at?: string; updated_at?: string };
/** A queued ask (proto QueuedAsk). */
export type QueuedAsk = { id: string; source?: string; title?: string; state?: string; state_reason?: string; job_ids?: string[]; created_by?: string; created_at?: string; updated_at?: string };

/** The asks state words in five buckets; an unknown word is queued, never hidden. */
export function askBucket(state: string | undefined): "queued" | "building" | "stuck" | "waiting" | "closed" {
  const s = (state ?? "").toLowerCase();
  if (/^(verified|done|dropped|closed|withdrawn)$/.test(s)) return "closed";
  if (/stuck|conflict|fail|held|paused|blocked|reopen/.test(s)) return "stuck";
  if (/^(in-preview|preview|designed)$/.test(s) || s.startsWith("waiting")) return "waiting";
  if (/^(building|pr-open|in-progress|running|merged|stacked)$/.test(s)) return "building";
  return "queued";
}

export type Me = { subject?: string; user_id?: string; email?: string };

/** The live sessions: running jobs, open threads with no job, and open asks. Someone else's is redacted at draw time (`redact`). */
export function sessionsOf(jobs: readonly Job[], threads: readonly Thread[], asks: readonly QueuedAsk[], me: Me): SessionPreview[] {
  const out: SessionPreview[] = [];
  const withJob = new Set<string>();
  for (const j of jobs) {
    if (!isLiveState(j.state ?? "")) continue;
    if (j.thread_id) withJob.add(j.thread_id);
    out.push({
      id: j.job_id,
      thread_id: j.thread_id || null,
      title: (j.brief ?? "").split("\n")[0].slice(0, 120) || j.kind || "A job",
      state: j.state ?? "",
      last_step: j.hold_reason || j.stage || "",
      starter: j.created_by || j.user || "",
      at: ms(j.updated_at ?? j.created_at),
      mine: isMine([j.created_by, j.user], me),
      kind: "session",
    });
  }
  for (const t of threads) {
    if ((t.status ?? "open") !== "open" || withJob.has(t.thread_id)) continue;
    out.push({
      id: t.thread_id,
      thread_id: t.thread_id,
      title: t.title || t.purpose || "A thread",
      state: "open",
      last_step: (t.state ?? "").split("\n").find((l) => l.trim()) ?? "",
      starter: t.created_by ?? "",
      at: ms(t.updated_at ?? t.created_at),
      mine: isMine([t.created_by], me),
      kind: "session",
    });
  }
  for (const a of asks) {
    if (askBucket(a.state) === "closed") continue;
    out.push({
      id: a.id,
      thread_id: null,
      title: a.title || a.id,
      state: a.state ?? "",
      last_step: a.state_reason ?? "",
      starter: a.created_by ?? "",
      at: ms(a.updated_at ?? a.created_at),
      mine: isMine([a.created_by], me),
      kind: "ask",
    });
  }
  return out.sort((a, b) => (b.at ?? 0) - (a.at ?? 0));
}
