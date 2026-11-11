// One read the page keeps current: it loads, then asks again every `every` ms (the Decisions proto has no stream, so
// polling at a stated interval is how this stays live), skips a hidden tab, and drops an answer that a newer read
// or a changed key has replaced.
import { useEffect, useRef, useState } from "react";
import type { Read } from "../lib/api.ts";

export function useRead<T>(load: () => Promise<Read<T>>, key: string, every = 0): Read<T> | null {
  const [state, setState] = useState<{ key: string; read: Read<T> } | null>(null);
  const gen = useRef(0);
  const run = useRef(load);
  run.current = load;
  // biome-ignore lint/correctness/useExhaustiveDependencies: `key` names what `load` reads; the latest `load` is in the ref.
  useEffect(() => {
    let live = true;
    const go = async () => {
      const mine = ++gen.current;
      const read = await run.current();
      if (live && mine === gen.current) setState({ key, read });
    };
    void go();
    if (every <= 0)
      return () => {
        live = false;
      };
    const id = window.setInterval(() => {
      if (!document.hidden) void go();
    }, every);
    return () => {
      live = false;
      window.clearInterval(id);
    };
  }, [key, every]);
  return state !== null && state.key === key ? state.read : null;
}
