import assert from "node:assert/strict";
import { test } from "node:test";
import {
  caps,
  countActions,
  first,
  glyphs,
  group,
  heading,
  isAction,
  last,
  type MenuEntry,
  place,
  placeSub,
  SEP,
  search,
  step,
  tidy,
  typeAhead,
} from "../src/frame/menu-core.ts";

const a = (id: string, label = id, more: object = {}): MenuEntry => ({ id, label, ...more });

test("group puts one separator between groups and none at the edges or doubled", () => {
  const g = group([a("x")], [], [a("y"), a("z")], false, [a("w")]);
  assert.deepEqual(
    g.map((e) => ("sep" in e ? "-" : (e as { id: string }).id)),
    ["x", "-", "y", "z", "-", "w"],
  );
  assert.deepEqual(tidy([SEP, SEP, a("p"), SEP, SEP, SEP]).length, 1);
});

test("glyphs turns the engine's words into Mac glyphs in the order ⌃⌥⇧⌘", () => {
  assert.equal(glyphs("Cmd+V"), "⌘V");
  assert.equal(glyphs("Shift+Option+C"), "⌥⇧C");
  assert.equal(glyphs("Cmd+Shift+C"), "⇧⌘C");
  assert.equal(glyphs("Delete"), "⌫");
  assert.deepEqual(caps("⇧⌘C"), ["⇧", "⌘", "C"]);
});

test("arrows skip separators, headings and wrap; Home and End land on the ends; disabled rows stay reachable", () => {
  const rows = [heading("H"), a("one"), SEP, a("two", "two", { disabled: "why" }), a("three")];
  assert.equal(first(rows), 1);
  assert.equal(last(rows), 4);
  assert.equal(step(rows, 1, 1), 3);
  assert.equal(step(rows, 4, 1), 1);
  assert.equal(step(rows, 1, -1), 4);
});

test("type-ahead finds the next row starting with the letters, from the one after the current", () => {
  const rows = [a("c", "Copy"), a("p", "Paste"), a("cu", "Cut"), a("s", "Select all")];
  assert.equal(typeAhead(rows, "p", -1), 1);
  assert.equal(typeAhead(rows, "c", 0), 2);
  assert.equal(typeAhead(rows, "sel", 0), 3);
  assert.equal(typeAhead(rows, "zz", 0), -1);
});

test("search looks into submenus and names where a match lives", () => {
  const tree: MenuEntry[] = [
    a("copy", "Copy"),
    a("as", "Copy as", { items: [a("png", "PNG image"), a("svg", "SVG")] }),
  ];
  assert.equal(countActions(tree), 3);
  const r = search(tree, "png");
  assert.equal(r.length, 1);
  assert.deepEqual(r[0].path, ["Copy as"]);
  assert.ok(isAction(r[0].action));
});

test("place flips at the right and bottom edge and stays inside the window", () => {
  const view = { w: 1000, h: 800 };
  const size = { w: 220, h: 300 };
  const o = place({ x: 100, y: 100 }, size, view);
  assert.deepEqual([o.left, o.top, o.flipX, o.flipY], [100, 100, false, false]);
  const f = place({ x: 950, y: 780 }, size, view);
  assert.equal(f.flipX, true);
  assert.equal(f.flipY, true);
  assert.ok(f.left >= 8 && f.left + size.w <= view.w);
  assert.ok(f.top >= 8 && f.top + size.h <= view.h);
  const tall = place({ x: 10, y: 10 }, { w: 220, h: 2000 }, view);
  assert.ok(tall.maxH <= view.h - 16);
});

test("a submenu opens beside its row and flips to the left at the edge", () => {
  const view = { w: 1000, h: 800 };
  const row = { left: 700, right: 920, top: 200, bottom: 228 };
  const s = placeSub(row, { w: 200, h: 120 }, view);
  assert.equal(s.flipX, true);
  assert.ok(s.left + 200 <= row.left + 8);
  const n = placeSub({ left: 100, right: 320, top: 200, bottom: 228 }, { w: 200, h: 120 }, view);
  assert.equal(n.flipX, false);
  assert.ok(n.left >= 320 - 8);
});
