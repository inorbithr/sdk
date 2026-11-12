// The board's camera (PRD 0065): pan and zoom as one transform, zoom about a point, zoom to fit,
// and which items are on screen (so a board of 500 items draws only what is visible). Pure, so
// it is tested on its own (tests/canvas-layout.test.ts).

import type { Box } from "./schema.ts";

/** Screen = board × zoom + (x, y). */
export type View = { x: number; y: number; zoom: number };
export const MIN_ZOOM = 0.08;
export const MAX_ZOOM = 3;
export const clampZoom = (z: number) => Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, z));

export const toBoard = (v: View, sx: number, sy: number) => ({ x: (sx - v.x) / v.zoom, y: (sy - v.y) / v.zoom });
export const toScreen = (v: View, bx: number, by: number) => ({ x: bx * v.zoom + v.x, y: by * v.zoom + v.y });

/** Zooms by `factor` keeping the board point under (sx, sy) where it is. */
export function zoomAt(v: View, sx: number, sy: number, factor: number): View {
  const zoom = clampZoom(v.zoom * factor);
  const k = zoom / v.zoom;
  return { zoom, x: sx - (sx - v.x) * k, y: sy - (sy - v.y) * k };
}

/** The view that shows `b` whole inside a `w`×`h` screen, with a margin, never above 100% unless asked. */
export function fit(b: Box | null, w: number, h: number, margin = 48, maxZoom = 1): View {
  if (!b || w <= 0 || h <= 0) return { x: 0, y: 0, zoom: 1 };
  const zoom = clampZoom(Math.min((w - margin * 2) / Math.max(1, b.w), (h - margin * 2) / Math.max(1, b.h), maxZoom));
  return { zoom, x: (w - b.w * zoom) / 2 - b.x * zoom, y: (h - b.h * zoom) / 2 - b.y * zoom };
}

/** The part of the board the screen shows, grown by `pad` screen pixels each side. */
export function visibleRect(v: View, w: number, h: number, pad = 200): Box {
  const a = toBoard(v, -pad, -pad);
  return { x: a.x, y: a.y, w: (w + pad * 2) / v.zoom, h: (h + pad * 2) / v.zoom };
}

export const overlaps = (a: Box, b: Box) => a.x < b.x + b.w && a.x + a.w > b.x && a.y < b.y + b.h && a.y + a.h > b.y;

/** How much detail to draw: full above 45%, a summary down to 18%, a coloured block below. */
export type Detail = "full" | "summary" | "block";
export const detailAt = (zoom: number): Detail => (zoom >= 0.45 ? "full" : zoom >= 0.18 ? "summary" : "block");

/** Ids of the items on screen, cheap enough for every frame at 500 items (one pass, no allocation per item). */
export function onScreen<T extends Box & { item_id: string }>(items: T[], v: View, w: number, h: number): Set<string> {
  const r = visibleRect(v, w, h);
  const out = new Set<string>();
  for (const i of items) if (overlaps(i, r)) out.add(i.item_id);
  return out;
}

/** A view moved so that `b` is centred, keeping the zoom (or zooming out just enough to fit it). */
export function centreOn(v: View, b: Box, w: number, h: number): View {
  const zoom = Math.min(v.zoom, clampZoom(Math.min((w - 96) / Math.max(1, b.w), (h - 96) / Math.max(1, b.h))));
  return { zoom, x: w / 2 - (b.x + b.w / 2) * zoom, y: h / 2 - (b.y + b.h / 2) * zoom };
}

/** Where to put a new item: the middle of what is on screen, nudged so it doesn't sit on top of another. */
export function freeSpot(
  v: View,
  w: number,
  h: number,
  size: { w: number; h: number },
  taken: Box[],
): { x: number; y: number } {
  const c = toBoard(v, w / 2, h / 2);
  let x = Math.round(c.x - size.w / 2);
  let y = Math.round(c.y - size.h / 2);
  for (let n = 0; n < 40 && taken.some((t) => overlaps({ x, y, ...size }, t)); n++) {
    x += 32;
    y += 32;
  }
  return { x, y };
}

/** A link between two boxes: from the side of one that faces the other, as a gentle curve. */
export function linkPath(a: Box, b: Box): { d: string; mid: { x: number; y: number } } {
  const ac = { x: a.x + a.w / 2, y: a.y + a.h / 2 };
  const bc = { x: b.x + b.w / 2, y: b.y + b.h / 2 };
  const dx = bc.x - ac.x,
    dy = bc.y - ac.y;
  const horizontal = Math.abs(dx) / (a.w + b.w) >= Math.abs(dy) / (a.h + b.h);
  const p0 = horizontal ? { x: dx >= 0 ? a.x + a.w : a.x, y: ac.y } : { x: ac.x, y: dy >= 0 ? a.y + a.h : a.y };
  const p1 = horizontal ? { x: dx >= 0 ? b.x : b.x + b.w, y: bc.y } : { x: bc.x, y: dy >= 0 ? b.y : b.y + b.h };
  const k = Math.max(40, (horizontal ? Math.abs(p1.x - p0.x) : Math.abs(p1.y - p0.y)) / 2);
  const c0 = horizontal ? { x: p0.x + Math.sign(dx || 1) * k, y: p0.y } : { x: p0.x, y: p0.y + Math.sign(dy || 1) * k };
  const c1 = horizontal ? { x: p1.x - Math.sign(dx || 1) * k, y: p1.y } : { x: p1.x, y: p1.y - Math.sign(dy || 1) * k };
  const mid = { x: (p0.x + 3 * c0.x + 3 * c1.x + p1.x) / 8, y: (p0.y + 3 * c0.y + 3 * c1.y + p1.y) / 8 };
  return { d: `M${p0.x},${p0.y} C${c0.x},${c0.y} ${c1.x},${c1.y} ${p1.x},${p1.y}`, mid };
}
