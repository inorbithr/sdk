// The layouts and the box order (src/frame/arrange.ts): what each ready layout places, the grid drag, the way back,
// the Arrange menu's moves, and reading what was kept.
import assert from "node:assert/strict";
import { test } from "node:test";
import {
  applyLayout,
  applySpan,
  arrangeBox,
  arranged,
  back,
  describe,
  dockClosed,
  FULL,
  initial,
  layoutOff,
  moveBox,
  moveCell,
  parseArrangement,
  resetAll,
  restore,
  spanPlace,
} from "../src/frame/arrange.ts";
import { keysOf, LAYOUT_SHORTCUTS, shortcutOf } from "../src/frame/shortcuts.ts";

const press = (key: string, code = key) => ({
  key,
  code,
  metaKey: false,
  ctrlKey: true,
  altKey: true,
  shiftKey: false,
});

test("the start is the page filling the area, page then dock, nothing to go back to", () => {
  const a = initial();
  assert.deepEqual(a.page, FULL);
  assert.equal(a.dock, null);
  assert.equal(a.before, null);
  assert.equal(arranged(a), false);
  assert.equal(describe(a), "");
});

test("the single layouts place the page on the 6 by 4 grid", () => {
  const at = (id: Parameters<typeof applyLayout>[1]) => applyLayout(initial(), id).page;
  assert.deepEqual(at("left"), { c0: 0, r0: 0, c1: 3, r1: 4 });
  assert.deepEqual(at("right"), { c0: 3, r0: 0, c1: 6, r1: 4 });
  assert.deepEqual(at("top"), { c0: 0, r0: 0, c1: 6, r1: 2 });
  assert.deepEqual(at("bottom"), { c0: 0, r0: 2, c1: 6, r1: 4 });
  const start = initial();
  assert.equal(applyLayout(start, "full"), start, "Full on a full page changes nothing");
  assert.equal(describe(applyLayout(initial(), "top")), "Top half");
});

test("a pair layout gives the first box in the order the first share", () => {
  const side = applyLayout(initial(), "side-by-side");
  assert.deepEqual(side.page, { c0: 0, r0: 0, c1: 3, r1: 4 });
  assert.deepEqual(side.dock, { c0: 3, r0: 0, c1: 6, r1: 4 });
  const two = applyLayout(initial(), "two-thirds");
  assert.equal(two.page.c1 - two.page.c0, 4);
  assert.equal(two.dock?.c1 !== undefined && two.dock.c1 - two.dock.c0, 2);
  const one = applyLayout(initial(), "one-third");
  assert.equal(one.page.c1 - one.page.c0, 2);
  // With the dock first, the dock gets the two thirds.
  const flipped = applyLayout(arrangeBox(initial(), "page", "end"), "two-thirds");
  assert.equal(flipped.dock && flipped.dock.c1 - flipped.dock.c0, 4);
  assert.equal(flipped.page.c1 - flipped.page.c0, 2);
  assert.equal(describe(two), "Two thirds and one third");
  assert.equal(describe(one), "One third and two thirds");
});

test("a grid drag is the box between two cells, either way round, never under 2 by 2, never off the grid", () => {
  assert.deepEqual(spanPlace({ col: 1, row: 0 }, { col: 4, row: 2 }), { c0: 1, r0: 0, c1: 5, r1: 3 });
  assert.deepEqual(spanPlace({ col: 4, row: 2 }, { col: 1, row: 0 }), { c0: 1, r0: 0, c1: 5, r1: 3 });
  assert.deepEqual(spanPlace({ col: 2, row: 1 }, { col: 2, row: 1 }), { c0: 2, r0: 1, c1: 4, r1: 3 });
  assert.deepEqual(spanPlace({ col: 5, row: 3 }, { col: 5, row: 3 }), { c0: 4, r0: 2, c1: 6, r1: 4 });
  assert.deepEqual(spanPlace({ col: -3, row: 99 }, { col: 99, row: -3 }), FULL);
  assert.equal(describe(applySpan(initial(), { c0: 1, r0: 0, c1: 5, r1: 3 })), "Grid 4 by 3");
});

test("Back to before returns to the arrangement before the last change, once", () => {
  const a = applyLayout(initial(), "left");
  assert.deepEqual(a.before?.page, FULL);
  const b = applyLayout(a, "bottom");
  assert.deepEqual(b.before?.page, a.page);
  const c = back(b);
  assert.deepEqual(c.page, a.page);
  assert.equal(c.before, null);
  assert.equal(back(c), c, "nothing is left to go back to");
  assert.equal(applyLayout(a, "left"), a, "the same layout again changes nothing and keeps the way back");
});

test("closing the dock ends a pair layout; a single layout stays", () => {
  const pair = applyLayout(initial(), "side-by-side");
  const closed = dockClosed(pair);
  assert.deepEqual(closed.page, FULL);
  assert.equal(closed.dock, null);
  assert.deepEqual(back(closed).dock, pair.dock, "the pair can be got back");
  const single = applyLayout(initial(), "left");
  assert.equal(dockClosed(single), single);
});

test("the Arrange menu moves a box one step or to an end, and a box at its end does not move", () => {
  assert.deepEqual(moveBox(["page", "dock"], "page", "right"), ["dock", "page"]);
  assert.deepEqual(moveBox(["page", "dock"], "dock", "start"), ["dock", "page"]);
  assert.deepEqual(moveBox(["dock", "page"], "dock", "end"), ["page", "dock"]);
  const order = ["page", "dock"] as const;
  assert.equal(moveBox([...order], "page", "left").join(), "page,dock");
  const same = [...order];
  assert.equal(moveBox(same, "page", "start"), same);
  assert.equal(moveBox(same, "dock", "end"), same);
});

test("moving a box in a pair layout swaps the cells with it", () => {
  const pair = applyLayout(initial(), "two-thirds");
  const swapped = arrangeBox(pair, "page", "end");
  assert.deepEqual(swapped.order, ["dock", "page"]);
  assert.deepEqual(swapped.page, pair.dock);
  assert.deepEqual(swapped.dock, pair.page);
  assert.equal(arranged(swapped), true);
});

test("Reset the arrangement puts the order, the places and the way back as at the start", () => {
  const messy = arrangeBox(applyLayout(initial(), "right"), "page", "end");
  assert.equal(arranged(messy), true);
  const fresh = resetAll(messy);
  assert.deepEqual(fresh, initial());
  const start = initial();
  assert.equal(resetAll(start), start);
});

test("a stored arrangement is made whole: junk falls back to the start", () => {
  assert.deepEqual(parseArrangement(null), initial());
  assert.deepEqual(parseArrangement("left"), initial());
  assert.deepEqual(parseArrangement({ order: ["dock", "dock"], page: { c0: 0, r0: 0, c1: 9, r1: 4 } }), initial());
  // Too small to keep the header reachable, or off the grid: not accepted.
  assert.deepEqual(parseArrangement({ page: { c0: 0, r0: 0, c1: 1, r1: 4 } }).page, FULL);
  assert.deepEqual(parseArrangement({ page: { c0: 0, r0: 0, c1: 3.5, r1: 4 } }).page, FULL);
  const kept = applyLayout(arrangeBox(initial(), "dock", "start"), "bottom");
  assert.deepEqual(parseArrangement(JSON.parse(JSON.stringify(kept))), kept);
});

test("the layout shortcuts are the Mac's: ⌃⌥ with an arrow, Return, G and Z", () => {
  assert.equal(LAYOUT_SHORTCUTS.length, 7);
  assert.equal(shortcutOf(press("ArrowLeft"))?.id, "layout-left");
  assert.equal(shortcutOf(press("ArrowDown"))?.id, "layout-bottom");
  assert.equal(shortcutOf(press("Enter"))?.id, "layout-full");
  assert.equal(shortcutOf(press("g", "KeyG"))?.id, "layout-grid");
  assert.equal(shortcutOf(press("z", "KeyZ"))?.id, "layout-back");
  assert.equal(keysOf("layout-full"), "⌃⌥↩");
  assert.equal(keysOf("layout-grid"), "⌃⌥G");
  // Without ⌥ it is not ours.
  assert.equal(shortcutOf({ ...press("ArrowLeft"), altKey: false }), undefined);
});

test("the grid picker's cursor moves by arrows and stops at the edge", () => {
  assert.deepEqual(moveCell({ col: 0, row: 0 }, "ArrowLeft"), { col: 0, row: 0 });
  assert.deepEqual(moveCell({ col: 5, row: 3 }, "ArrowRight"), { col: 5, row: 3 });
  assert.deepEqual(moveCell({ col: 2, row: 1 }, "ArrowDown"), { col: 2, row: 2 });
  assert.deepEqual(moveCell({ col: 2, row: 1 }, "x"), { col: 2, row: 1 });
});

test("the way-back chip puts the page back to full, or only the boxes back in order", () => {
  const moved = applyLayout(initial(), "right");
  assert.deepEqual(restore(moved).page, FULL);
  const swapped = arrangeBox(initial(), "page", "end");
  assert.deepEqual(restore(swapped).order, ["page", "dock"]);
  assert.equal(layoutOff(swapped), false);
});
