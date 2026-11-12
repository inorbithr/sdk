// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 InOrbit d.o.o.
//
// Moved from iohr-mac src/lib/sidebar-core.ts (origin/main d3ac8759) under ADR 0230: the shell frame layer
// lives here under Apache-2.0, and iohr-mac imports it (RFC 0217 C8 deletes its copy). Only the column's
// mode and width moved; the pins, sections and ranking stay in the app. The functions take any state
// that carries a mode and a width, so the app's own sidebar state passes through unchanged.

export const MODES = ["expanded", "rail", "hidden"] as const;
export type SidebarMode = (typeof MODES)[number];
export const MIN_WIDTH = 200;
export const MAX_WIDTH = 360;
export const DEFAULT_WIDTH = 256;
export const RAIL_WIDTH = 64;
/** Below this window width an expanded sidebar shows as the rail. */
export const NARROW_BELOW = 1100;

/** What the frame needs from a sidebar state. */
export type SidebarFrame = { mode: SidebarMode; width: number };

/** The counts a row or a rail icon can show. */
export type Badges = { waiting?: number; failing?: number; unread?: number };

export const clampWidth = (w: unknown) =>
  typeof w === "number" && Number.isFinite(w) ? Math.min(MAX_WIDTH, Math.max(MIN_WIDTH, Math.round(w))) : DEFAULT_WIDTH;

export function setWidth<S extends SidebarFrame>(s: S, w: number): S {
  const width = clampWidth(w);
  return width === s.width ? s : { ...s, width };
}

export function setMode<S extends SidebarFrame>(s: S, mode: SidebarMode): S {
  return s.mode === mode ? s : { ...s, mode };
}

/** The header button: expanded folds to the rail; the rail or hidden opens fully. */
export const toggleCollapsed = <S extends SidebarFrame>(s: S): S =>
  setMode(s, s.mode === "expanded" ? "rail" : "expanded");

/** ⌘\: hides the sidebar, or brings it back expanded. */
export const toggleHidden = <S extends SidebarFrame>(s: S): S =>
  setMode(s, s.mode === "hidden" ? "expanded" : "hidden");

/** What is drawn: an expanded sidebar in a narrow window is the rail; the stored mode is left alone. */
export function effectiveMode(mode: SidebarMode, viewport: number): SidebarMode {
  return mode === "expanded" && viewport < NARROW_BELOW ? "rail" : mode;
}

/** The column's width in px for what is drawn. */
export const columnWidth = (mode: SidebarMode, width: number) =>
  mode === "hidden" ? 0 : mode === "rail" ? RAIL_WIDTH : clampWidth(width);

/** A stored mode and width become a valid frame: an unknown mode is expanded, a bad width the default. */
export function parseFrame(o: unknown): SidebarFrame {
  const r = o && typeof o === "object" && !Array.isArray(o) ? (o as Record<string, unknown>) : {};
  const mode = (MODES as readonly unknown[]).includes(r.mode) ? (r.mode as SidebarMode) : "expanded";
  return { mode, width: clampWidth(r.width) };
}
