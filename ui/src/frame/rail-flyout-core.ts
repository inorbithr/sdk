// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 InOrbit d.o.o.
//
// Moved from iohr-mac src/lib/rail-flyout.ts (origin/main d3ac8759) under ADR 0230: the shell frame layer
// lives here under Apache-2.0, and iohr-mac imports it (RFC 0217 C8 deletes its copy).
//
// The collapsed rail's flyout submenus (PRD 0106): when it opens and closes, as a pure state machine
// free of the app so it can be tested. The component feeds it events with the time, sets one timer
// for `state.pending.at`, and sends a "tick" when it fires.
//
//   closed --hover 150ms--> open(hover) --leave, 200ms--> closed
//   open(any) --hover another icon--> open(that icon) at once
//   open(hover) --click icon--> open(pinned) --click icon again--> go to the page (and close)
//   open(pinned) --Esc / click elsewhere--> closed
//   keyboard: → or Enter on an icon opens it pinned and focuses the first item; ↑ ↓ move; ← or Esc closes.

export const OPEN_DELAY = 150;
export const CLOSE_DELAY = 200;

export type Pending = { op: "open"; id: string; at: number } | { op: "close"; at: number };
/** Where keyboard focus should be: the rail icon, or entry n of the flyout (0 is its name link). */
export type FocusTarget = "icon" | number | null;

export type FlyoutState = {
  /** The icon whose flyout is showing. */
  open: string | null;
  /** Kept open by a click or the keyboard: pointer movement no longer closes it. */
  pinned: boolean;
  pending: Pending | null;
  focus: FocusTarget;
  /** Grows whenever a focus move is asked for, so asking for the same target twice still acts. */
  focusSeq: number;
};

export const initial: FlyoutState = { open: null, pinned: false, pending: null, focus: null, focusSeq: 0 };

export type FlyoutEvent =
  | { type: "iconEnter"; id: string }
  | { type: "iconLeave"; id: string }
  | { type: "flyoutEnter" }
  | { type: "flyoutLeave" }
  /** The pointer moved while a close is pending; `safe` says it is heading into the flyout (inside the safe triangle). */
  | { type: "pointerMove"; safe: boolean }
  | { type: "iconClick"; id: string; hasItems: boolean }
  /** → or Enter on a rail icon; `items` counts the flyout's items under its name. */
  | { type: "keyOpen"; id: string; items: number }
  | { type: "key"; key: "ArrowUp" | "ArrowDown" | "ArrowLeft" | "Home" | "End"; items: number }
  | { type: "escape" }
  | { type: "outsideClick" }
  | { type: "tick" }
  /** The section went away (for instance the sidebar is no longer the rail). */
  | { type: "reset" };

export type Step = { state: FlyoutState /** Navigate to this icon's page. */; go?: string };

const ask = (s: FlyoutState, focus: FocusTarget): FlyoutState => ({ ...s, focus, focusSeq: s.focusSeq + 1 });
const shut = (s: FlyoutState): FlyoutState => ({ ...s, open: null, pinned: false, pending: null });
const scheduleClose = (s: FlyoutState, now: number): FlyoutState =>
  s.open && !s.pinned ? { ...s, pending: { op: "close", at: now + CLOSE_DELAY } } : s;

export function step(s: FlyoutState, e: FlyoutEvent, now: number): Step {
  switch (e.type) {
    case "iconEnter": {
      if (s.open === e.id) return { state: { ...s, pending: null } };
      // Another flyout is showing: switch at once. Otherwise rest on the icon first.
      if (s.open) return { state: { ...s, open: e.id, pending: null, focus: null } };
      return { state: { ...s, pending: { op: "open", id: e.id, at: now + OPEN_DELAY } } };
    }
    case "iconLeave": {
      if (s.pending?.op === "open" && s.pending.id === e.id) return { state: { ...s, pending: null } };
      if (s.open === e.id) return { state: scheduleClose(s, now) };
      return { state: s };
    }
    case "flyoutEnter":
      return { state: s.pending?.op === "close" ? { ...s, pending: null } : s };
    case "flyoutLeave":
      return { state: scheduleClose(s, now) };
    case "pointerMove":
      // Heading for the flyout: the close waits, the whole 200 ms again.
      return {
        state: s.pending?.op === "close" && e.safe ? { ...s, pending: { op: "close", at: now + CLOSE_DELAY } } : s,
      };
    case "iconClick": {
      if (!e.hasItems) return { state: shut(s), go: e.id };
      if (s.open === e.id && s.pinned) return { state: shut(s), go: e.id };
      return { state: { ...s, open: e.id, pinned: true, pending: null } };
    }
    case "keyOpen":
      return { state: ask({ ...s, open: e.id, pinned: true, pending: null }, e.items > 0 ? 1 : 0) };
    case "key": {
      if (!s.open) return { state: s };
      if (e.key === "ArrowLeft") return { state: ask(shut(s), "icon") };
      const last = e.items; // entries 0..items
      const at = typeof s.focus === "number" ? Math.min(s.focus, last) : -1;
      const to =
        e.key === "Home"
          ? 0
          : e.key === "End"
            ? last
            : e.key === "ArrowDown"
              ? at >= last
                ? 0
                : at + 1
              : at <= 0
                ? last
                : at - 1;
      return { state: ask(s, to) };
    }
    case "escape":
      // Esc also cancels a hover that has not opened yet.
      return s.open ? { state: ask(shut(s), "icon") } : { state: s.pending ? { ...s, pending: null } : s };
    case "outsideClick":
      return s.open ? { state: shut(s) } : { state: s };
    case "tick": {
      const p = s.pending;
      if (!p || now < p.at) return { state: s };
      if (p.op === "open") return { state: { ...s, open: p.id, pending: null } };
      return { state: shut(s) };
    }
    case "reset":
      return { state: shut({ ...s, focus: null }) };
  }
}

// ---------------------------------------------------------------- the safe triangle

export type Pt = { x: number; y: number };
export type Box = { left: number; top: number; right: number; bottom: number };

const side = (p: Pt, a: Pt, b: Pt) => (p.x - b.x) * (a.y - b.y) - (a.x - b.x) * (p.y - b.y);

/**
 * Is the pointer on its way from the icon to the flyout? The triangle runs from where the pointer
 * left the icon to the flyout's two near corners (its left edge), so a diagonal move keeps it open
 * and a move away from it does not. The flyout itself counts as safe.
 */
export function inSafeTriangle(p: Pt, from: Pt, flyout: Box): boolean {
  if (p.x >= flyout.left && p.x <= flyout.right && p.y >= flyout.top && p.y <= flyout.bottom) return true;
  const a = { x: flyout.left, y: flyout.top };
  const b = { x: flyout.left, y: flyout.bottom };
  const d1 = side(p, from, a);
  const d2 = side(p, a, b);
  const d3 = side(p, b, from);
  const neg = d1 < 0 || d2 < 0 || d3 < 0;
  const pos = d1 > 0 || d2 > 0 || d3 > 0;
  return !(neg && pos);
}

/**
 * Where the flyout sits: its left at the rail's edge, its top level with the icon, then moved up
 * so it ends inside the window (and never above it). `height` is its measured height.
 */
export function placeFlyout(
  icon: Box,
  height: number,
  viewport: { w: number; h: number },
  margin = 8,
): { left: number; top: number; maxHeight: number } {
  const maxHeight = Math.max(80, viewport.h - margin * 2);
  const h = Math.min(height, maxHeight);
  const top = Math.max(margin, Math.min(icon.top, viewport.h - margin - h));
  return { left: icon.right + 6, top, maxHeight };
}
