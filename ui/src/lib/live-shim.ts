// A thin stand-in for the live agent's `useLive` (board: ui-live) until it merges; delete this file then.
// Same call shape: `useLive(path, { reduce, initial })` gives `{ value, state, reason }`. It reads the
// stream through the local server's proxy with the per-serve token, resumes after a drop, and never
// holds a credential of its own.
import { useEffect, useRef, useState } from "react";
import { proxyPath } from "./api.ts";
import { startToken } from "./panels.ts";

export type LiveState = "connecting" | "live" | "reconnecting" | "off";

/** The data lines of one SSE block, joined, or null for a comment or heartbeat block. */
export function sseData(block: string): string | null {
  const lines = block
    .split("\n")
    .filter((l) => l.startsWith("data:"))
    .map((l) => l.slice(5).replace(/^ /, ""));
  return lines.length ? lines.join("\n") : null;
}

export function useLive<T, V>(
  path: string | null,
  opts: { reduce: (prev: V, e: T) => V; initial: V },
): { value: V; state: LiveState; reason: string | null } {
  const [value, setValue] = useState<V>(opts.initial);
  const [state, setState] = useState<LiveState>(path ? "connecting" : "off");
  const [reason, setReason] = useState<string | null>(null);
  const reduce = useRef(opts.reduce);
  reduce.current = opts.reduce;
  const initial = useRef(opts.initial);
  useEffect(() => {
    if (!path) {
      setState("off");
      return;
    }
    const ctl = new AbortController();
    let acc = initial.current;
    let wait = 1000;
    const run = async () => {
      while (!ctl.signal.aborted) {
        const url = proxyPath(path);
        const token = startToken();
        if (url === null || token === null)
          return void (setState("off"), setReason("This page has no start token or path."));
        try {
          const res = await fetch(url, {
            headers: { "x-iohr-token": token, accept: "text/event-stream" },
            signal: ctl.signal,
            cache: "no-store",
          });
          if (!res.ok || !res.body) throw new Error(`HTTP ${res.status}`);
          setState("live");
          setReason(null);
          wait = 1000;
          const reader = res.body.pipeThrough(new TextDecoderStream()).getReader();
          let buf = "";
          for (;;) {
            const { value: chunk, done } = await reader.read();
            if (done) break;
            buf += chunk.replace(/\r\n/g, "\n");
            let i = buf.indexOf("\n\n");
            while (i >= 0) {
              const data = sseData(buf.slice(0, i));
              buf = buf.slice(i + 2);
              if (data) {
                try {
                  acc = reduce.current(acc, JSON.parse(data) as T);
                  setValue(acc);
                } catch {
                  // a frame that is not JSON is skipped
                }
              }
              i = buf.indexOf("\n\n");
            }
          }
        } catch (e) {
          if (ctl.signal.aborted) return;
          setReason(e instanceof Error ? e.message : "The stream dropped.");
        }
        setState("reconnecting");
        await new Promise((r) => setTimeout(r, wait));
        wait = Math.min(wait * 2, 30_000);
      }
    };
    void run();
    return () => ctl.abort();
  }, [path]);
  return { value, state, reason };
}

/** A write through the local server, standing in for the live agent's `apiSend`: the API's own answer or reason. */
export async function apiSend(
  method: "POST" | "PATCH" | "PUT" | "DELETE",
  path: string,
  body?: unknown,
): Promise<{ ok: true } | { ok: false; reason: string }> {
  const url = proxyPath(path);
  const token = startToken();
  if (url === null || token === null) return { ok: false, reason: "This page has no start token or path." };
  try {
    const res = await fetch(url, {
      method,
      headers: { "x-iohr-token": token, "content-type": "application/json" },
      body: body === undefined ? undefined : JSON.stringify(body),
      cache: "no-store",
    });
    if (res.ok) return { ok: true };
    let reason = res.status === 403 ? "Your role may not do that." : `The API answered HTTP ${res.status}.`;
    try {
      const e = (await res.json()) as { error?: unknown };
      const m = typeof e.error === "string" ? e.error : (e.error as { message?: unknown } | undefined)?.message;
      if (typeof m === "string" && m) reason = m;
    } catch {
      // the status line says it
    }
    return { ok: false, reason };
  } catch {
    return { ok: false, reason: "The local `iohr ui serve` did not answer." };
  }
}
