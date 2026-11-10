// The page's live layer, one copy: reads over SSE through the local server's proxy (`apiStream`, `useLive`) and writes
// (`apiSend` for an ok or a reason, `apiWrite` for the answer's data). Every call carries the per-serve token and the
// page never holds a credential of its own. A stand-in for the live agent's client until it publishes one in lib/api.ts.
import { useEffect, useRef, useState } from "react";
import { proxyPath, type Read } from "./api.ts";
import { startToken } from "./panels.ts";

export type LiveState = "connecting" | "live" | "reconnecting" | "off";
export type StreamMeta = { event: string; id: string };

/** The data lines of one SSE block, joined, or null for a comment or heartbeat block. */
export function sseData(block: string): string | null {
  const lines = block
    .split("\n")
    .filter((l) => l.startsWith("data:"))
    .map((l) => l.slice(5).replace(/^ /, ""));
  return lines.length ? lines.join("\n") : null;
}

/** One write through the local server, with the API's answer as a `Read`. Never throws. */
export async function apiWrite<T = unknown>(
  method: "POST" | "PATCH" | "PUT" | "DELETE",
  p: string,
  body?: unknown,
  opts: { query?: Record<string, string>; idempotencyKey?: string; signal?: AbortSignal } = {},
): Promise<Read<T>> {
  const url = proxyPath(p, opts.query);
  const token = startToken();
  if (url === null) return { state: "error", status: 400, reason: "That is not a path the page may write." };
  if (token === null)
    return { state: "error", status: 0, reason: "This page has no start token: open it with `iohr ui open`." };
  let res: Response;
  try {
    res = await fetch(url, {
      method,
      headers: {
        "x-iohr-token": token,
        "content-type": "application/json",
        ...(opts.idempotencyKey ? { "idempotency-key": opts.idempotencyKey } : {}),
      },
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
    return {
      state: "error",
      status: res.status,
      reason: typeof m === "string" && m !== "" ? m : `The API answered HTTP ${res.status}.`,
    };
  }
  return json === null
    ? { state: "error", status: res.status, reason: "The API answer was not JSON." }
    : { state: "ok", data: json as T };
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
  opts: {
    query?: Record<string, string>;
    method?: string;
    body?: unknown;
    signal?: AbortSignal;
    onState?: (s: LiveState, reason: string) => void;
  } = {},
): () => void {
  const ctl = new AbortController();
  const stop = () => ctl.abort();
  opts.signal?.addEventListener("abort", stop, { once: true });
  const url = proxyPath(p, opts.query);
  const token = startToken();
  void (async () => {
    if (url === null || token === null) {
      opts.onState?.("off", url === null ? "That is not a path the page may read." : "This page has no start token.");
      return;
    }
    let wait = 1000;
    let first = true;
    while (!ctl.signal.aborted) {
      opts.onState?.(first ? "connecting" : "reconnecting", "");
      try {
        const res = await fetch(url, {
          method: opts.method ?? "GET",
          headers: {
            "x-iohr-token": token,
            accept: "text/event-stream",
            ...(opts.body !== undefined ? { "content-type": "application/json" } : {}),
          },
          body: opts.body === undefined ? undefined : JSON.stringify(opts.body),
          cache: "no-store",
          signal: ctl.signal,
        });
        if (res.status === 401) {
          opts.onState?.("off", "Signed out.");
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
                onEvent(JSON.parse(f.data) as T, { event: f.event, id: f.id });
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

/** A stream's folded value and its state, for a component. */
export function useLive<T, V>(
  p: string | null,
  opts: { query?: Record<string, string>; reduce: (prev: V, data: T, meta: StreamMeta) => V; initial: V },
): { value: V; state: LiveState; reason: string | null } {
  const [value, setValue] = useState<V>(opts.initial);
  const [state, setState] = useState<LiveState>(p === null ? "off" : "connecting");
  const [reason, setReason] = useState<string | null>(null);
  const reduce = useRef(opts.reduce);
  reduce.current = opts.reduce;
  const initial = useRef(opts.initial);
  const q = JSON.stringify(opts.query ?? {});
  useEffect(() => {
    if (p === null) {
      setState("off");
      return;
    }
    setValue(initial.current);
    return apiStream<T>(p, (d, m) => setValue((v) => reduce.current(v, d, m)), {
      query: JSON.parse(q) as Record<string, string>,
      onState: (s, r) => {
        setState(s);
        setReason(r === "" ? null : r);
      },
    });
  }, [p, q]);
  return { value, state, reason };
}

/** A write through the local server: ok, or the API's own reason in words. */
export async function apiSend(
  method: "POST" | "PATCH" | "PUT" | "DELETE",
  path: string,
  body?: unknown,
): Promise<{ ok: true } | { ok: false; reason: string }> {
  const r = await apiWrite(method, path, body);
  if (r.state === "ok") return { ok: true };
  if (r.state === "signed-out") return { ok: false, reason: "Signed out." };
  return { ok: false, reason: r.status === 403 ? "Your role may not do that." : r.reason };
}
