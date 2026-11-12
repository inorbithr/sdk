// The browser draft of a canvas (src/canvas/board-store-core.ts): list, open, save, undo, a second tab, a full disk.
import assert from "node:assert/strict";
import { test } from "node:test";
import { makeNote } from "../src/canvas/board-core.ts";
import {
  BODY,
  Board,
  type Clock,
  copyCanvas,
  createCanvas,
  deleteCanvas,
  HISTORY,
  lastCanvas,
  listCanvases,
  memoryKV,
  NO_CANVAS,
  readView,
  rememberCanvas,
  renameCanvas,
  writeView,
} from "../src/canvas/board-store-core.ts";
import { type Canvas, serialize } from "../src/canvas/schema.ts";
import { fixtureCanvas } from "./canvas-fixture.ts";

/** A clock the test drives by hand. */
function clock() {
  const tasks: { id: number; f: () => void }[] = [];
  let n = 0,
    t = 0;
  const c: Clock = {
    set: (f) => {
      tasks.push({ id: ++n, f });
      return n;
    },
    clear: (id) => {
      const i = tasks.findIndex((x) => x.id === id);
      if (i >= 0) tasks.splice(i, 1);
    },
    now: () => `2026-11-10T10:00:${String(++t).padStart(2, "0")}.000Z`,
  };
  return {
    c,
    run: () => {
      const all = tasks.splice(0);
      for (const x of all) x.f();
    },
    waiting: () => tasks.length,
  };
}

const seeded = () => {
  const kv = memoryKV();
  const c = fixtureCanvas();
  kv.set(BODY + c.canvas_id, serialize(c));
  return { kv, id: c.canvas_id };
};
const note = (c: Canvas) => c.items.filter((i) => i.type === "note").length;

test("opening a canvas that is not there says so; one that is opens with its items", () => {
  const { kv, id } = seeded();
  assert.equal(new Board("cnv_nope", kv, clock().c).getSnapshot().error, NO_CANVAS);
  const b = new Board(id, kv, clock().c);
  assert.equal(b.getSnapshot().error, null);
  assert.equal(b.getSnapshot().canvas?.items.length, 6);
});

test("an edit shows at once and is saved 400 ms later, once, with the revision up by one", () => {
  const { kv, id } = seeded();
  const k = clock();
  const b = new Board(id, kv, k.c);
  b.change((c) => ({ ...c, items: [...c.items, makeNote({ x: 0, y: 0 }, "a")] }));
  b.change((c) => ({ ...c, items: [...c.items, makeNote({ x: 0, y: 0 }, "b")] }));
  assert.equal(note(b.getSnapshot().canvas!), 4);
  assert.equal(b.getSnapshot().dirty, true);
  assert.equal(listCanvases(kv)[0].items, 6, "not yet on disk");
  assert.equal(k.waiting(), 1, "the two edits share one timer");
  k.run();
  assert.equal(listCanvases(kv)[0].items, 8);
  assert.equal(b.getSnapshot().canvas?.revision, 1);
  assert.equal(b.getSnapshot().dirty, false);
  assert.ok(b.getSnapshot().savedAt);
});

test("undo and redo step back and forward, and a new edit clears redo", () => {
  const { kv, id } = seeded();
  const b = new Board(id, kv, clock().c);
  const add = (t: string) => b.change((c) => ({ ...c, items: [...c.items, makeNote({ x: 0, y: 0 }, t)] }));
  add("a");
  add("b");
  b.undo();
  assert.equal(note(b.getSnapshot().canvas!), 3);
  assert.equal(b.getSnapshot().future, 1);
  b.redo();
  assert.equal(note(b.getSnapshot().canvas!), 4);
  b.undo();
  add("c");
  assert.equal(b.getSnapshot().future, 0);
  b.undo();
  b.undo();
  b.undo();
  assert.equal(note(b.getSnapshot().canvas!), 2, "back to the start and no further");
  assert.equal(b.getSnapshot().past, 0);
});

test("a drag made of many steps is one undo step when only its start is a checkpoint", () => {
  const { kv, id } = seeded();
  const b = new Board(id, kv, clock().c);
  b.checkpoint();
  for (let i = 1; i <= 5; i++)
    b.change((c) => ({ ...c, items: c.items.map((x) => (x.item_id === "itm_note-idea" ? { ...x, x: 700 + i } : x)) }), {
      undoable: false,
    });
  assert.equal(b.getSnapshot().past, 1);
  b.undo();
  assert.equal(b.getSnapshot().canvas?.items.find((x) => x.item_id === "itm_note-idea")?.x, 700);
});

test("history is capped at 80 steps", () => {
  const { kv, id } = seeded();
  const b = new Board(id, kv, clock().c);
  for (let i = 0; i < HISTORY + 20; i++) b.change((c) => ({ ...c, title: `t${i}` }));
  assert.equal(b.getSnapshot().past, HISTORY);
});

test("a save that finds a newer revision (another tab) replays our edits on it", () => {
  const { kv, id } = seeded();
  const k = clock();
  const a = new Board(id, kv, k.c);
  const other = new Board(id, kv, clock().c);
  other.change((c) => ({ ...c, items: [...c.items, makeNote({ x: 0, y: 0 }, "from the other tab")] }));
  other.flush();
  a.change((c) => ({ ...c, items: [...c.items, makeNote({ x: 0, y: 0 }, "from this tab")] }));
  k.run();
  const disk = new Board(id, kv, clock().c).getSnapshot().canvas!;
  const texts = disk.items.filter((i) => i.type === "note").map((i) => (i as { text: string }).text);
  assert.ok(texts.includes("from the other tab") && texts.includes("from this tab"), texts.join("|"));
  assert.equal(disk.revision, 2);
});

test("a tab with nothing unsaved takes another tab's save; one with edits on the way keeps them", () => {
  const { kv, id } = seeded();
  const a = new Board(id, kv, clock().c);
  const other = new Board(id, kv, clock().c);
  other.change((c) => ({ ...c, title: "Renamed elsewhere" }));
  other.flush();
  a.external();
  assert.equal(a.getSnapshot().canvas?.title, "Renamed elsewhere");
  a.change((c) => ({ ...c, description: "mine" }));
  other.change((c) => ({ ...c, title: "Again" }));
  other.flush();
  a.external();
  assert.equal(a.getSnapshot().canvas?.title, "Renamed elsewhere", "unsaved edits are not overwritten");
});

test("a full disk is said, the edit stays on screen and is saved when there is room", () => {
  const { kv, id } = seeded();
  const k = clock();
  const real = kv.set;
  let full = true;
  kv.set = (key, v) => {
    if (full) throw new DOMException("quota", "QuotaExceededError");
    real(key, v);
  };
  const b = new Board(id, kv, k.c);
  b.change((c) => ({ ...c, title: "Big" }));
  k.run();
  assert.match(b.getSnapshot().error ?? "", /storage is full/);
  assert.equal(b.getSnapshot().canvas?.title, "Big");
  assert.equal(b.getSnapshot().dirty, true);
  full = false;
  b.flush();
  assert.equal(b.getSnapshot().error, null);
  assert.equal(listCanvases(kv)[0].title, "Big");
});

test("create, list, copy, rename, delete; a body that does not parse is left out", () => {
  const kv = memoryKV();
  const a = createCanvas(kv, "  First ", "2026-11-10T09:00:00.000Z");
  assert.equal(a.title, "First");
  const b = copyCanvas(kv, a.canvas_id, "2026-11-10T10:00:00.000Z");
  assert.equal(b.title, "First (copy)");
  kv.set(`${BODY}cnv_bad`, "{not json");
  assert.deepEqual(
    listCanvases(kv).map((s) => s.title),
    ["First (copy)", "First"],
    "newest first",
  );
  renameCanvas(kv, a.canvas_id, "Renamed", "2026-11-10T11:00:00.000Z");
  assert.equal(listCanvases(kv)[0].title, "Renamed");
  assert.equal(createCanvas(kv, "   ").title, "Untitled canvas");
  deleteCanvas(kv, a.canvas_id);
  assert.ok(!listCanvases(kv).some((s) => s.canvas_id === a.canvas_id));
});

test("the view and the last canvas are kept per viewer and survive junk", () => {
  const kv = memoryKV();
  assert.equal(readView(kv, "cnv_a"), null);
  writeView(kv, "cnv_a", { x: 10.4, y: -20.6, zoom: 0.51234 });
  assert.deepEqual(readView(kv, "cnv_a"), { x: 10, y: -21, zoom: 0.512 });
  kv.set("iohr-ui.canvas.view.cnv_b", '{"x":"a","y":1,"zoom":1}');
  assert.equal(readView(kv, "cnv_b"), null);
  kv.set("iohr-ui.canvas.view.cnv_c", "junk");
  assert.equal(readView(kv, "cnv_c"), null);
  assert.equal(lastCanvas(kv), null);
  rememberCanvas(kv, "cnv_a");
  assert.equal(lastCanvas(kv), "cnv_a");
  deleteCanvas(kv, "cnv_a");
  assert.equal(lastCanvas(kv), null, "deleting the last canvas forgets it");
});
