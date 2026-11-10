import assert from "node:assert/strict";
import { test } from "node:test";
import { canvasPath, canvasRoute } from "../src/canvas/canvas-route.ts";
import { type KeyRow, pressed } from "../src/canvas/keys-core.ts";
import table from "../src/shortcuts/pages/canvas.keys.json" with { type: "json" };

const rows = table.rows as KeyRow[];
const ev = (key: string, o: Partial<{ code: string; metaKey: boolean; ctrlKey: boolean; shiftKey: boolean; altKey: boolean }> = {}) => ({
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
