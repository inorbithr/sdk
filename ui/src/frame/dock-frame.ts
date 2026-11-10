// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 InOrbit d.o.o.
//
// Moved from iohr-mac src/lib/side-panels.ts (origin/main d3ac8759) under ADR 0230: the right dock's width limits
// (MIN_PANEL, MAX_PANEL, maxPanel, clampPanel and the first width). iohr-mac imports them in RFC 0217 C8 and deletes
// its copy. The Mac app keeps what is its own: which panels exist, their order among the boxes and the window that
// grows for them (Tauri). Like the sidebar's state, the dock's is plain data the app keeps where it likes.

export const MIN_PANEL = 300;
export const MAX_PANEL = 720;
/** What the page beside the dock always keeps. */
export const PAGE_ROOM = 420;
/** A dock's width before anyone drags it (the Mac app's comments and operations panels). */
export const FIRST_PANEL = 380;

/** As wide as the window allows, leaving the page beside it at least 420 px (never under MAX_PANEL). */
export const maxPanel = (win = typeof window === "undefined" ? 0 : window.innerWidth) =>
  Math.max(MAX_PANEL, win - PAGE_ROOM);
export const clampPanel = (w: number, win?: number) => Math.min(maxPanel(win), Math.max(MIN_PANEL, Math.round(w)));

/** What the frame needs from a dock state. */
export type DockFrame = { open: boolean; width: number };

/** A stored dock state made whole: anything missing or wrong falls back to closed at the first width. */
export function parseDock(o: unknown): DockFrame {
  const d = typeof o === "object" && o !== null ? (o as Record<string, unknown>) : {};
  const w = d.width;
  return {
    open: d.open === true,
    width: typeof w === "number" && Number.isFinite(w) ? Math.max(MIN_PANEL, Math.round(w)) : FIRST_PANEL,
  };
}

export function setDockWidth<S extends DockFrame>(s: S, w: number, win?: number): S {
  const width = clampPanel(w, win);
  return width === s.width ? s : { ...s, width };
}

export const toggleDock = <S extends DockFrame>(s: S): S => ({ ...s, open: !s.open });
export const closeDock = <S extends DockFrame>(s: S): S => (s.open ? { ...s, open: false } : s);
export const openDock = <S extends DockFrame>(s: S): S => (s.open ? s : { ...s, open: true });
