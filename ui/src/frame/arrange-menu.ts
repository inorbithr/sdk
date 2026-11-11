// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 InOrbit d.o.o.
//
// The layout button's menu and the Arrange menu as the shared menu model (frame/context-menu-core.ts), in the Mac
// app's order and words (iohr-mac src/components/window-layout-button.tsx and box-bar.tsx). Pure: it says what the
// rows are and which are off; what a row does is the caller's, so node tests it.

import { type Arrangement, type BoxId, indexOf, LAYOUTS, type LayoutId } from "./arrange.ts";
import { group, type MenuEntry } from "./context-menu-core.ts";
import { keysOf } from "./shortcuts.ts";

export type LayoutActions = {
  layout: (id: LayoutId) => void;
  grid: () => void;
  back: () => void;
};

const KEYED: Partial<Record<LayoutId, string>> = {
  left: "layout-left",
  right: "layout-right",
  top: "layout-top",
  bottom: "layout-bottom",
  full: "layout-full",
};

/** Side by side, the thirds, the halves and Full, then Grid…, then Back to before (off with nothing to go back to). */
export function layoutMenu(a: Arrangement, act: LayoutActions): MenuEntry[] {
  const rows = (pair: boolean): MenuEntry[] =>
    LAYOUTS.filter((l) => l.pair === pair).map((l) => ({
      id: `layout:${l.id}`,
      label: l.label,
      icon: pair ? "layout-pair" : "layout",
      keys: KEYED[l.id] ? keysOf(KEYED[l.id] as string) : undefined,
      run: () => act.layout(l.id),
    }));
  return group(
    rows(true),
    rows(false),
    [{ id: "layout:grid", label: "Grid…", icon: "grid", keys: keysOf("layout-grid"), run: act.grid }],
    [
      {
        id: "layout:back",
        label: "Back to before",
        icon: "undo",
        keys: keysOf("layout-back"),
        disabled: a.before === null ? "Nothing to go back to yet" : false,
        run: act.back,
      },
    ],
  );
}

export type ArrangeActions = {
  move: (to: "left" | "right" | "start" | "end") => void;
  reset: () => void;
};

/** Move left, right, to the start or the end, then Reset the arrangement; a move the box cannot make is off. */
export function arrangeMenu(a: Arrangement, id: BoxId, act: ArrangeActions): MenuEntry[] {
  const i = indexOf(a.order, id);
  const atStart = i <= 0;
  const atEnd = i < 0 || i >= a.order.length - 1;
  const off = (at: boolean, why: string) => (at ? why : false);
  return group(
    [
      {
        id: "arrange:left",
        label: "Move left",
        icon: "left",
        disabled: off(atStart, "Already first"),
        run: () => act.move("left"),
      },
      {
        id: "arrange:right",
        label: "Move right",
        icon: "right",
        disabled: off(atEnd, "Already last"),
        run: () => act.move("right"),
      },
      {
        id: "arrange:start",
        label: "Move to the start",
        icon: "to-start",
        disabled: off(atStart, "Already first"),
        run: () => act.move("start"),
      },
      {
        id: "arrange:end",
        label: "Move to the end",
        icon: "to-end",
        disabled: off(atEnd, "Already last"),
        run: () => act.move("end"),
      },
    ],
    [{ id: "arrange:reset", label: "Reset the arrangement", icon: "reset", run: act.reset }],
  );
}
