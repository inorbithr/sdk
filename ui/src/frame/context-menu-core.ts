// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 InOrbit d.o.o.
//
// Moved from iohr-mac src/lib/context-menu-core.ts (origin/main d3ac8759) under ADR 0230: the shell frame layer
// lives here under Apache-2.0, and iohr-mac imports it (RFC 0217 C8 deletes its copy).
//
// The one context menu's model (PRD "Enterprise context menu"): what an item is, how a list of items is tidied
// into groups, how Excalidraw-style shortcut text becomes Mac glyphs, which row the arrows and type-ahead land on,
// and where the panel goes so it never leaves the window. Pure: no React, no DOM, so node tests it.
//
// The model is plain data (ids, labels, icon keys as strings, glyph strings), so the Flutter app's long-press
// menu (core/mobile/apps/iohr) can read the same JSON later and draw it its own way.

export type MenuAction = {
  id: string;
  label: string;
  /** A key in the icon registry (components/context-menu.tsx); unknown keys draw no icon. */
  icon?: string;
  /** The shortcut as Mac glyphs ("⇧⌘C"); `glyphs()` turns "Cmd+Shift+C" into it. */
  keys?: string;
  /** Set (true or false) for a toggle: it shows a check when true and has role menuitemcheckbox. */
  checked?: boolean;
  /** true, or the reason it can't be used now (shown as its tooltip). */
  disabled?: boolean | string;
  /** Destructive: drawn in the danger colour. Put these last. */
  danger?: boolean;
  /** A short grey line after the label. */
  hint?: string;
  run?: () => void;
  /** A submenu. */
  items?: MenuEntry[];
};
export type MenuSeparator = { sep: true };
export type MenuHeading = { heading: string };
export type MenuEntry = MenuAction | MenuSeparator | MenuHeading;

export const SEP: MenuSeparator = { sep: true };
export const heading = (heading: string): MenuHeading => ({ heading });
export const isAction = (e: MenuEntry): e is MenuAction => "id" in e;
export const isSep = (e: MenuEntry): e is MenuSeparator => "sep" in e;
export const isHeading = (e: MenuEntry): e is MenuHeading => "heading" in e;
export const isOff = (a: MenuAction) => !!a.disabled;
export const reasonOf = (a: MenuAction) => (typeof a.disabled === "string" ? a.disabled : "");

/** Drops what shows nothing: a separator at either end or twice in a row, a heading with nothing under it. */
export function tidy(entries: MenuEntry[]): MenuEntry[] {
  const out: MenuEntry[] = [];
  for (const e of entries) {
    if (isSep(e) && (out.length === 0 || isSep(out[out.length - 1]))) continue;
    out.push(isAction(e) && e.items ? { ...e, items: tidy(e.items) } : e);
  }
  // A heading is dropped when the next row is a separator or the end.
  const kept = out.filter((e, i) => !isHeading(e) || (out[i + 1] && !isSep(out[i + 1])));
  while (kept.length && isSep(kept[kept.length - 1])) kept.pop();
  while (kept.length && isSep(kept[0])) kept.shift();
  return kept;
}

/** Groups joined by separators; an empty group (or false/null) leaves no trace. */
export function group(...groups: (MenuEntry[] | false | null | undefined)[]): MenuEntry[] {
  const out: MenuEntry[] = [];
  for (const g of groups) {
    if (!g || g.length === 0) continue;
    if (out.length) out.push(SEP);
    out.push(...g);
  }
  return tidy(out);
}

/** Every action in the list, the submenus' too; `path` says which submenu it is in. */
export function flatten(entries: MenuEntry[], path: string[] = []): { action: MenuAction; path: string[] }[] {
  return entries.flatMap((e) =>
    isAction(e) ? (e.items ? flatten(e.items, [...path, e.label]) : [{ action: e, path }]) : [],
  );
}

export const countActions = (entries: MenuEntry[]) => flatten(entries).length;

/** Search: the actions whose label (or submenu's name) holds every word of the query, best first. */
export function search(entries: MenuEntry[], query: string): { action: MenuAction; path: string[] }[] {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (!words.length) return [];
  const scored: { r: { action: MenuAction; path: string[] }; s: number }[] = [];
  for (const r of flatten(entries)) {
    const label = r.action.label.toLowerCase();
    const full = [...r.path, r.action.label].join(" ").toLowerCase();
    if (!words.every((w) => full.includes(w))) continue;
    scored.push({ r, s: words.every((w) => label.startsWith(w)) ? 0 : words.every((w) => label.includes(w)) ? 1 : 2 });
  }
  return scored.sort((a, b) => a.s - b.s).map((x) => x.r);
}

// ---------------------------------------------------------------- shortcut glyphs

const MOD_ORDER = ["ctrl", "alt", "shift", "cmd"] as const;
const MOD_OF: Record<string, (typeof MOD_ORDER)[number]> = {
  ctrl: "ctrl",
  control: "ctrl",
  alt: "alt",
  option: "alt",
  opt: "alt",
  shift: "shift",
  cmd: "cmd",
  command: "cmd",
  meta: "cmd",
  cmdorctrl: "cmd",
  ctrlorcmd: "cmd",
};
const GLYPH: Record<(typeof MOD_ORDER)[number], string> = { ctrl: "⌃", alt: "⌥", shift: "⇧", cmd: "⌘" };
const KEY_OF: Record<string, string> = {
  delete: "⌫",
  backspace: "⌫",
  del: "⌦",
  enter: "↩",
  return: "↩",
  escape: "Esc",
  esc: "Esc",
  space: "Space",
  tab: "Tab",
  arrowleft: "←",
  arrowright: "→",
  arrowup: "↑",
  arrowdown: "↓",
};

/** "Cmd+Shift+C", "Shift+Option+C", "CtrlOrCmd+Delete" → "⇧⌘C", "⇧⌥C", "⌘⌫" in the Mac's order (⌃⌥⇧⌘). Glyph text passes through. */
export function glyphs(text: string | undefined | null): string {
  const t = (text ?? "").trim();
  if (!t) return "";
  if (KEY_OF[t.toLowerCase()]) return KEY_OF[t.toLowerCase()];
  if (!/[A-Za-z0-9]\+|\+[A-Za-z]/.test(t) || /[⌃⌥⇧⌘]/.test(t)) return t;
  const parts = t.split(/\+(?=.)/);
  const key = parts.pop() ?? "";
  const mods = new Set<(typeof MOD_ORDER)[number]>();
  let rest = "";
  for (const p of parts) {
    const m = MOD_OF[p.trim().toLowerCase()];
    if (m) mods.add(m);
    else rest += p;
  }
  const k = KEY_OF[key.trim().toLowerCase()] ?? (key.length === 1 ? key.toUpperCase() : key);
  return (
    MOD_ORDER.filter((m) => mods.has(m))
      .map((m) => GLYPH[m])
      .join("") +
    rest +
    k
  );
}

/** "⇧⌘C" → ["⇧", "⌘", "C"]: one cap per key; a named key ("Esc", "Space") stays whole. */
export function caps(keys: string): string[] {
  const out: string[] = [];
  let i = 0;
  const chars = [...keys];
  while (i < chars.length && /[⌃⌥⇧⌘]/.test(chars[i])) out.push(chars[i++]);
  const rest = chars.slice(i).join("");
  if (rest) out.push(rest);
  return out;
}

/** The caps with a React key each, stable without the list index: the cap, then which time it repeats ("⌘", "⌘2"). */
export function keyedCaps(keys: string): { cap: string; key: string }[] {
  const seen = new Map<string, number>();
  return caps(keys).map((cap) => {
    const n = (seen.get(cap) ?? 0) + 1;
    seen.set(cap, n);
    return { cap, key: n === 1 ? cap : `${cap}${n}` };
  });
}

/** A row's React key, stable without the list index: an action's id, a heading's text, and for a separator the id
 *  of the next action (tidy never leaves one at the end). */
export function rowKey(rows: MenuEntry[], i: number): string {
  const r = rows[i];
  if (isAction(r)) return r.id;
  if (isHeading(r)) return `h-${r.heading}`;
  const next = rows.slice(i + 1).find(isAction);
  return `sep-before-${next ? next.id : "end"}`;
}

// ---------------------------------------------------------------- moving through rows

/** The next action index from `from` (-1 for none) going `dir`, wrapping; -1 when there is no action. */
export function step(rows: MenuEntry[], from: number, dir: 1 | -1): number {
  const n = rows.length;
  if (!rows.some(isAction)) return -1;
  let i = from;
  for (let k = 0; k < n; k++) {
    i = from === -1 && dir === -1 && k === 0 ? n - 1 : (i + dir + n) % n;
    if (isAction(rows[i])) return i;
  }
  return -1;
}

export const first = (rows: MenuEntry[]) => step(rows, -1, 1);
export const last = (rows: MenuEntry[]) => step(rows, -1, -1);

/** Type-ahead: the buffer "co" lands on the first action (from the current row on, wrapping) whose label starts with it; one repeated letter ("cc") cycles to the next match. */
export function typeAhead(rows: MenuEntry[], buffer: string, from: number): number {
  const b = buffer.toLowerCase();
  if (!b) return -1;
  const cycle = b.length > 1 && [...b].every((c) => c === b[0]);
  const want = cycle ? b[0] : b;
  const n = rows.length;
  const start = from < 0 ? 0 : cycle || b.length === 1 ? from + 1 : from;
  for (let k = 0; k < n; k++) {
    const i = (start + k) % n;
    const r = rows[i];
    if (isAction(r) && r.label.toLowerCase().startsWith(want)) return i;
  }
  return -1;
}

// ---------------------------------------------------------------- where it goes

export type Size = { w: number; h: number };
export type Placed = { left: number; top: number; flipX: boolean; flipY: boolean; maxH: number };

/** A panel at the pointer, flipped to the other side of it at the right and bottom edges, and kept `margin` inside the window. */
export function place(at: { x: number; y: number }, size: Size, view: Size, margin = 8): Placed {
  const maxH = Math.max(120, view.h - margin * 2);
  const h = Math.min(size.h, maxH);
  let left = at.x;
  let flipX = false;
  if (left + size.w > view.w - margin) {
    left = at.x - size.w;
    flipX = true;
  }
  let top = at.y;
  let flipY = false;
  if (top + h > view.h - margin) {
    top = at.y - h;
    flipY = true;
  }
  return {
    left: Math.max(margin, Math.min(left, view.w - size.w - margin)),
    top: Math.max(margin, Math.min(top, view.h - h - margin)),
    flipX,
    flipY,
    maxH,
  };
}

/** A submenu beside its row: to the right (a few px over the parent's edge), to the left when that won't fit; its first row lines up with the parent's. */
export function placeSub(
  row: { left: number; right: number; top: number; bottom: number },
  size: Size,
  view: Size,
  margin = 8,
  pad = 6,
): Placed {
  const maxH = Math.max(120, view.h - margin * 2);
  const h = Math.min(size.h, maxH);
  let left = row.right - 4;
  let flipX = false;
  if (left + size.w > view.w - margin) {
    left = row.left - size.w + 4;
    flipX = true;
  }
  const top = Math.max(margin, Math.min(row.top - pad, view.h - h - margin));
  return { left: Math.max(margin, left), top, flipX, flipY: false, maxH };
}
