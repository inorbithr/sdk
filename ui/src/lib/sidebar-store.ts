import { useSyncExternalStore } from "react";
import type { UpdateSidebar } from "../frame/frame.tsx";
import { parseFrame, type SidebarFrame } from "../frame/sidebar-frame.ts";

// This page's sidebar state: the frame's mode and width, kept in the browser (localStorage) for this origin. It
// follows iohr-mac's src/lib/sidebar-store.ts, which keeps more (pins, projects) in a file; the frame itself owns no
// state, so each app keeps it where it likes. Storage can be missing or refused (a private window, blocked site
// data); the page then starts from the default and keeps the state for this load only.

const KEY = "iohr-ui.sidebar";

const read = (): SidebarFrame => {
  try {
    const t = localStorage.getItem(KEY);
    return parseFrame(t ? JSON.parse(t) : null);
  } catch {
    return parseFrame(null);
  }
};

let state: SidebarFrame = read();
const subs = new Set<() => void>();
const subscribe = (f: () => void) => {
  subs.add(f);
  return () => {
    subs.delete(f);
  };
};
const get = () => state;

export const updateSidebar: UpdateSidebar = (f) => {
  const next = f(state);
  if (next === state) return;
  state = next;
  try {
    localStorage.setItem(KEY, JSON.stringify(next));
  } catch {
    // Not kept past this load; the state in memory is still right.
  }
  for (const s of subs) s();
};

export function useSidebar() {
  return [useSyncExternalStore(subscribe, get, get), updateSidebar] as const;
}
