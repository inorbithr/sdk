// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 InOrbit d.o.o.
//
// How the window's boxes are arranged, the pure part (RFC 0217, slice 2). The Mac app places native windows on a 6 by
// 4 grid of the screen (iohr-mac src/lib/window-layouts.ts) and orders the boxes beside a page (side-panels.ts, the
// box bar's Arrange menu). A browser tab cannot move its own window, so the same words mean the same arrangement
// inside the page area: the page, with its dock, sits at a place on a 6 by 4 grid, and the page and the dock can swap
// sides. No React, no DOM, so node tests it. The state is plain data the app keeps where it likes (localStorage here).

export const GRID = { cols: 6, rows: 4 } as const;
/** A place never gets smaller than this many cells, so the page's header and its way back stay reachable. */
export const MIN_SPAN = { cols: 2, rows: 2 } as const;

/** A rectangle of grid cells: columns c0 up to (not including) c1, rows r0 up to r1. */
export type Place = { c0: number; r0: number; c1: number; r1: number };

export const FULL: Place = { c0: 0, r0: 0, c1: GRID.cols, r1: GRID.rows };

export const BOXES = ["page", "dock"] as const;
export type BoxId = (typeof BOXES)[number];
export const DEFAULT_ORDER: BoxId[] = ["page", "dock"];

export type Snapshot = { page: Place; dock: Place | null };

/** Where the page sits, where the dock sits when it has a place of its own, and what "Back to before" returns to. */
export type Arrangement = { order: BoxId[]; page: Place; dock: Place | null; before: Snapshot | null };

export const initial = (): Arrangement => ({ order: [...DEFAULT_ORDER], page: FULL, dock: null, before: null });

export type LayoutId = "side-by-side" | "two-thirds" | "one-third" | "left" | "right" | "top" | "bottom" | "full";

/** Ready layouts in the order the menu lists them; `pair` ones give the page and the dock a share each. */
export const LAYOUTS: { id: LayoutId; label: string; pair: boolean }[] = [
  { id: "side-by-side", label: "Side by side", pair: true },
  { id: "two-thirds", label: "Two thirds and one third", pair: true },
  { id: "one-third", label: "One third and two thirds", pair: true },
  { id: "left", label: "Left half", pair: false },
  { id: "right", label: "Right half", pair: false },
  { id: "top", label: "Top half", pair: false },
  { id: "bottom", label: "Bottom half", pair: false },
  { id: "full", label: "Full", pair: false },
];

const col = (c0: number, c1: number): Place => ({ c0, r0: 0, c1, r1: GRID.rows });
const row = (r0: number, r1: number): Place => ({ c0: 0, r0, c1: GRID.cols, r1 });

const SINGLE: Record<string, Place> = {
  left: col(0, 3),
  right: col(3, 6),
  top: row(0, 2),
  bottom: row(2, 4),
  full: FULL,
};

/** The column split of a pair layout: how many of the 6 columns the first box gets. */
const FIRST_COLS: Record<string, number> = { "side-by-side": 3, "two-thirds": 4, "one-third": 2 };

const same = (a: Place | null, b: Place | null) =>
  a === b || (a !== null && b !== null && a.c0 === b.c0 && a.r0 === b.r0 && a.c1 === b.c1 && a.r1 === b.r1);
export const samePlace = same;

const snap = (a: Arrangement): Snapshot => ({ page: a.page, dock: a.dock });
const unchanged = (a: Arrangement, page: Place, dock: Place | null) => same(a.page, page) && same(a.dock, dock);

/** Puts the arrangement into `page` and `dock`, remembering what it was; no change returns the same object. */
function place(a: Arrangement, page: Place, dock: Place | null): Arrangement {
  if (unchanged(a, page, dock)) return a;
  return { ...a, page, dock, before: snap(a) };
}

/**
 * A ready layout. A pair layout gives the first box in the order the first share (so "Two thirds and one third"
 * puts two thirds on whichever box is first) and the dock the other cells; the others place the page and its dock
 * together. `opensDock` says the dock must be shown for the layout to mean anything.
 */
export function applyLayout(a: Arrangement, id: LayoutId): Arrangement {
  const first = FIRST_COLS[id];
  if (first !== undefined) {
    const a0 = col(0, first);
    const a1 = col(first, GRID.cols);
    return a.order[0] === "page" ? place(a, a0, a1) : place(a, a1, a0);
  }
  const p = SINGLE[id];
  return p ? place(a, p, null) : a;
}

export const opensDock = (id: LayoutId) => LAYOUTS.some((l) => l.id === id && l.pair);

/** A drag from one cell to another (either order) is the box of cells between them, grown to the minimum span. */
export function spanPlace(from: { col: number; row: number }, to: { col: number; row: number }): Place {
  const clamp = (n: number, max: number) => Math.min(Math.max(Math.trunc(n), 0), max - 1);
  let c0 = clamp(Math.min(from.col, to.col), GRID.cols);
  let c1 = clamp(Math.max(from.col, to.col), GRID.cols) + 1;
  let r0 = clamp(Math.min(from.row, to.row), GRID.rows);
  let r1 = clamp(Math.max(from.row, to.row), GRID.rows) + 1;
  // Grown towards the free side so the box never leaves the grid.
  while (c1 - c0 < MIN_SPAN.cols) {
    if (c1 < GRID.cols) c1 += 1;
    else c0 -= 1;
  }
  while (r1 - r0 < MIN_SPAN.rows) {
    if (r1 < GRID.rows) r1 += 1;
    else r0 -= 1;
  }
  return { c0, r0, c1, r1 };
}

/** The page and its dock in one chosen box of cells. */
export const applySpan = (a: Arrangement, p: Place): Arrangement => place(a, p, null);

/** "Back to before": the arrangement from before the last change; none is left to go back to afterwards. */
export function back(a: Arrangement): Arrangement {
  if (a.before === null) return a;
  return { ...a, page: a.before.page, dock: a.before.dock, before: null };
}

/** The dock was closed: a pair layout ends (the page takes the whole area), anything else stays as it is. */
export function dockClosed(a: Arrangement): Arrangement {
  return a.dock === null ? a : place(a, FULL, null);
}

/** Whether the page is anywhere but the whole area, or the boxes are swapped. */
export const arranged = (a: Arrangement) => layoutOff(a) || a.order[0] !== "page";

/** Whether the page is anywhere but the whole area. */
export const layoutOff = (a: Arrangement) => !same(a.page, FULL) || a.dock !== null;

/** What the way-back chip does: the page to full when it is not, else the boxes back in their first order. */
export const restore = (a: Arrangement): Arrangement => (layoutOff(a) ? applyLayout(a, "full") : resetOrder(a));

/** Name of what the page is in now, for the way-back chip; "" when the page fills the area. */
export function describe(a: Arrangement): string {
  if (a.dock !== null) {
    const cols = (a.order[0] === "page" ? a.page : (a.dock as Place)).c1;
    return cols === 3 ? "Side by side" : cols === 4 ? "Two thirds and one third" : "One third and two thirds";
  }
  const hit = LAYOUTS.find((l) => !l.pair && SINGLE[l.id] && same(SINGLE[l.id], a.page));
  if (hit) return hit.id === "full" ? "" : hit.label;
  return `Grid ${a.page.c1 - a.page.c0} by ${a.page.r1 - a.page.r0}`;
}

// ---------------------------------------------------------------- the box order (the Arrange menu)

/** Where a box is in the order, -1 when it is not a box. */
export const indexOf = (order: BoxId[], id: BoxId) => order.indexOf(id);

/** Moves a box one step left or right, or to either end; the same array comes back when it cannot move. */
export function moveBox(order: BoxId[], id: BoxId, to: "left" | "right" | "start" | "end"): BoxId[] {
  const i = order.indexOf(id);
  if (i < 0) return order;
  const j = to === "left" ? i - 1 : to === "right" ? i + 1 : to === "start" ? 0 : order.length - 1;
  if (j < 0 || j >= order.length || j === i) return order;
  const next = [...order];
  next.splice(i, 1);
  next.splice(j, 0, id);
  return next;
}

export const resetOrder = (a: Arrangement): Arrangement =>
  a.order[0] === "page" ? a : { ...a, order: [...DEFAULT_ORDER] };

/** Moves one box. A pair layout keeps its first share on whichever box is first, so its cells swap with the order. */
export function arrangeBox(a: Arrangement, id: BoxId, to: "left" | "right" | "start" | "end"): Arrangement {
  const order = moveBox(a.order, id, to);
  if (order === a.order) return a;
  const next: Arrangement = { ...a, order };
  return a.dock === null ? next : { ...next, page: a.dock, dock: a.page };
}

/** "Reset the arrangement": the box order, the places and the way back, all as at the start. */
export const resetAll = (a: Arrangement): Arrangement => (arranged(a) || a.before !== null ? initial() : a);

// ---------------------------------------------------------------- reading what was kept

const isPlace = (o: unknown): o is Place => {
  if (typeof o !== "object" || o === null) return false;
  const p = o as Record<string, unknown>;
  const ints = [p.c0, p.r0, p.c1, p.r1].every((n) => Number.isInteger(n));
  if (!ints) return false;
  const { c0, r0, c1, r1 } = p as Place;
  return (
    c0 >= 0 && r0 >= 0 && c1 <= GRID.cols && r1 <= GRID.rows && c1 - c0 >= MIN_SPAN.cols && r1 - r0 >= MIN_SPAN.rows
  );
};

const exact = (p: Place): Place => ({ c0: p.c0, r0: p.r0, c1: p.c1, r1: p.r1 });

const parseSnapshot = (o: unknown): Snapshot | null => {
  const r = typeof o === "object" && o !== null ? (o as Record<string, unknown>) : null;
  if (r === null || !isPlace(r.page)) return null;
  if (r.dock !== null && r.dock !== undefined && !isPlace(r.dock)) return null;
  return { page: exact(r.page), dock: r.dock ? exact(r.dock as Place) : null };
};

/** A stored arrangement made whole: anything missing or wrong is the start (the page fills the area, page then dock). */
export function parseArrangement(o: unknown): Arrangement {
  const r = typeof o === "object" && o !== null && !Array.isArray(o) ? (o as Record<string, unknown>) : {};
  const order =
    Array.isArray(r.order) && r.order.length === 2 && BOXES.every((b) => (r.order as unknown[]).includes(b))
      ? (r.order as BoxId[])
      : [...DEFAULT_ORDER];
  const now = parseSnapshot(r);
  const before = parseSnapshot(r.before);
  if (now === null) return { ...initial(), order, before };
  return { order, page: now.page, dock: now.dock, before };
}

// ---------------------------------------------------------------- the grid picker's keys

/** The cell an arrow key moves the picker's cursor to; it stops at the edge of the grid. */
export function moveCell(at: { col: number; row: number }, key: string): { col: number; row: number } {
  const d: Record<string, [number, number]> = {
    ArrowLeft: [-1, 0],
    ArrowRight: [1, 0],
    ArrowUp: [0, -1],
    ArrowDown: [0, 1],
  };
  const [dc, dr] = d[key] ?? [0, 0];
  return {
    col: Math.min(GRID.cols - 1, Math.max(0, at.col + dc)),
    row: Math.min(GRID.rows - 1, Math.max(0, at.row + dr)),
  };
}
