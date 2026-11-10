import assert from "node:assert/strict";
import { test } from "node:test";
import { emptyCanvas } from "../src/canvas/schema.ts";
import { asksFor, buildBrief, buildSelRef, isRepeat, labelsOf, stateOf, suggestionAsk, suggestionText, suggestKind } from "../src/canvas/suggest-core.ts";

const canvas = () => {
  const c = { ...emptyCanvas("Plan"), canvas_id: "cnv_1" };
  c.items.push({ item_id: "itm_a", type: "note", x: 0, y: 0, w: 10, h: 10, text: "The API gateway" } as never);
  c.items.push({ item_id: "itm_b", type: "note", x: 0, y: 0, w: 10, h: 10, text: "Second" } as never);
  return c;
};

test("selection reference, labels and kind", () => {
  const ref = buildSelRef(canvas(), ["itm_a", "itm_a", "itm_b", "nope"]);
  assert.equal(ref?.elements.length, 2);
  assert.equal(labelsOf(ref!), "“The API gateway” and “Second”");
  assert.equal(suggestKind(ref), "api");
});

test("suggestion text and ask carry the canvas page", () => {
  const ref = buildSelRef(canvas(), ["itm_b"])!;
  assert.equal(suggestionText("wrong", "  "), "This is wrong.");
  assert.equal(suggestionText("wrong", "no  way"), "no way");
  assert.equal(suggestionAsk(ref, "")?.body, undefined);
  assert.equal(suggestionAsk(ref, "x")?.source, "canvas:cnv_1");
});

test("a double press files once", () => {
  const ref = buildSelRef(canvas(), ["itm_b"])!;
  const m = new Map<string, number>();
  assert.equal(isRepeat(m, ref, "x", 0), false);
  assert.equal(isRepeat(m, ref, "X", 100), true);
  assert.equal(isRepeat(m, ref, "x", 9000), false);
});

test("brief never lets the work merge; state and asks from the picture", () => {
  const ref = buildSelRef(canvas(), ["itm_b"])!;
  const b = buildBrief({ canvas: canvas(), ref, marker: "cwk-1", kind: "design" });
  assert.match(b, /Never merge, roll or deploy/);
  assert.equal(stateOf({ state: "RUNNING" }).state, "running");
  assert.equal(stateOf({ state: "done", pr: "x" }).state, "done");
  assert.equal(stateOf({ state: "queued" }).state, "queued");
  const l = asksFor([{ id: "a1", source: "canvas:cnv_1", title: "t", state: "queued" }, { id: "a2", source: "canvas:other" }], "cnv_1");
  assert.deepEqual(l.map((x) => x.id), ["a1"]);
});
