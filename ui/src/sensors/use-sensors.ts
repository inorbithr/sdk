// The sensors page's data (RFC 0217): the account's sensors from the sensors service, kept live.
//
//   list     GET /v1/atlas/sensors, whole (pages followed), again on every `summary` of the signal stream, and
//            every SAFETY_POLL_MS only while the stream is not live;
//   readings GET /v1/atlas/sensors/{id}/readings, when a planet or a sensor is opened (its sparkline);
//   stream   SSE /v1/atlas/signals/events: each `reading` moves a sensor's value, last seen and health at once;
//   sessions GET /v1/coordinator/{jobs,threads,asks}, again (debounced) on each event of /v1/coordinator/picture/events;
//   clock    a tick every TICK_MS so "2 minutes ago" and staleness move between looks.
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { type Read, apiGet } from "../lib/api.ts";
import {
  type SessionPreview,
  type SignalEvent,
  type Snapshot,
  diffEvents,
  mergeEvents,
} from "../lib/sensors-core.ts";
import {
  type Job,
  type QueuedAsk,
  type Thread,
  type WireReading,
  type WireSensors,
  type WireSignalEvent,
  fromApi,
  pushReading,
  readingsOf,
  sessionsOf,
  streamReading,
  withReadings,
} from "../lib/sensors-wire.ts";
import type { Reading } from "../lib/sensors-core.ts";
import { type LiveState, apiStream } from "./live-shim.ts";

export const TICK_MS = 10_000;
export const SAFETY_POLL_MS = 60_000;
export const SESSIONS_DEBOUNCE_MS = 1_500;
const PAGES_MAX = 10;

export type SensorsData = {
  snap: Snapshot | null;
  events: SignalEvent[];
  fresh: SignalEvent[];
  sessions: SessionPreview[];
  /** How the signal stream stands: connecting, live, reconnecting, offline. */
  conn: LiveState;
  loading: boolean;
  signedOut: boolean;
  /** Reads the readings of one sensor (its sparkline). */
  open: (sensorIds: readonly string[]) => void;
  reload: () => void;
  /** The raw list, for the consent sheet (it needs owner and state, which the page keeps). */
  at: number | null;
};

async function listAll(account: string): Promise<Read<WireSensors>> {
  let token = "";
  const merged: WireSensors = { groups: [] };
  for (let i = 0; i < PAGES_MAX; i += 1) {
    const q: Record<string, string> = { account_id: account, page_size: "200" };
    if (token) q.page_token = token;
    const r = await apiGet<WireSensors>("/v1/atlas/sensors", q);
    if (r.state !== "ok") return r;
    merged.groups = [...(merged.groups ?? []), ...(r.data.groups ?? [])];
    token = r.data.next_page_token ?? "";
    if (!token) break;
  }
  return { state: "ok", data: merged };
}

export function useSensorsData(account: string, subject: string): SensorsData {
  const [list, setList] = useState<WireSensors | null>(null);
  const [by, setBy] = useState<Record<string, readonly Reading[]>>({});
  const [liveRaw, setLiveRaw] = useState<Record<string, string>>({});
  const [now, setNow] = useState(() => Date.now());
  const [conn, setConn] = useState<LiveState>("connecting");
  const [problems, setProblems] = useState<string[]>([]);
  const [signedOut, setSignedOut] = useState(false);
  const [loading, setLoading] = useState(true);
  const [at, setAt] = useState<number | null>(null);
  const [events, setEvents] = useState<SignalEvent[]>([]);
  const [fresh, setFresh] = useState<SignalEvent[]>([]);
  const [jobs, setJobs] = useState<Job[]>([]);
  const [threads, setThreads] = useState<Thread[]>([]);
  const [asks, setAsks] = useState<QueuedAsk[]>([]);
  const asked = useRef(new Set<string>());
  const prev = useRef<Snapshot | null>(null);
  const connRef = useRef<LiveState>("connecting");

  const load = useCallback(async () => {
    if (!account) {
      setLoading(false);
      return;
    }
    setLoading(true);
    const r = await listAll(account);
    if (r.state === "signed-out") setSignedOut(true);
    else if (r.state === "error") setProblems([`Sensors: ${r.reason}`]);
    else {
      setSignedOut(false);
      setProblems([]);
      setList(r.data);
      setAt(Date.now());
    }
    setLoading(false);
  }, [account]);

  // the list, and the clock
  useEffect(() => {
    void load();
    const poll = window.setInterval(() => {
      if (!document.hidden && connRef.current !== "live") void load();
    }, SAFETY_POLL_MS);
    const tick = window.setInterval(() => !document.hidden && setNow(Date.now()), TICK_MS);
    return () => {
      window.clearInterval(poll);
      window.clearInterval(tick);
    };
  }, [load]);

  // the signal stream
  useEffect(() => {
    if (!account) return;
    let timer = 0;
    const stop = apiStream<WireSignalEvent>(
      "/v1/atlas/signals/events",
      (e) => {
        const r = streamReading(e);
        if (r) {
          setBy((b) => pushReading(b, r.sensor_id, r.reading));
          if (r.state) setLiveRaw((m) => ({ ...m, [r.sensor_id]: r.state ?? "" }));
        } else if (e.type === "summary") {
          window.clearTimeout(timer);
          timer = window.setTimeout(() => void load(), 500);
        }
      },
      {
        query: { account_id: account },
        onState: (s) => {
          connRef.current = s;
          setConn(s);
          if (s === "live") void load();
        },
      },
    );
    return () => {
      window.clearTimeout(timer);
      stop();
    };
  }, [account, load]);

  // sessions, on the shared picture stream
  const loadSessions = useCallback(async () => {
    const [j, t, a] = await Promise.all([
      apiGet<{ jobs?: Job[] }>("/v1/coordinator/jobs", {}),
      apiGet<{ threads?: Thread[] }>("/v1/coordinator/threads", {}),
      apiGet<{ asks?: QueuedAsk[] }>("/v1/coordinator/asks", {}),
    ]);
    if (j.state === "ok") setJobs(j.data.jobs ?? []);
    if (t.state === "ok") setThreads(t.data.threads ?? []);
    if (a.state === "ok") setAsks(a.data.asks ?? []);
  }, []);
  useEffect(() => {
    if (!account) return;
    void loadSessions();
    let timer = 0;
    const stop = apiStream("/v1/coordinator/picture/events", () => {
      window.clearTimeout(timer);
      timer = window.setTimeout(() => void loadSessions(), SESSIONS_DEBOUNCE_MS);
    });
    return () => {
      window.clearTimeout(timer);
      stop();
    };
  }, [account, loadSessions]);

  const open = useCallback((ids: readonly string[]) => {
    for (const id of ids) {
      if (asked.current.has(id)) continue;
      asked.current.add(id);
      void apiGet<{ readings?: WireReading[] }>(`/v1/atlas/sensors/${encodeURIComponent(id)}/readings`, {}).then((r) => {
        if (r.state !== "ok") {
          asked.current.delete(id);
          return;
        }
        const rows = readingsOf(r.data.readings);
        setBy((b) => rows.reduce((acc, x) => pushReading(acc, id, x), b));
      });
    }
  }, []);

  const snap = useMemo<Snapshot | null>(() => {
    if (!list) return null;
    const sources = withReadings(fromApi(list, now), by, liveRaw, now);
    return { origin: "api", label: "From the sensors service", sources, at: now, problems };
  }, [list, by, liveRaw, now, problems]);

  useEffect(() => {
    if (!snap) return;
    const p = prev.current;
    prev.current = snap;
    const f = diffEvents(p, snap);
    if (f.length) {
      setEvents((e) => mergeEvents(e, f));
      setFresh(f);
    }
  }, [snap]);

  const sessions = useMemo(() => sessionsOf(jobs, threads, asks, { subject }), [jobs, threads, asks, subject]);

  return { snap, events, fresh, sessions, conn, loading, signedOut, open, reload: () => void load(), at };
}
