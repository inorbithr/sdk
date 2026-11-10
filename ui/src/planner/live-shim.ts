// A stand-in for the live agent's client (feat/ui-live: `apiSend`, `apiStream`, `useLive` in lib/api.ts), with the
// signatures the port board confirmed. It exists only until that PR merges; then this file is deleted and the
// planner imports from lib/api.ts. It goes through the same local proxy as `apiGet` (`/api/proxy/v1/...` with the
// per-serve token) and never reaches the API itself.

import { useEffect, useRef, useState } from "react";
import type { Read } from "../lib/api.ts";
import { startToken } from "../lib/panels.ts";

export type LiveState = "connecting" | "live" | "reconnecting" | "offline";
export type StreamMeta = { event: string; id: string };

const path = (p: string, query: Record<string, string> = {}): string | null => {
  if (!p.startsWith("/v1/") || p.includes("..") || p.includes("//") || /[?#\\]/.test(p)) return null;
  const q = Object.entries(query).map(([k, v]) => `${encodeURIComponent(k)}=${encodeURIComponent(v)}`);
  return `/api/proxy${p}${q.length === 0 ? "" : `?${q.join("&")}`}`;
};

/** One write through the local server. Never throws. */
export async function apiSend<T = unknown>(
  method: "POST" | "PATCH" | "PUT" | "DELETE",
  p: string,
  body?: unknown,
  opts: { query?: Record<string, string>; idempotencyKey?: string; signal?: AbortSignal } = {},
): Promise<Read<T>> {
  const url = path(p, opts.query);
  const token = startToken();
  if (url === null) return { state: "error", status: 400, reason: "That is not a path the page may write." };
  if (token === null) return { state: "error", status: 0, reason: "This page has no start token: open it with `iohr ui open`." };
  let res: Response;
  try {
    res = await fetch(url, {
      method,
      headers: { "x-iohr-token": token, "content-type": "application/json", ...(opts.idempotencyKey ? { "idempotency-key": opts.idempotencyKey } : {}) },
      body: body === undefined ? undefined : JSON.stringify(body),
      cache: "no-store",
      ...(opts.signal ? { signal: opts.signal } : {}),
    });
  } catch {
    return { state: "error", status: 0, reason: "The local `iohr ui serve` did not answer." };
  }
  if (res.status === 401) return { state: "signed-out" };
  const text = await res.text().catch(() => "");
  let json: unknown = null;
  try {
    json = text === "" ? {} : JSON.parse(text);
  } catch {
    json = null;
  }
  if (!res.ok) {
    const e = (json as { error?: unknown } | null)?.error;
    const m = typeof e === "string" ? e : (e as { message?: unknown } | undefined)?.message;
    return { state: "error", status: res.status, reason: typeof m === "string" && m !== "" ? m : `The API answered HTTP ${res.status}.` };
  }
  return json === null ? { state: "error", status: res.status, reason: "The API answer was not JSON." } : { state: "ok", data: json as T };
}

/** Splits an SSE byte text into frames: `[event, id, data]`; the rest is the unfinished tail. Pure. */
export function sseFrames(buffer: string): { frames: { event: string; id: string; data: string }[]; rest: string } {
  const parts = buffer.replace(/\r\n/g, "\n").split("\n\n");
  const rest = parts.pop() ?? "";
  const frames = parts.flatMap((block) => {
    let event = "message";
    let id = "";
    const data: string[] = [];
    for (const l of block.split("\n")) {
      if (l.startsWith(":") || l === "") continue;
      const i = l.indexOf(":");
      const f = i < 0 ? l : l.slice(0, i);
      const v = i < 0 ? "" : l.slice(i + 1).replace(/^ /, "");
      if (f === "event") event = v;
      else if (f === "id") id = v;
      else if (f === "data") data.push(v);
    }
    return data.length === 0 ? [] : [{ event, id, data: data.join("\n") }];
  });
  return { frames, rest };
}

/** Reads a stream over the proxy; reconnects with a growing pause. Returns the stop function. */
export function apiStream<T = unknown>(
  p: string,
  onEvent: (data: T, meta: StreamMeta) => void,
  opts: { query?: Record<string, string>; method?: string; body?: unknown; signal?: AbortSignal; onState?: (s: LiveState, reason: string) => void } = {},
): () => void {
  const ctl = new AbortController();
  const stop = () => ctl.abort();
  opts.signal?.addEventListener("abort", stop, { once: true });
  const url = path(p, opts.query);
  const token = startToken();
  void (async () => {
    if (url === null || token === null) {
      opts.onState?.("offline", url === null ? "That is not a path the page may read." : "This page has no start token.");
      return;
    }
    let wait = 1000;
    let first = true;
    while (!ctl.signal.aborted) {
      opts.onState?.(first ? "connecting" : "reconnecting", "");
      try {
        const res = await fetch(url, {
          method: opts.method ?? "GET",
          headers: { "x-iohr-token": token, accept: "text/event-stream", ...(opts.body !== undefined ? { "content-type": "application/json" } : {}) },
          body: opts.body === undefined ? undefined : JSON.stringify(opts.body),
          cache: "no-store",
          signal: ctl.signal,
        });
        if (res.status === 401) {
          opts.onState?.("offline", "Signed out.");
          return;
        }
        if (!res.ok || !res.body) {
          opts.onState?.("reconnecting", `The stream answered HTTP ${res.status}.`);
        } else {
          opts.onState?.("live", "");
          wait = 1000;
          const reader = res.body.pipeThrough(new TextDecoderStream()).getReader();
          let buf = "";
          for (;;) {
            const { value, done } = await reader.read();
            if (done) break;
            const s = sseFrames(buf + value);
            buf = s.rest;
            for (const f of s.frames) {
              try {
                onEventSafe(onEvent, JSON.parse(f.data) as T, { event: f.event, id: f.id });
              } catch {
                // a frame that is not JSON is skipped
              }
            }
          }
        }
      } catch {
        if (ctl.signal.aborted) return;
      }
      first = false;
      if (ctl.signal.aborted) return;
      opts.onState?.("reconnecting", "");
      await new Promise((r) => setTimeout(r, wait));
      wait = Math.min(wait * 2, 15_000);
    }
  })();
  return stop;
}

const onEventSafe = <T,>(fn: (d: T, m: StreamMeta) => void, d: T, m: StreamMeta) => fn(d, m);

/** A stream's folded value and its state, for a component. */
export function useLive<T, V>(
  p: string | null,
  opts: { query?: Record<string, string>; reduce: (prev: V, data: T, meta: StreamMeta) => V; initial: V },
): { value: V; state: LiveState; reason: string } {
  const [value, setValue] = useState<V>(opts.initial);
  const [state, setState] = useState<LiveState>(p === null ? "offline" : "connecting");
  const [reason, setReason] = useState("");
  const reduce = useRef(opts.reduce);
  reduce.current = opts.reduce;
  const q = JSON.stringify(opts.query ?? {});
  useEffect(() => {
    if (p === null) {
      setState("offline");
      return;
    }
    setValue(opts.initial);
    return apiStream<T>(p, (d, m) => setValue((v) => reduce.current(v, d, m)), {
      query: JSON.parse(q) as Record<string, string>,
      onState: (s, r) => {
        setState(s);
        setReason(r);
      },
    });
    // biome-ignore lint/correctness/useExhaustiveDependencies: the initial value is read once per stream
  }, [p, q]);
  return { value, state, reason };
}
