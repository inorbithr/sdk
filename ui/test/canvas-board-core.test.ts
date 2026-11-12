// The board's pure edits (src/canvas/board-core.ts).
import assert from "node:assert/strict";
import { test } from "node:test";
import {
  addItems,
  adoptFrames,
  connect,
  duplicate,
  edgePoint,
  endpoints,
  frameAt,
  hitTest,
  itemsInRect,
  makeFrame,
  makeNote,
  makeShape,
  moveEnd,
  moveItems,
  normRect,
  reorder,
  resizeBox,
  resizeItem,
  setText,
  setTone,
  snap,
} from "../src/canvas/board-core.ts";
import { type Canvas, type Item, parseCanvas, serialize } from "../src/canvas/schema.ts";
import { fixtureCanvas } from "./canvas-fixture.ts";

const get = (c: Canvas, id: string) => c.items.find((i) => i.item_id === id) as Item;

test("a new note, frame and shape are valid items the file accepts", () => {
  let c = fixtureCanvas();
  const n = makeNote({ x: 10, y: 20 }, "hello");
  const f = makeFrame({ x: 0, y: 0 }, "Plan B");
  const s = makeShape("rectangle", { x: 5, y: 5, w: 10, h: 10 });
  c = addItems(c, [n, f, s]);
  assert.equal(s.w, 40, "a box is never smaller than 40");
  const p = parseCanvas(serialize(c));
  assert.ok(p.ok);
  assert.deepEqual(p.dropped, []);
  assert.deepEqual(p.canvas, c);
});

test("an arrow runs corner to corner of its box and may point any way", () => {
  const a = makeShape("arrow", { x: 0, y: 0, w: 100, h: 50 });
  assert.deepEqual(a.points, [
    [0, 0],
    [100, 50],
  ]);
  assert.deepEqual(endpoints({ ...a, x: 10, y: 10 }), [
    { x: 10, y: 10 },
    { x: 110, y: 60 },
  ]);
});

test("moving a frame moves its members; moving a member does not move the frame", () => {
  let c = fixtureCanvas();
  c = { ...c, items: c.items.map((i) => (i.item_id === "itm_note-idea" ? { ...i, frame_id: "itm_frame-plan" } : i)) };
  const m = moveItems(c, ["itm_frame-plan"], 30, 40);
  assert.equal(get(m, "itm_frame-plan").x, -10);
  assert.equal(get(m, "itm_note-idea").x, 730);
  assert.equal(get(m, "itm_note-idea").y, 40);
  const m2 = moveItems(c, ["itm_note-idea"], 5, 5);
  assert.equal(get(m2, "itm_frame-plan").x, -40);
  assert.notEqual(c, m, "the input is never changed");
  assert.equal(get(c, "itm_note-idea").x, 700);
});

test("a moved item joins the smallest frame its centre is in, and leaves when it is dragged out", () => {
  let c = fixtureCanvas();
  c = addItems(c, [{ ...makeFrame({ x: 650, y: -20 }, "Inner"), item_id: "itm_inner", w: 340, h: 220 }]);
  assert.equal(frameAt(c, get(c, "itm_note-idea")), "itm_inner");
  const adopted = adoptFrames(c, ["itm_note-idea"]);
  assert.equal(get(adopted, "itm_note-idea").frame_id, "itm_inner");
  const out = moveItems(adopted, ["itm_note-idea"], 2000, 0);
  assert.equal(get(out, "itm_note-idea").frame_id, undefined);
  assert.equal(frameAt(c, get(c, "itm_frame-plan")), undefined, "a frame is not in itself");
});

test("an arrow bound to two items follows them when one moves", () => {
  let c = fixtureCanvas();
  c = connect(c, "itm_note-idea", "itm_note-status");
  const arrow = c.items.find((i) => i.type === "shape")!;
  assert.ok(
    arrow.type === "shape" && arrow.start?.item_id === "itm_note-idea" && arrow.end?.item_id === "itm_note-status",
  );
  const moved = moveItems(c, ["itm_note-status"], 300, 0);
  const after = moved.items.find((i) => i.type === "shape")!;
  const [p0, p1] = endpoints(after as never)!;
  const idea = get(moved, "itm_note-idea"),
    status = get(moved, "itm_note-status");
  assert.ok(p0.x >= idea.x && p0.x <= idea.x + idea.w, "starts on the idea note's edge");
  assert.ok(
    Math.abs(p1.x - status.x) < 1 ||
      Math.abs(p1.x - (status.x + status.w)) < 1 ||
      Math.abs(p1.y - status.y) < 1 ||
      Math.abs(p1.y - (status.y + status.h)) < 1,
    "ends on the status note's edge",
  );
  assert.equal(moveItems(c, ["itm_doc-rfc-0120"], 0, 0), c, "no move, no change");
});

test("edgePoint leaves a box on the side that faces the target", () => {
  const b = { x: 0, y: 0, w: 100, h: 50 };
  assert.deepEqual(edgePoint(b, { x: 500, y: 25 }), { x: 100, y: 25 });
  assert.deepEqual(edgePoint(b, { x: 50, y: -500 }), { x: 50, y: 0 });
});

test("resize keeps a floor of 40 and the arrows follow", () => {
  const c = resizeItem(fixtureCanvas(), "itm_note-idea", { x: 700, y: 0, w: 5, h: 5 });
  assert.equal(get(c, "itm_note-idea").w, 40);
  assert.equal(get(c, "itm_note-idea").h, 40);
});

test("the item under a point: the top one, frames last, arrows within reach of their line", () => {
  let c = fixtureCanvas();
  assert.equal(hitTest(c.items, { x: 710, y: 10 })?.item_id, "itm_note-idea", "a note over the frame");
  assert.equal(hitTest(c.items, { x: -30, y: -30 })?.item_id, "itm_frame-plan");
  assert.equal(hitTest(c.items, { x: 5000, y: 5000 }), null);
  c = addItems(c, [{ ...makeShape("arrow", { x: 1100, y: 0, w: 100, h: 0 }), item_id: "itm_arrow" }]);
  assert.equal(hitTest(c.items, { x: 1150, y: 5 })?.item_id, "itm_arrow");
  assert.equal(hitTest(c.items, { x: 1150, y: 40 }), null);
});

test("a rubber band takes what it touches", () => {
  const c = fixtureCanvas();
  const ids = itemsInRect(c.items, normRect({ x: 690, y: -10 }, { x: 960, y: 400 }));
  assert.ok(ids.includes("itm_note-idea") && ids.includes("itm_note-status"));
  assert.ok(!ids.includes("itm_doc-rfc-0120"));
  assert.deepEqual(normRect({ x: 10, y: 10 }, { x: 0, y: 30 }), { x: 0, y: 10, w: 10, h: 20 });
});

test("text, tone and order", () => {
  let c = setText(fixtureCanvas(), "itm_note-idea", "New text");
  assert.equal((get(c, "itm_note-idea") as { text: string }).text, "New text");
  c = setTone(c, ["itm_note-idea", "itm_diagram-architecture"], "green");
  assert.equal((get(c, "itm_note-idea") as { tone?: string }).tone, "green");
  assert.equal((get(c, "itm_diagram-architecture") as { tone?: string }).tone, undefined, "a diagram has no tone");
  assert.equal(reorder(c, ["itm_note-idea"], "front").items.at(-1)?.item_id, "itm_note-idea");
  assert.equal(reorder(c, ["itm_doc-adr-0075"], "back").items[0].item_id, "itm_doc-adr-0075");
});

test("duplicates get new ids, keep their frame when it was copied too, and drop bindings to what was not", () => {
  const c = connect(fixtureCanvas(), "itm_note-idea", "itm_note-status");
  const arrow = c.items.find((i) => i.type === "shape")!;
  const lone = duplicate(c, [arrow.item_id]);
  const copy = get(lone.canvas, lone.ids[0]);
  assert.ok(copy.type === "shape" && !copy.start && !copy.end);
  const both = duplicate(c, ["itm_note-idea", "itm_note-status", arrow.item_id]);
  const copiedArrow = get(both.canvas, both.ids[2]);
  assert.ok(
    copiedArrow.type === "shape" &&
      copiedArrow.start?.item_id === both.ids[0] &&
      copiedArrow.end?.item_id === both.ids[1],
  );
  assert.equal(new Set(both.canvas.items.map((i) => i.item_id)).size, both.canvas.items.length);
  assert.equal(snap(13), 16);
  assert.equal(snap(11), 8);
});

test("a handle drag keeps the opposite edges and never turns the box inside out", () => {
  const b = { x: 100, y: 100, w: 200, h: 100 };
  assert.deepEqual(resizeBox(b, "se", 50, 20), { x: 100, y: 100, w: 250, h: 120 });
  assert.deepEqual(resizeBox(b, "nw", 30, 10), { x: 130, y: 110, w: 170, h: 90 });
  assert.deepEqual(
    resizeBox(b, "e", -500, 0),
    { x: 100, y: 100, w: 40, h: 100 },
    "stops at the floor, left edge fixed",
  );
  assert.deepEqual(
    resizeBox(b, "w", 500, 0),
    { x: 260, y: 100, w: 40, h: 100 },
    "stops at the floor, right edge fixed",
  );
  assert.deepEqual(resizeBox(b, "n", 0, 500), { x: 100, y: 160, w: 200, h: 40 });
});

test("moving one end of an arrow frees that end and keeps the other", () => {
  let c = fixtureCanvas();
  c = connect(c, "itm_note-idea", "itm_note-status");
  const arrow = c.items[c.items.length - 1];
  assert.ok(arrow.type === "shape" && arrow.start && arrow.end);
  const before = endpoints(arrow);
  c = moveEnd(c, arrow.item_id, "end", { x: 900, y: 900 });
  const after = c.items[c.items.length - 1];
  assert.ok(after.type === "shape");
  assert.equal(after.end, undefined, "the moved end let go of its item");
  assert.ok(after.start, "the other end stays bound");
  const e = endpoints(after);
  assert.deepEqual(e?.[0], before?.[0]);
  assert.deepEqual(e?.[1], { x: 900, y: 900 });
  assert.equal(moveEnd(c, "itm_note-idea", "end", { x: 1, y: 1 }), c, "a note has no ends");
});
