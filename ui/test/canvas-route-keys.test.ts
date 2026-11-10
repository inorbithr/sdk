import assert from "node:assert/strict";
import { test } from "node:test";
import { editTarget, itemLabel, makeFrame, makeNote } from "../src/canvas/board-core.ts";
import { canvasPath, canvasRoute } from "../src/canvas/canvas-route.ts";
import { type KeyRow, pressed } from "../src/canvas/keys-core.ts";
import { emptyCanvas } from "../src/canvas/schema.ts";
import table from "../src/shortcuts/pages/canvas.keys.json" with { type: "json" };

const rows = table.rows as KeyRow[];
const ev = (
  key: string,
  o: Partial<{ code: string; metaKey: boolean; ctrlKey: boolean; shiftKey: boolean; altKey: boolean }> = {},
) => ({
  key,
  metaKey: false,
  ctrlKey: false,
  altKey: false,
  shiftKey: false,
  ...o,
});

test("routes: the list, one canvas, and anything else", () => {
  assert.deepEqual(canvasRoute("/canvas"), { id: null });
  assert.deepEqual(canvasRoute("/canvas/"), { id: null });
  assert.deepEqual(canvasRoute("/canvas/cnv_01"), { id: "cnv_01" });
  assert.equal(canvasRoute("/canvases"), null);
  assert.equal(canvasRoute("/canvas/a/b"), null);
  assert.equal(canvasRoute("/canvas/%E0%A4%A"), null);
  assert.equal(canvasPath("cnv a"), "/canvas/cnv%20a");
  assert.equal(canvasPath(null), "/canvas");
});

test("keys: a bare letter is a tool outside text and nothing inside it; a chord works in both", () => {
  assert.equal(pressed(rows, ev("n", { code: "KeyN" }), false), "canvas-note");
  assert.equal(pressed(rows, ev("n", { code: "KeyN" }), true), null);
  assert.equal(pressed(rows, ev("z", { code: "KeyZ", metaKey: true }), true), "canvas-undo");
  assert.equal(pressed(rows, ev("z", { code: "KeyZ", metaKey: true, shiftKey: true }), false), "canvas-redo");
  assert.equal(pressed(rows, ev("Escape"), true), "canvas-cancel");
  assert.equal(pressed(rows, ev("!", { code: "Digit1", shiftKey: true }), false), "canvas-fit-home");
  assert.equal(pressed(rows, ev("Backspace"), true), null);
  assert.equal(pressed(rows, ev("Backspace"), false), "canvas-delete");
});

test("Enter opens edit on one selected note, frame or shape, and only there", () => {
  const note = makeNote({ x: 0, y: 0 }, "secret body");
  const frame = makeFrame({ x: 0, y: 0 });
  const c = { ...emptyCanvas("T"), items: [note, frame] };
  assert.equal(pressed(rows, ev("Enter"), false), "canvas-edit");
  assert.equal(pressed(rows, ev("Enter"), true), null, "Enter in a text box stays a newline");
  assert.equal(pressed(rows, ev("Escape"), true), "canvas-cancel");
  assert.equal(editTarget(c, [note.item_id]), note.item_id);
  assert.equal(editTarget(c, [note.item_id, frame.item_id]), null);
  assert.equal(editTarget(c, []), null);
  assert.equal(editTarget(c, ["itm_missing"]), null);
});

test("an item's label is its type and short title, never a note's body", () => {
  assert.equal(itemLabel(makeNote({ x: 0, y: 0 }, "secret body")), "note");
  assert.equal(itemLabel(makeFrame({ x: 0, y: 0 }, "Plan")), "frame: Plan");
});
