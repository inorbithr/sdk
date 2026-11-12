// The Orbits list (src/lib/orbits-core.ts).
import assert from "node:assert/strict";
import { test } from "node:test";
import { add, defaultOrbits, HOME, MAX_ORBITS, neighbour, parseOrbits, remove } from "../src/lib/orbits-core.ts";

test("the start is Home alone", () => {
  assert.deepEqual(defaultOrbits().pages, [HOME]);
});

test("a page is added once, at the end, up to the limit", () => {
  const a = add(defaultOrbits(), "ext:crawler");
  assert.deepEqual(a.pages, ["home", "ext:crawler"]);
  assert.equal(add(a, "ext:crawler"), a);
  let full = defaultOrbits();
  for (let i = 0; i < MAX_ORBITS + 3; i++) full = add(full, `ext:e${i}`);
  assert.equal(full.pages.length, MAX_ORBITS);
});

test("Home cannot be taken out; another page can", () => {
  const a = add(defaultOrbits(), "ext:x");
  assert.equal(remove(a, HOME), a);
  assert.deepEqual(remove(a, "ext:x").pages, ["home"]);
  assert.equal(remove(a, "ext:none"), a);
});

test("previous and next stop at the ends", () => {
  const a = add(add(defaultOrbits(), "ext:a"), "ext:b");
  assert.equal(neighbour(a, "ext:a", 1), "ext:b");
  assert.equal(neighbour(a, "ext:a", -1), "home");
  assert.equal(neighbour(a, "home", -1), null);
  assert.equal(neighbour(a, "ext:b", 1), null);
  assert.equal(neighbour(a, "elsewhere", 1), null);
});

test("a stored set is made whole: Home first, duplicates and vanished pages dropped", () => {
  assert.deepEqual(parseOrbits(null).pages, ["home"]);
  assert.deepEqual(parseOrbits({ version: 2, pages: ["ext:a"] }).pages, ["home"]);
  const raw = { version: 1, pages: ["ext:a", 7, "home", "ext:a", "ext:gone", "ext:b"] };
  assert.deepEqual(parseOrbits(raw, (p) => p !== "ext:gone").pages, ["home", "ext:a", "ext:b"]);
});
