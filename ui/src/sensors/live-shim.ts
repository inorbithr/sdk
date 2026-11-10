// A stand-in for the live agent's `apiSend` and `apiStream` (board, 2026-11-10), with their signatures, so the
// sensors page is built against the shape that will land. Delete this file and import from `lib/api.ts` when the
// live PR merges. Writes and streams go through the same local proxy as reads: `/api/proxy/v1/...` with the
// per-serve token in a header.
import { type Read, proxyPath } from "../lib/api.ts";
import { startToken } from "../lib/panels.ts";

export type LiveState = "connecting" | "live" | "reconnecting" | "offline";

type SendOpts = { query?: Record<string, string>; idempotencyKey?: string; signal?: AbortSignal };

/** One write through the local server. Never throws: the answer is a `Read`. */
export async function apiSend<T = unknown>(
  method: "POST" | "PATCH" | "PUT" | "DELETE",
  path: string,
  body?: unknown,
  opts: SendOpts = {},
): Promise<Read<T>> {
  const url = proxyPath(path, opts.query ?? {});
  const token = startToken();
  if (url === null) return { state: "error", status: 400, reason: "That is not a path the page may write." };
  if (token === null)
    return { state: "error", status: 0, reason: "This page has no start token: open it with `iohr ui open`." };
  const headers: Record<string, string> = { "x-iohr-token": token };
  if (body !== undefined) headers["content-type"] = "application/json";
  if (opts.idempotencyKey) headers["idempotency-key"] = opts.idempotencyKey;
  let res: Response;
  try {
    res = await fetch(url, {
      method,
      headers,
      body: body === undefined ? undefined : JSON.stringify(body),
      cache: "no-store",
      signal: opts.signal,
    });
  } catch {
    return { state: "error", status: 0, reason: "The local `iohr ui serve` did not answer." };
  }
  if (res.status === 401) return { state: "signed-out" };
  const text = await res.text().catch(() => "");
  let json: unknown = null;
  try {
    json = text === "" ? null : JSON.parse(text);
  } catch {
    json = null;
  }
  if (!res.ok) {
    const e = (json as { error?: unknown } | null)?.error;
    const m = typeof e === "string" ? e : (e as { message?: unknown } | undefined)?.message;
    return { state: "error", status: res.status, reason: typeof m === "string" && m !== "" ? m : `The API answered HTTP ${res.status}.` };
  }
  return { state: "ok", data: json as T };
}

type StreamOpts<T> = {
  query?: Record<string, string>;
  method?: "GET" | "POST";
  body?: unknown;
  signal?: AbortSignal;
  onState?: (s: LiveState, why?: string) => void;
  _t?: T;
};

/** Parses server-sent frames out of text chunks; keeps the unfinished tail. */
export function parseFrames(buf: string): { frames: { event: string; id: string | null; data: string }[]; rest: string } {
  const parts = buf.replace(/\r\n/g, "\n").split("\n\n");
  const rest = parts.pop() ?? "";
  const frames: { event: string; id: string | null; data: string }[] = [];
  for (const p of parts) {
    let event = "message";
    let id: string | null = null;
    const data: string[] = [];
    for (const line of p.split("\n")) {
      if (line.startsWith(":")) continue;
      const i = line.indexOf(":");
      const k = i < 0 ? line : line.slice(0, i);
      const v = i < 0 ? "" : line.slice(i + 1).replace(/^ /, "");
      if (k === "event") event = v;
      else if (k === "id") id = v;
      else if (k === "data") data.push(v);
    }
    if (data.length > 0) frames.push({ event, id, data: data.join("\n") });
  }
  return { frames, rest };
}

/** Follows a server-sent stream; reconnects with backoff and Last-Event-ID. Returns the stop function. */
export function apiStream<T = unknown>(
  path: string,
  onEvent: (data: T, meta: { event: string; id: string | null }) => void,
  opts: StreamOpts<T> = {},
): () => void {
  const url = proxyPath(path, opts.query ?? {});
  const token = startToken();
  const ctl = new AbortController();
  opts.signal?.addEventListener("abort", () => ctl.abort());
  let stopped = false;
  let lastId: string | null = null;
  const state = (s: LiveState, why?: string) => {
    if (!stopped) opts.onState?.(s, why);
  };
  if (url === null || token === null) {
    state("offline", "This page may not follow that stream.");
    return () => {};
  }
  void (async () => {
    let attempt = 0;
    state("connecting");
    while (!stopped) {
      try {
        const headers: Record<string, string> = { "x-iohr-token": token, accept: "text/event-stream" };
        if (lastId !== null) headers["last-event-id"] = lastId;
        const res = await fetch(url, {
          method: opts.method ?? "GET",
          headers,
          body: opts.body === undefined ? undefined : JSON.stringify(opts.body),
          cache: "no-store",
          signal: ctl.signal,
        });
        if (!res.ok || res.body === null) throw new Error(`HTTP ${res.status}`);
        attempt = 0;
        state("live");
        const reader = res.body.getReader();
        const dec = new TextDecoder();
        let buf = "";
        for (;;) {
          const { value, done } = await reader.read();
          if (done) break;
          buf += dec.decode(value, { stream: true });
          const { frames, rest } = parseFrames(buf);
          buf = rest;
          for (const f of frames) {
            if (f.id !== null) lastId = f.id;
            try {
              onEvent(JSON.parse(f.data) as T, { event: f.event, id: f.id });
            } catch {
              // a frame that is not JSON is skipped
            }
          }
        }
      } catch (e) {
        if (stopped) return;
        state("reconnecting", e instanceof Error ? e.message : "stream failed");
      }
      if (stopped) return;
      attempt += 1;
      if (attempt > 6) state("offline", "The stream did not come back.");
      await new Promise((r) => setTimeout(r, Math.min(15000, 500 * 2 ** Math.min(attempt, 5))));
    }
  })();
  return () => {
    stopped = true;
    ctl.abort();
  };
}
