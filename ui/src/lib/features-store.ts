import { useEffect, useState, useSyncExternalStore } from "react";
import {
  bringBack,
  bringEverythingOnline,
  featureOf,
  parseStates,
  putInMaintenance,
  type States,
  type Undo,
  undoLive,
  undoOf,
} from "./features-core.ts";
import { stored } from "./stored.ts";

// Which pages are in maintenance on this machine (lib/features-core.ts, lib/stored.ts), and the one change that can be
// taken back for ten seconds. The undo is kept in memory only: a reload ends it, which is the safe side.
const states = stored<States>("iohr-ui.features", parseStates);

export const useFeatures = states.use;

let undo: Undo | null = null;
const subs = new Set<() => void>();
const emit = () => {
  for (const f of subs) f();
};
const subscribe = (f: () => void) => {
  subs.add(f);
  return () => {
    subs.delete(f);
  };
};

function change(label: string, f: (s: States) => States) {
  let before: States | null = null;
  states.update((s) => {
    const next = f(s);
    if (next !== s) before = s;
    return next;
  });
  if (before !== null) {
    undo = undoOf(label, before, Date.now());
    emit();
  }
}

export const putDown = (id: string, why: { reason: string; eta: string }) =>
  change(`Put ${featureOf(id)?.name ?? id} in maintenance`, (s) => putInMaintenance(s, id, why));
export const backOnline = (id: string) =>
  change(`Brought ${featureOf(id)?.name ?? id} back online`, (s) => bringBack(s, id));
export const allOnline = () => change("Brought everything online", bringEverythingOnline);

/** Takes the last change back, while it can still be taken back. */
export function undoLast() {
  if (!undoLive(undo, Date.now())) return;
  const before = undo.before;
  undo = null;
  states.update(() => before);
  emit();
}

/** The change that can be taken back now, or null; it lapses by itself after ten seconds. */
export function useUndo(): Undo | null {
  const u = useSyncExternalStore(
    subscribe,
    () => undo,
    () => null,
  );
  const [, tick] = useState(0);
  useEffect(() => {
    if (u === null) return;
    const wait = u.until - Date.now();
    if (wait <= 0) return;
    const id = window.setTimeout(() => tick((n) => n + 1), wait + 5);
    return () => window.clearTimeout(id);
  }, [u]);
  return undoLive(u, Date.now()) ? u : null;
}
