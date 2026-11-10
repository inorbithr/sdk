// What the page knows of who it acts as, and how it reads the API (RFC 0217, round 4). The page never holds a
// credential: `iohr ui serve` signs in as `iohr login` does and calls the API for the page, so every read goes to
// `/api/proxy/v1/...` on this origin with the per-serve token in a header, and the answer is only what the API said.
import { startToken } from "./panels.ts";

/** How the sign-in the page started stands. */
export type LoginPhase = "idle" | "waiting" | "failed";

/** `GET /api/session`: no credential, ever. */
export type SessionState =
  | {
      state: "signed-in";
      profile: string;
      kind: string;
      account: string;
      subject: string;
      scopes: string[];
      login: LoginPhase;
    }
  | { state: "signed-out"; login: LoginPhase; reason: string | null }
  | { state: "unreachable"; reason: string };

/** A read's answer: the JSON the API sent, or why not. Never throws. */
export type Read<T> =
  | { state: "ok"; data: T }
  | { state: "signed-out" }
  | { state: "error"; status: number; reason: string };

const phase = (v: unknown): LoginPhase => (v === "waiting" || v === "failed" ? v : "idle");
const text = (v: unknown): string => (typeof v === "string" ? v : "");

const own = (token: string | null): HeadersInit => (token === null ? {} : { "x-iohr-token": token });

/** Who this machine is signed in as, and how a sign-in started from the page is going. */
export async function readSession(token = startToken(), fetcher: typeof fetch = fetch): Promise<SessionState> {
  if (token === null)
    return { state: "unreachable", reason: "This page has no start token: open it with `iohr ui open`." };
  let res: Response;
  try {
    res = await fetcher("/api/session", { headers: own(token), cache: "no-store" });
  } catch {
    return { state: "unreachable", reason: "The local `iohr ui serve` did not answer." };
  }
  if (res.status === 401)
    return {
      state: "unreachable",
      reason: "This page is not opened from `iohr ui open`: its launch link has not been used.",
    };
  if (!res.ok) return { state: "unreachable", reason: `The session answer failed (HTTP ${res.status}).` };
  let body: unknown;
  try {
    body = await res.json();
  } catch {
    return { state: "unreachable", reason: "The session answer was not JSON." };
  }
  const v = typeof body === "object" && body !== null ? (body as Record<string, unknown>) : {};
  const login = phase(v.login);
  if (v.signed_in === true) {
    const scopes = Array.isArray(v.scopes) ? v.scopes.filter((s): s is string => typeof s === "string") : [];
    return {
      state: "signed-in",
      profile: text(v.profile),
      kind: text(v.kind),
      account: text(v.account),
      subject: text(v.subject),
      scopes,
      login,
    };
  }
  return { state: "signed-out", login, reason: typeof v.reason === "string" ? v.reason : null };
}

const send = async (path: string, token: string | null, fetcher: typeof fetch): Promise<boolean> => {
  if (token === null) return false;
  try {
    return (await fetcher(path, { method: "POST", headers: own(token), cache: "no-store" })).ok;
  } catch {
    return false;
  }
};

/** Starts the sign-in: the person's browser opens, as with `iohr login --web`. True once the server took it. */
export const signIn = (token = startToken(), fetcher: typeof fetch = fetch): Promise<boolean> =>
  send("/api/session/login", token, fetcher);

/** Signs out as `iohr logout` does. True once it did. */
export const signOut = (token = startToken(), fetcher: typeof fetch = fetch): Promise<boolean> =>
  send("/api/session/logout", token, fetcher);

/** The proxy path of an API path (`/v1/...`) and its query; null when it is not a plain read the server would take. */
export function proxyPath(path: string, query: Record<string, string> = {}): string | null {
  if (!path.startsWith("/v1/") || path.includes("..") || path.includes("//") || /[?#\\]/.test(path)) return null;
  const pairs = Object.entries(query).map(([k, v]) => `${encodeURIComponent(k)}=${encodeURIComponent(v)}`);
  return `/api/proxy${path}${pairs.length === 0 ? "" : `?${pairs.join("&")}`}`;
}

/** One read of the API through the local server: GET only, and the API's own status. */
export async function apiGet<T = unknown>(
  path: string,
  query: Record<string, string> = {},
  token = startToken(),
  fetcher: typeof fetch = fetch,
): Promise<Read<T>> {
  const url = proxyPath(path, query);
  if (url === null) return { state: "error", status: 400, reason: "That is not a path the page may read." };
  if (token === null)
    return { state: "error", status: 0, reason: "This page has no start token: open it with `iohr ui open`." };
  let res: Response;
  try {
    res = await fetcher(url, { headers: own(token), cache: "no-store" });
  } catch {
    return { state: "error", status: 0, reason: "The local `iohr ui serve` did not answer." };
  }
  if (res.status === 401) return { state: "signed-out" };
  if (!res.ok) {
    let reason = `The API answered HTTP ${res.status}.`;
    try {
      const e = (await res.json()) as { error?: unknown };
      const m = typeof e.error === "string" ? e.error : (e.error as { message?: unknown } | undefined)?.message;
      if (typeof m === "string" && m !== "") reason = m;
    } catch {
      // the status line says it
    }
    return { state: "error", status: res.status, reason };
  }
  try {
    return { state: "ok", data: (await res.json()) as T };
  } catch {
    return { state: "error", status: res.status, reason: "The API answer was not JSON." };
  }
}

/** The methods a write may use. */
export type WriteMethod = "POST" | "PATCH" | "PUT" | "DELETE";

/**
 * One write (or a read that needs a body, such as a POST that resolves) of the API through the local server: JSON in,
 * the API's own status out. Never throws. The server takes it only with the token header and this page's origin.
 */
export async function apiSend<T = unknown>(
  method: WriteMethod,
  path: string,
  body?: unknown,
  opts: { query?: Record<string, string>; signal?: AbortSignal } = {},
  token = startToken(),
  fetcher: typeof fetch = fetch,
): Promise<Read<T>> {
  const url = proxyPath(path, opts.query ?? {});
  if (url === null) return { state: "error", status: 400, reason: "That is not a path the page may call." };
  if (token === null)
    return { state: "error", status: 0, reason: "This page has no start token: open it with `iohr ui open`." };
  const headers: Record<string, string> = { "x-iohr-token": token };
  const init: RequestInit = { method, headers, cache: "no-store", signal: opts.signal ?? null };
  if (body !== undefined) {
    headers["content-type"] = "application/json";
    init.body = JSON.stringify(body);
  }
  let res: Response;
  try {
    res = await fetcher(url, init);
  } catch {
    return { state: "error", status: 0, reason: "The local `iohr ui serve` did not answer." };
  }
  if (res.status === 401) return { state: "signed-out" };
  if (!res.ok) return { state: "error", status: res.status, reason: await failure(res) };
  if (res.status === 204) return { state: "ok", data: undefined as T };
  try {
    return { state: "ok", data: (await res.json()) as T };
  } catch {
    return { state: "error", status: res.status, reason: "The API answer was not JSON." };
  }
}

async function failure(res: Response): Promise<string> {
  try {
    const e = (await res.json()) as { error?: unknown };
    const m = typeof e.error === "string" ? e.error : (e.error as { message?: unknown } | undefined)?.message;
    if (typeof m === "string" && m !== "") return m;
  } catch {
    // the status line says it
  }
  return `The API answered HTTP ${res.status}.`;
}

/** Where a live stream stands: opening, flowing, retrying after a break, or given up. */
export type LiveState = "connecting" | "live" | "reconnecting" | "offline";

/** What an event carries besides its data. */
export type StreamMeta = { event: string; id: string | null };

export type StreamOptions<T> = {
  query?: Record<string, string>;
  /** The query key that resumes after the last event id (`after` for sensors): set on a reconnect. */
  resumeParam?: string;
  onState?: (state: LiveState, why?: string) => void;
  signal?: AbortSignal;
  /** Test seams. */
  token?: string | null;
  fetcher?: typeof fetch;
  wait?: (ms: number) => Promise<void>;
  silenceMs?: number;
  /** The live socket (`/api/live`); defaults to this page's own when the browser has a `WebSocket`. `false` is SSE only. */
  socket?: SocketConfig | false;
  _t?: T;
};

/** A WebSocket as the hub uses it, so a test can stand in for the browser's. */
export type SocketLike = {
  send(data: string): void;
  close(): void;
  onopen: (() => void) | null;
  onmessage: ((e: { data: unknown }) => void) | null;
  onclose: (() => void) | null;
  onerror: (() => void) | null;
};
export type SocketConfig = { url: string; ctor: new (url: string) => SocketLike };

type Sub = { onEvent: (data: never, meta: StreamMeta) => void; onState?: (s: LiveState, why?: string) => void };
type Shared = { subs: Set<Sub>; stop: () => void; state: LiveState; why?: string };
const streams = new Map<string, Shared>();

const BACKOFF = [500, 1000, 2000, 4000, 8000, 15000, 30000];
/** The server sends a keepalive every 15 s; this long with no bytes at all and the link is dead. */
const SILENCE_MS = 45_000;

/** How long the page reads server-sent events instead after the socket could not be opened. */
const SOCKET_COOLDOWN_MS = 60_000;

type WireCall = { data(d: unknown): void; end(): void; fail(status: number, message: string): void };
type Wire = { ws: SocketLike; calls: Map<string, WireCall>; next: number; ready: Promise<void> };
let wire: Wire | null = null;
let socketDownUntil = 0;
/** Paths the server has no socket call for: read as server-sent events from then on. */
const unmapped = new Set<string>();

/** Forgets the socket and what it learned (tests). */
export function resetLive(): void {
  wire?.ws.close();
  wire = null;
  socketDownUntil = 0;
  unmapped.clear();
}

function pageSocket(): SocketConfig | null {
  const loc = (globalThis as { location?: { protocol: string; host: string } }).location;
  const ctor = (globalThis as { WebSocket?: SocketConfig["ctor"] }).WebSocket;
  if (loc === undefined || ctor === undefined) return null;
  return { url: `${loc.protocol === "https:" ? "wss:" : "ws:"}//${loc.host}/api/live`, ctor };
}

/**
 * The one WebSocket the page's streams share. The browser cannot set a header on a handshake, so the per-serve token
 * is the first frame (the server reads nothing before it). It resolves when the server says `ready`.
 */
function openWire(cfg: SocketConfig, token: string): Wire {
  const ws = new cfg.ctor(cfg.url);
  const calls = new Map<string, WireCall>();
  const w: Wire = { ws, calls, next: 1, ready: Promise.resolve() };
  let opened = false;
  const broken = (message: string) => {
    if (wire === w) wire = null;
    if (!opened) socketDownUntil = Date.now() + SOCKET_COOLDOWN_MS;
    const held = [...calls.values()];
    calls.clear();
    for (const c of held) c.fail(0, message);
  };
  w.ready = new Promise<void>((resolve, reject) => {
    ws.onopen = () => ws.send(JSON.stringify({ type: "auth", token }));
    ws.onerror = () => {
      reject(new Error("The live socket could not be opened."));
      ws.close();
    };
    ws.onclose = () => {
      reject(new Error("The live socket closed."));
      broken("The live socket closed.");
    };
    ws.onmessage = (e) => {
      let f: { type?: string; id?: string; data?: unknown; status?: string; message?: string };
      try {
        f = JSON.parse(String(e.data));
      } catch {
        return;
      }
      if (f.type === "ready") {
        opened = true;
        resolve();
      } else if (f.id === undefined) {
        if (f.type === "error") {
          reject(new Error(f.message ?? "The live socket failed."));
          broken(f.message ?? "The live socket failed.");
          ws.close();
        }
      } else {
        const c = calls.get(f.id);
        if (c === undefined) return;
        if (f.type === "data") c.data(f.data);
        else if (f.type === "end") {
          calls.delete(f.id);
          c.end();
        } else if (f.type === "error") {
          calls.delete(f.id);
          c.fail(Number.parseInt(f.status ?? "0", 10), f.message ?? "The stream failed.");
        }
      }
    };
  });
  w.ready.catch(() => {});
  return w;
}

type SocketOutcome = { kind: "fallback" } | { kind: "ended" } | { kind: "fatal"; why: string };

/**
 * One round of a stream over the shared socket: `fallback` when it cannot be used (no socket, the server has no socket
 * call for this path, or it is cooling down after a failure to open), `ended` when the API ended it, `fatal` for a
 * refusal that will not change; any other break throws, and the caller reconnects with backoff.
 */
async function overSocket(
  cfg: SocketConfig | null,
  token: string,
  url: string,
  apiPath: string,
  signal: AbortSignal,
  onLive: () => void,
  onData: (d: unknown) => void,
): Promise<SocketOutcome> {
  if (cfg === null || unmapped.has(url) || Date.now() < socketDownUntil) return { kind: "fallback" };
  let w = wire;
  if (w === null) {
    w = openWire(cfg, token);
    wire = w;
  }
  try {
    await w.ready;
  } catch {
    return { kind: "fallback" };
  }
  if (signal.aborted) return { kind: "ended" };
  const live = w;
  const id = String(live.next++);
  return new Promise<SocketOutcome>((resolve, reject) => {
    const stop = () => {
      if (live.calls.delete(id)) live.ws.send(JSON.stringify({ type: "cancel", id }));
      resolve({ kind: "ended" });
    };
    signal.addEventListener("abort", stop, { once: true });
    const done = () => signal.removeEventListener("abort", stop);
    live.calls.set(id, {
      data: onData,
      end: () => {
        done();
        resolve({ kind: "ended" });
      },
      fail: (status, message) => {
        done();
        if (message === "unmapped") {
          unmapped.add(url);
          resolve({ kind: "fallback" });
        } else if (status >= 400 && status < 500 && status !== 429) resolve({ kind: "fatal", why: message });
        else reject(new Error(message));
      },
    });
    live.ws.send(JSON.stringify({ type: "call", id, path: apiPath }));
    onLive();
  });
}

/** The parsed events of an SSE byte stream: `id`, `event`, `data`; comments return `null`-data frames for liveness. */
export function parseSse(buffer: string): { frames: { id: string | null; event: string; data: string }[]; rest: string } {
  const frames: { id: string | null; event: string; data: string }[] = [];
  const parts = buffer.split(/\r?\n\r?\n/);
  const rest = parts.pop() ?? "";
  for (const part of parts) {
    let id: string | null = null;
    let event = "message";
    const data: string[] = [];
    for (const line of part.split(/\r?\n/)) {
      if (line.startsWith(":")) continue;
      const i = line.indexOf(":");
      const k = i < 0 ? line : line.slice(0, i);
      const v = i < 0 ? "" : line.slice(i + 1).replace(/^ /, "");
      if (k === "id") id = v;
      else if (k === "event") event = v;
      else if (k === "data") data.push(v);
    }
    if (data.length > 0) frames.push({ id, event, data: data.join("\n") });
  }
  return { frames, rest };
}

/**
 * A live stream of the API through the local server: server-sent events over fetch (the token travels in a header, which
 * EventSource cannot send). Reconnects with backoff, resuming after the last id through `resumeParam`; identical
 * (path, query) streams on one page share one connection. Returns the function that stops this subscription.
 */
export function apiStream<T = unknown>(
  path: string,
  onEvent: (data: T, meta: StreamMeta) => void,
  opts: StreamOptions<T> = {},
): () => void {
  const url = proxyPath(path, opts.query ?? {});
  const token = opts.token === undefined ? startToken() : opts.token;
  const sub: Sub = { onEvent: onEvent as Sub["onEvent"], ...(opts.onState ? { onState: opts.onState } : {}) };
  if (url === null || token === null) {
    opts.onState?.("offline", url === null ? "That is not a path the page may stream." : "This page has no start token.");
    return () => {};
  }
  let shared = streams.get(url);
  if (shared === undefined) {
    shared = openShared(url, path, opts, token);
    streams.set(url, shared);
  }
  shared.subs.add(sub);
  opts.onState?.(shared.state, shared.why);
  const held = shared;
  let stopped = false;
  const stop = () => {
    if (stopped) return;
    stopped = true;
    held.subs.delete(sub);
    if (held.subs.size === 0) {
      held.stop();
      if (streams.get(url) === held) streams.delete(url);
    }
  };
  opts.signal?.addEventListener("abort", stop, { once: true });
  if (opts.signal?.aborted) stop();
  return stop;
}

function openShared<T>(url: string, _path: string, opts: StreamOptions<T>, token: string): Shared {
  const fetcher = opts.fetcher ?? fetch;
  const wait = opts.wait ?? ((ms: number) => new Promise<void>((r) => setTimeout(r, ms)));
  const silence = opts.silenceMs ?? SILENCE_MS;
  const socketCfg = opts.socket === false ? null : (opts.socket ?? pageSocket());
  const shared: Shared = { subs: new Set(), stop: () => {}, state: "connecting" };
  const ctl = new AbortController();
  const set = (state: LiveState, why?: string) => {
    shared.state = state;
    if (why === undefined) delete shared.why;
    else shared.why = why;
    for (const s of [...shared.subs]) s.onState?.(state, why);
  };
  shared.stop = () => ctl.abort();
  let lastId: string | null = null;
  const resumed = (): string => {
    if (lastId === null || opts.resumeParam === undefined) return url;
    return `${url}${url.includes("?") ? "&" : "?"}${encodeURIComponent(opts.resumeParam)}=${encodeURIComponent(lastId)}`;
  };
  const run = async () => {
    let attempt = 0;
    while (!ctl.signal.aborted) {
      let fatal: string | null = null;
      const link = new AbortController();
      const onStop = () => link.abort();
      ctl.signal.addEventListener("abort", onStop, { once: true });
      let timer: ReturnType<typeof setTimeout> | undefined;
      const beat = () => {
        clearTimeout(timer);
        timer = setTimeout(() => link.abort(), silence);
      };
      try {
        const sock = await overSocket(
          socketCfg,
          token,
          url,
          resumed().slice("/api/proxy".length),
          ctl.signal,
          () => set("live"),
          (data) => {
            attempt = 0;
            const id = typeof (data as { sse_id?: unknown })?.sse_id === "string" ? (data as { sse_id: string }).sse_id : null;
            if (id !== null && id !== "") lastId = id;
            for (const s of [...shared.subs]) s.onEvent(data as never, { event: "message", id });
          },
        );
        if (sock.kind === "fatal") fatal = sock.why;
        if (sock.kind === "fallback") {
          beat();
          const headers: Record<string, string> = { "x-iohr-token": token, accept: "text/event-stream" };
          if (lastId !== null) headers["last-event-id"] = lastId;
          const res = await fetcher(resumed(), { headers, cache: "no-store", signal: link.signal });
          if (res.status === 401) fatal = "signed out";
          else if (!res.ok || res.body === null) {
            if (res.status >= 400 && res.status < 500 && res.status !== 408 && res.status !== 429)
              fatal = await failure(res);
            else throw new Error(`HTTP ${res.status}`);
          } else {
            const reader = res.body.getReader();
            const decoder = new TextDecoder();
            let buf = "";
            set("live");
            for (;;) {
              const { done, value } = await reader.read();
              if (done) break;
              beat();
              attempt = 0;
              buf += decoder.decode(value, { stream: true });
              const { frames, rest } = parseSse(buf);
              buf = rest;
              for (const f of frames) {
                if (f.event === "error") {
                  let status = 0;
                  let reason = "The stream failed.";
                  try {
                    const e = (JSON.parse(f.data) as { error?: { status?: string; message?: string } }).error;
                    status = Number.parseInt(e?.status ?? "0", 10);
                    reason = e?.message ?? reason;
                  } catch {
                    // keep the default
                  }
                  if (status >= 400 && status < 500 && status !== 429) fatal = reason;
                  throw new Error(reason);
                }
                let data: unknown;
                try {
                  data = JSON.parse(f.data);
                } catch {
                  continue;
                }
                if (f.id !== null) lastId = f.id;
                for (const s of [...shared.subs]) s.onEvent(data as never, { event: f.event, id: f.id });
              }
            }
          }
        }
      } catch (e) {
        if (fatal === null && !ctl.signal.aborted) set("reconnecting", e instanceof Error ? e.message : "broken");
      } finally {
        clearTimeout(timer);
        ctl.signal.removeEventListener("abort", onStop);
        link.abort();
      }
      if (ctl.signal.aborted) return;
      if (fatal !== null) {
        set("offline", fatal);
        return;
      }
      set("reconnecting", shared.why);
      const delay = BACKOFF[Math.min(attempt, BACKOFF.length - 1)] ?? 30000;
      attempt += 1;
      await wait(delay);
    }
  };
  void run();
  return shared;
}
