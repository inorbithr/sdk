import { useSyncExternalStore } from "react";

// A piece of this page's state kept in the browser (localStorage) for this origin: the sidebar's and the dock's.
// The frame owns no state, so each app keeps it where it likes (iohr-mac keeps its sidebar in a file). Storage can
// be missing or refused (a private window, blocked site data); the page then starts from the default the parser
// gives and keeps the state for this load only.

export type Stored<T> = {
  /** The state now, live. */
  use: () => T;
  /** The state now, once (outside React). */
  get: () => T;
  /** Calls `f` after every change; returns the way to stop. */
  subscribe: (f: () => void) => () => void;
  /** Changes the state; one that returns the same object changes nothing. */
  update: (f: (s: T) => T) => void;
};

export function stored<T>(key: string, parse: (o: unknown) => T): Stored<T> {
  const read = (): T => {
    try {
      const t = localStorage.getItem(key);
      return parse(t ? JSON.parse(t) : null);
    } catch {
      return parse(null);
    }
  };
  let state = read();
  const subs = new Set<() => void>();
  const subscribe = (f: () => void) => {
    subs.add(f);
    return () => {
      subs.delete(f);
    };
  };
  const get = () => state;
  return {
    use: () => useSyncExternalStore(subscribe, get, get),
    get,
    subscribe,
    update: (f) => {
      const next = f(state);
      if (next === state) return;
      state = next;
      try {
        localStorage.setItem(key, JSON.stringify(next));
      } catch {
        // Not kept past this load; the state in memory is still right.
      }
      for (const s of subs) s();
    },
  };
}
