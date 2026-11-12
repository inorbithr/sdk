// `useLive`: the React face of `apiStream` (RFC 0217, the live slice). A view says which stream it wants and how an
// event folds into its value; the hook carries the "live / reconnecting / offline" state beside it.
import { useEffect, useRef, useState } from "react";
import { apiStream, type LiveState, type StreamMeta } from "./api.ts";

export type { LiveState } from "./api.ts";

export type Live<V> = { value: V | undefined; state: LiveState; reason: string | null };

/** The stream at `path` (null: off). Without `reduce` the value is the latest event. */
export function useLive<T = unknown, V = T>(
  path: string | null,
  opts: {
    query?: Record<string, string>;
    resumeParam?: string;
    reduce?: (prev: V | undefined, event: T, meta: StreamMeta) => V;
    initial?: V;
  } = {},
): Live<V> {
  const [live, setLive] = useState<Live<V>>({ value: opts.initial, state: "connecting", reason: null });
  const reduce = useRef(opts.reduce);
  reduce.current = opts.reduce;
  const key = JSON.stringify(opts.query ?? {});
  // biome-ignore lint/correctness/useExhaustiveDependencies: the stream restarts on the path and the query only
  useEffect(() => {
    if (path === null) {
      setLive({ value: opts.initial, state: "offline", reason: null });
      return;
    }
    setLive((l) => ({ ...l, state: "connecting", reason: null }));
    return apiStream<T>(
      path,
      (event, meta) =>
        setLive((l) => ({
          ...l,
          value: reduce.current ? reduce.current(l.value, event, meta) : (event as unknown as V),
        })),
      {
        ...(opts.query ? { query: opts.query } : {}),
        ...(opts.resumeParam ? { resumeParam: opts.resumeParam } : {}),
        onState: (state, why) => setLive((l) => ({ ...l, state, reason: why ?? null })),
      },
    );
  }, [path, key]);
  return live;
}
