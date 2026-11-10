import assert from "node:assert/strict";
import { test } from "node:test";
import { boardMenu, itemMenu } from "../src/canvas/canvas-menus.ts";
import { flatten, isAction, isOff } from "../src/frame/context-menu-core.ts";
import { pick } from "../src/canvas/picker-core.ts";
import { makeNote } from "../src/canvas/board-core.ts";

const noop = () => {};
const act = { edit: noop, duplicate: noop, front: noop, back: noop, tone: noop, fit: noop, remove: noop, inspect: noop };
const find = (rows: ReturnType<typeof itemMenu>, id: string) => rows.filter(isAction).find((r) => r.id === id) ?? flatten(rows).find((r) => r.action.id === id)?.action;

test("the item menu is off with no selection and ends with Delete", () => {
  const rows = itemMenu([], act);
  assert.ok(isOff(find(rows, "item:remove")!));
  const last = flatten(rows).at(-1)?.action;
  assert.equal(last?.id, "item:remove");
  assert.equal(find(itemMenu([makeNote({ x: 0, y: 0 })], act), "item:remove")?.danger, true);
});

test("a note can be edited and coloured, a card cannot", () => {
  const note = makeNote({ x: 0, y: 0 });
  assert.equal(isOff(find(itemMenu([note], act), "item:edit")!), false);
  assert.equal(isOff(find(itemMenu([note], act), "item:tone")!), false);
  const pr = pick("pr", "inorbithr/sdk#522");
  assert.ok(pr.ok);
  const card = pr.ok ? pr.make({ x: 0, y: 0 }) : note;
  assert.ok(isOff(find(itemMenu([card], act), "item:edit")!));
  assert.ok(isOff(find(itemMenu([card], act), "item:tone")!));
  assert.ok(isOff(find(itemMenu([note, note], act), "item:edit")!));
});

test("the board menu offers undo and redo only when there is something to go back to", () => {
  const off = boardMenu({ note: noop, frame: noop, add: noop, fitAll: noop, fitHome: noop, undo: null, redo: null });
  assert.ok(isOff(find(off, "board:undo")!));
  const on = boardMenu({ note: noop, frame: noop, add: noop, fitAll: noop, fitHome: noop, undo: noop, redo: null });
  assert.equal(isOff(find(on, "board:undo")!), false);
  assert.ok(isOff(find(on, "board:redo")!));
});

test("the picker reads references and says what is wrong", () => {
  const pr = pick("pr", " inorbithr/sdk#522 ");
  assert.ok(pr.ok);
  if (pr.ok) assert.deepEqual([pr.make({ x: 8, y: 16 }).type, pr.make({ x: 8, y: 16 }).title], ["pr", "inorbithr/sdk#522"]);
  const d = pick("decision", "rfc 0217");
  assert.ok(d.ok);
  if (d.ok) assert.equal(d.make({ x: 0, y: 0 }).title, "RFC 0217");
  const doc = pick("document", "lab_01M49HTNZHP26N7MAFB5G988XR ldoc_01M6YNVZ4YM54FGC1C8NCYSCJW");
  assert.ok(doc.ok);
  for (const [k, t] of [["pr", "nope"], ["decision", "memo 1"], ["document", "ldoc_x"], ["pr", ""]] as const) assert.equal(pick(k, t).ok, false);
});
