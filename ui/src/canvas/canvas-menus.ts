// The canvas menus as menu rows (frame/context-menu-core.ts), in the pattern of lib/page-menus.ts: pure, so node tests
// the rows and when they are off; the page says what each row does. After the Mac app's canvas context menus.

import { group, type MenuEntry } from "../frame/context-menu-core.ts";
import { type Item, TONES, type Tone } from "./schema.ts";

export type ItemActions = {
  edit: () => void;
  duplicate: () => void;
  front: () => void;
  back: () => void;
  tone: (t: Tone) => void;
  fit: () => void;
  remove: () => void;
  inspect: () => void;
};

const hasTone = (i: Item) => i.type === "note" || i.type === "frame";
const hasText = (i: Item) => i.type === "note" || i.type === "frame" || i.type === "shape";

/** The menu of a selection: edit (one text item), look, order, then remove last. `items` is what is selected. */
export function itemMenu(items: Item[], act: ItemActions): MenuEntry[] {
  const one = items.length === 1 ? items[0] : undefined;
  const none = items.length === 0;
  const tones = items.length > 0 && items.every(hasTone);
  return group(
    [
      {
        id: "item:edit",
        label: "Edit text",
        icon: "edit",
        keys: "Enter",
        disabled: !one ? "Select one item" : hasText(one) ? false : "This card has no text",
        run: act.edit,
      },
      { id: "item:inspect", label: "Details", icon: "info", disabled: one ? false : "Select one item", run: act.inspect },
    ],
    [
      {
        id: "item:tone",
        label: "Colour",
        icon: "palette",
        disabled: tones ? false : "Notes and frames only",
        items: TONES.map((t) => ({ id: `item:tone:${t}`, label: t[0].toUpperCase() + t.slice(1), run: () => act.tone(t) })),
      },
      { id: "item:front", label: "Bring to front", icon: "up", disabled: none, run: act.front },
      { id: "item:back", label: "Send to back", icon: "down", disabled: none, run: act.back },
      { id: "item:fit", label: "Zoom to selection", icon: "fit", keys: "⇧2", disabled: none, run: act.fit },
    ],
    [{ id: "item:dup", label: "Duplicate", icon: "copy", keys: "⌘D", disabled: none, run: act.duplicate }],
    [{ id: "item:remove", label: "Delete", icon: "trash", keys: "⌫", danger: true, disabled: none, run: act.remove }],
  );
}

export type BoardActions = {
  note: () => void;
  frame: () => void;
  add: () => void;
  fitAll: () => void;
  fitHome: () => void;
  undo: (() => void) | null;
  redo: (() => void) | null;
};

/** The menu of empty board: what to add here, the view, and undo or redo when there is something to go back to. */
export function boardMenu(act: BoardActions): MenuEntry[] {
  return group(
    [
      { id: "board:note", label: "Sticky note", icon: "note", keys: "N", run: act.note },
      { id: "board:frame", label: "Frame", icon: "frame", run: act.frame },
      { id: "board:add", label: "Add a card…", icon: "plus", run: act.add },
    ],
    [
      { id: "board:fit-all", label: "Zoom to fit everything", icon: "fit", keys: "⇧3", run: act.fitAll },
      { id: "board:fit-home", label: "Back to 100%", icon: "reset", keys: "⇧1", run: act.fitHome },
    ],
    [
      { id: "board:undo", label: "Undo", icon: "undo", keys: "⌘Z", disabled: act.undo ? false : "Nothing to undo", run: act.undo ?? undefined },
      { id: "board:redo", label: "Redo", icon: "redo", keys: "⇧⌘Z", disabled: act.redo ? false : "Nothing to redo", run: act.redo ?? undefined },
    ],
  );
}
