// The board's pure edits (PRD 0065): make an item, move it (its frame's members and the arrows bound to it
// follow), resize it, put it in a frame, find what is under a point. Every function takes a canvas and returns
// a new one, never changes its input, so the store can undo and replay them and node tests can run them.

import {
  type Box,
  bounds,
  type Canvas,
  type FrameItem,
  type ImageItem,
  type Item,
  type NoteItem,
  newId,
  type ShapeItem,
  type ShapeKind,
  type Target,
  type Tone,
} from "./schema.ts";

export const MIN_SIZE = 40;
export const GRID = 8;
export const NOTE_SIZE = { w: 220, h: 150 };
export const FRAME_SIZE = { w: 480, h: 320 };

export const snap = (n: number, g = GRID) => Math.round(n / g) * g;

type Pt = { x: number; y: number };

export function makeNote(at: Pt, text = "", tone: Tone = "amber"): NoteItem {
  return { item_id: newId("itm"), type: "note", title: "", text, tone, x: at.x, y: at.y, ...NOTE_SIZE };
}

export function makeFrame(at: Pt, title = "Section"): FrameItem {
  return { item_id: newId("itm"), type: "frame", title, x: at.x, y: at.y, ...FRAME_SIZE };
}

/** A shape from a box. An arrow or a line runs corner to corner of it, its points relative to the box. */
export function makeShape(shape: ShapeKind, b: Box, extra: Partial<ShapeItem> = {}): ShapeItem {
  const linear = shape === "arrow" || shape === "line";
  const w = linear ? b.w : Math.max(MIN_SIZE, b.w);
  const h = linear ? b.h : Math.max(MIN_SIZE, b.h);
  return {
    item_id: newId("itm"),
    type: "shape",
    shape,
    title: "",
    x: b.x,
    y: b.y,
    w,
    h,
    ...(linear
      ? {
          points: [
            [0, 0],
            [b.w, b.h],
          ] as [number, number][],
          ...(shape === "arrow" ? { arrowheads: { end: "arrow" } } : {}),
        }
      : {}),
    ...extra,
  };
}

export function makeImage(
  src: string,
  size: { w: number; h: number },
  at: Pt,
  natural?: { w: number; h: number },
  title = "",
): ImageItem {
  return {
    item_id: newId("itm"),
    type: "image",
    title,
    src,
    x: at.x,
    y: at.y,
    w: size.w,
    h: size.h,
    ...(natural ? { natural_w: natural.w, natural_h: natural.h } : {}),
  };
}

export const addItems = (c: Canvas, items: Item[]): Canvas => ({ ...c, items: [...c.items, ...items] });

// ---------------------------------------------------------------- hit testing

const inside = (b: Box, p: Pt) => p.x >= b.x && p.x <= b.x + b.w && p.y >= b.y && p.y <= b.y + b.h;

/** Frames behind, everything else in file order: the order things are painted, back to front. */
const order = (items: Item[]): Item[] => [
  ...items.filter((i) => i.type === "frame"),
  ...items.filter((i) => i.type !== "frame"),
];

/** Distance from a point to the segment a-b. */
function distToSegment(p: Pt, a: Pt, b: Pt): number {
  const dx = b.x - a.x,
    dy = b.y - a.y;
  const len2 = dx * dx + dy * dy;
  const t = len2 === 0 ? 0 : Math.max(0, Math.min(1, ((p.x - a.x) * dx + (p.y - a.y) * dy) / len2));
  return Math.hypot(p.x - (a.x + t * dx), p.y - (a.y + t * dy));
}

/** The endpoints of an arrow or a line in board coordinates. */
export function endpoints(s: ShapeItem): [Pt, Pt] | null {
  const pts = s.points;
  if (!pts || pts.length < 2) return null;
  const a = pts[0],
    b = pts[pts.length - 1];
  return [
    { x: s.x + a[0], y: s.y + a[1] },
    { x: s.x + b[0], y: s.y + b[1] },
  ];
}

/** The topmost item under a board point; an arrow counts within `reach` of its line (in board units). */
export function hitTest(items: Item[], p: Pt, reach = 8): Item | null {
  const painted = order(items);
  for (let i = painted.length - 1; i >= 0; i--) {
    const it = painted[i];
    if (it.type === "shape" && (it.shape === "arrow" || it.shape === "line")) {
      const e = endpoints(it);
      if (e && distToSegment(p, e[0], e[1]) <= reach) return it;
      continue;
    }
    if (inside(it, p)) return it;
  }
  return null;
}

export const normRect = (a: Pt, b: Pt): Box => ({
  x: Math.min(a.x, b.x),
  y: Math.min(a.y, b.y),
  w: Math.abs(b.x - a.x),
  h: Math.abs(b.y - a.y),
});

const touches = (a: Box, b: Box) => a.x < b.x + b.w && a.x + a.w > b.x && a.y < b.y + b.h && a.y + a.h > b.y;

/** Ids of the items a rubber-band rectangle touches. */
export const itemsInRect = (items: Item[], r: Box): string[] =>
  items.filter((i) => touches(i, r)).map((i) => i.item_id);

// ---------------------------------------------------------------- frames

/** The ids of the items that belong to a frame or diagram (their `frame_id` names it). */
export const membersOf = (c: Canvas, id: string): string[] =>
  c.items.filter((i) => i.frame_id === id).map((i) => i.item_id);

/** The smallest frame whose box holds the centre of `it`, never `it` itself. */
export function frameAt(c: Canvas, it: Box & { item_id: string }): string | undefined {
  const centre = { x: it.x + it.w / 2, y: it.y + it.h / 2 };
  let best: FrameItem | undefined;
  for (const f of c.items) {
    if (f.type !== "frame" || f.item_id === it.item_id || !inside(f, centre)) continue;
    if (!best || f.w * f.h < best.w * best.h) best = f;
  }
  return best?.item_id;
}

/** After a move: each moved item that is not a frame joins the frame it now sits in, or leaves its old one. */
export function adoptFrames(c: Canvas, ids: string[]): Canvas {
  const moved = new Set(ids);
  return {
    ...c,
    items: c.items.map((i) => {
      if (!moved.has(i.item_id) || i.type === "frame") return i;
      const f = frameAt(c, i);
      if (f === i.frame_id) return i;
      const { frame_id: _old, ...rest } = i;
      return (f ? { ...rest, frame_id: f } : rest) as Item;
    }),
  };
}

// ---------------------------------------------------------------- arrows bound to items

/** Where the line from the centre of `b` to `toward` leaves the box. */
export function edgePoint(b: Box, toward: Pt): Pt {
  const c = { x: b.x + b.w / 2, y: b.y + b.h / 2 };
  const dx = toward.x - c.x,
    dy = toward.y - c.y;
  if (dx === 0 && dy === 0) return c;
  const k = Math.min(dx === 0 ? Infinity : b.w / 2 / Math.abs(dx), dy === 0 ? Infinity : b.h / 2 / Math.abs(dy));
  return { x: c.x + dx * k, y: c.y + dy * k };
}

const centreOf = (b: Box): Pt => ({ x: b.x + b.w / 2, y: b.y + b.h / 2 });

/** Re-draws every arrow bound to an item in `moved` so it still runs from one to the other. */
export function followBound(c: Canvas, moved: Set<string>): Canvas {
  const by = new Map(c.items.map((i) => [i.item_id, i]));
  let changed = false;
  const items = c.items.map((it) => {
    if (it.type !== "shape" || it.shape !== "arrow" || moved.has(it.item_id)) return it;
    const s = it.start?.item_id ? by.get(it.start.item_id) : undefined;
    const e = it.end?.item_id ? by.get(it.end.item_id) : undefined;
    if (!(s && moved.has(s.item_id)) && !(e && moved.has(e.item_id))) return it;
    const ends = endpoints(it);
    if (!ends) return it;
    // A bound end sits on its item's edge, facing the other end; a free end stays where it was.
    const aimB = e ? centreOf(e) : ends[1];
    const aimA = s ? centreOf(s) : ends[0];
    const p0 = s ? edgePoint(s, aimB) : ends[0];
    const p1 = e ? edgePoint(e, aimA) : ends[1];
    const x = Math.min(p0.x, p1.x),
      y = Math.min(p0.y, p1.y);
    changed = true;
    return {
      ...it,
      x,
      y,
      w: Math.abs(p1.x - p0.x),
      h: Math.abs(p1.y - p0.y),
      points: [
        [p0.x - x, p0.y - y],
        [p1.x - x, p1.y - y],
      ] as [number, number][],
    };
  });
  return changed ? { ...c, items } : c;
}

/** An arrow from one item to another, bound to both, so it follows them. */
export function connect(c: Canvas, from: string, to: string): Canvas {
  const a = c.items.find((i) => i.item_id === from),
    b = c.items.find((i) => i.item_id === to);
  if (!a || !b || a === b) return c;
  const p0 = edgePoint(a, centreOf(b)),
    p1 = edgePoint(b, centreOf(a));
  const x = Math.min(p0.x, p1.x),
    y = Math.min(p0.y, p1.y);
  const start: Target = { item_id: from },
    end: Target = { item_id: to };
  const arrow = makeShape(
    "arrow",
    { x, y, w: Math.abs(p1.x - p0.x), h: Math.abs(p1.y - p0.y) },
    {
      points: [
        [p0.x - x, p0.y - y],
        [p1.x - x, p1.y - y],
      ],
      start,
      end,
    },
  );
  return addItems(c, [arrow]);
}

// ---------------------------------------------------------------- move, resize, edit

/** Moves items by (dx, dy); members of a moved frame go with it, and arrows bound to any of them follow. */
export function moveItems(c: Canvas, ids: string[], dx: number, dy: number): Canvas {
  if (!dx && !dy) return c;
  const moving = new Set(ids);
  for (const id of ids) for (const m of membersOf(c, id)) moving.add(m);
  const items = c.items.map((i) => (moving.has(i.item_id) ? ({ ...i, x: i.x + dx, y: i.y + dy } as Item) : i));
  return followBound(adoptFrames({ ...c, items }, ids), moving);
}

export function resizeItem(c: Canvas, id: string, b: Box): Canvas {
  const w = Math.max(MIN_SIZE, b.w),
    h = Math.max(MIN_SIZE, b.h);
  const items = c.items.map((i) => (i.item_id === id ? ({ ...i, x: b.x, y: b.y, w, h } as Item) : i));
  return followBound({ ...c, items }, new Set([id]));
}

export function setText(c: Canvas, id: string, text: string): Canvas {
  return {
    ...c,
    items: c.items.map((i) =>
      i.item_id === id && (i.type === "note" || i.type === "frame" || i.type === "shape") ? { ...i, text } : i,
    ),
  };
}

export function setTitle(c: Canvas, id: string, title: string): Canvas {
  return { ...c, items: c.items.map((i) => (i.item_id === id ? { ...i, title } : i)) };
}

export function setTone(c: Canvas, ids: string[], tone: Tone): Canvas {
  const set = new Set(ids);
  return {
    ...c,
    items: c.items.map((i) => (set.has(i.item_id) && (i.type === "note" || i.type === "frame") ? { ...i, tone } : i)),
  };
}

/** Brings items to the front (the end of the list) or sends them to the back. */
export function reorder(c: Canvas, ids: string[], to: "front" | "back"): Canvas {
  const set = new Set(ids);
  const mine = c.items.filter((i) => set.has(i.item_id));
  const rest = c.items.filter((i) => !set.has(i.item_id));
  return { ...c, items: to === "front" ? [...rest, ...mine] : [...mine, ...rest] };
}

/** Copies of items, offset, with new ids. Arrow bindings to items that were not copied are dropped. */
export function duplicate(c: Canvas, ids: string[], offset = 24): { canvas: Canvas; ids: string[] } {
  const src = c.items.filter((i) => ids.includes(i.item_id));
  const map = new Map(src.map((i) => [i.item_id, newId("itm")]));
  const copies = src.map((i) => {
    const copy = {
      ...structuredClone(i),
      item_id: map.get(i.item_id) as string,
      x: i.x + offset,
      y: i.y + offset,
    } as Item;
    if (copy.frame_id && map.has(copy.frame_id)) copy.frame_id = map.get(copy.frame_id);
    if (copy.type === "shape") {
      if (copy.start && !map.has(copy.start.item_id)) delete copy.start;
      else if (copy.start) copy.start = { ...copy.start, item_id: map.get(copy.start.item_id) as string };
      if (copy.end && !map.has(copy.end.item_id)) delete copy.end;
      else if (copy.end) copy.end = { ...copy.end, item_id: map.get(copy.end.item_id) as string };
    }
    return copy;
  });
  return { canvas: addItems(c, copies), ids: copies.map((i) => i.item_id) };
}

/** The box around the items with these ids (all of them when `ids` is empty). */
export const boundsOf = (c: Canvas, ids: string[] = []): Box | null =>
  bounds(ids.length ? c.items.filter((i) => ids.includes(i.item_id)) : c.items);

// ---------------------------------------------------------------- handles

export type Handle = "nw" | "n" | "ne" | "e" | "se" | "s" | "sw" | "w";
export const HANDLES: readonly Handle[] = ["nw", "n", "ne", "e", "se", "s", "sw", "w"];

/**
 * The box after a handle is dragged by (dx, dy) board units. The opposite edges stay where they are and the box never
 * gets smaller than `min`, so a drag past the far edge stops there instead of turning the box inside out.
 */
export function resizeBox(b: Box, h: Handle, dx: number, dy: number, min = MIN_SIZE): Box {
  let left = b.x,
    top = b.y,
    right = b.x + b.w,
    bottom = b.y + b.h;
  if (h.includes("w")) left = Math.min(left + dx, right - min);
  if (h.includes("e")) right = Math.max(right + dx, left + min);
  if (h.includes("n")) top = Math.min(top + dy, bottom - min);
  if (h.includes("s")) bottom = Math.max(bottom + dy, top + min);
  return { x: left, y: top, w: right - left, h: bottom - top };
}

/** Moves one end of an arrow or a line to a board point, free (a bound end lets go of its item). */
export function moveEnd(c: Canvas, id: string, which: "start" | "end", to: Pt): Canvas {
  const it = c.items.find((i) => i.item_id === id);
  if (!it || it.type !== "shape") return c;
  const ends = endpoints(it);
  if (!ends) return c;
  const p0 = which === "start" ? to : ends[0],
    p1 = which === "end" ? to : ends[1];
  const x = Math.min(p0.x, p1.x),
    y = Math.min(p0.y, p1.y);
  const next: ShapeItem = {
    ...it,
    x,
    y,
    w: Math.abs(p1.x - p0.x),
    h: Math.abs(p1.y - p0.y),
    points: [
      [p0.x - x, p0.y - y],
      [p1.x - x, p1.y - y],
    ],
  };
  delete next[which];
  return { ...c, items: c.items.map((i) => (i.item_id === id ? next : i)) };
}

/** The id of the one selected item that has text to edit (a note, frame or shape), or null: what Enter opens. */
export function editTarget(c: Canvas, sel: readonly string[]): string | null {
  if (sel.length !== 1) return null;
  const it = c.items.find((i) => i.item_id === sel[0]);
  return it && (it.type === "note" || it.type === "frame" || it.type === "shape") ? it.item_id : null;
}

/** The accessible name of an item: its type and a short title, never a note's body text. */
export function itemLabel(it: Item): string {
  const t = it.title.trim().slice(0, 60);
  return t ? `${it.type}: ${t}` : it.type;
}

/** The DOM id an item is given, so the board host can name the selected one in aria-activedescendant. */
export const itemDomId = (id: string) => `cv-item-${id}`;
