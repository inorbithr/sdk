// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 InOrbit d.o.o.
//
// The frame's sidebar state, moved with src/frame/sidebar-frame.ts from iohr-mac tests/sidebar.test.ts (origin/main
// d3ac8759) under ADR 0230. parseFrame stands in for the Mac app's parseState on the mode and the width.
import assert from "node:assert/strict";
import { test } from "node:test";
import {
  columnWidth,
  DEFAULT_WIDTH,
  effectiveMode,
  MAX_WIDTH,
  MIN_WIDTH,
  NARROW_BELOW,
  parseFrame,
  RAIL_WIDTH,
  setMode,
  setWidth,
  toggleCollapsed,
  toggleHidden,
} from "../src/frame/sidebar-frame.ts";

test("mode and width: defaults, repair, clamping and the round trip", () => {
  const d = parseFrame(null);
  assert.equal(d.mode, "expanded");
  assert.equal(d.width, DEFAULT_WIDTH);
  const s = parseFrame({ mode: "sideways", width: 9999 });
  assert.equal(s.mode, "expanded");
  assert.equal(s.width, MAX_WIDTH);
  assert.equal(parseFrame({ width: 10 }).width, MIN_WIDTH);
  assert.equal(parseFrame({ width: "x" }).width, DEFAULT_WIDTH);
  assert.equal(parseFrame({ width: Number.NaN }).width, DEFAULT_WIDTH);
  const t = setWidth(setMode(d, "rail"), 300);
  assert.deepEqual(parseFrame(JSON.parse(JSON.stringify(t))), t);
  assert.equal(setWidth(d, 0).width, MIN_WIDTH);
  assert.equal(setWidth(d, DEFAULT_WIDTH), d);
  assert.equal(setMode(d, "expanded"), d);
});

test("anything that is not an object parses to the default frame", () => {
  for (const o of [undefined, 3, "rail", [], ["rail", 300], true]) {
    assert.deepEqual(parseFrame(o), { mode: "expanded", width: DEFAULT_WIDTH });
  }
  assert.deepEqual(parseFrame({ mode: "hidden", width: 300 }), { mode: "hidden", width: 300 });
});

test("the helpers keep an app's own fields on its state", () => {
  const s = { mode: "expanded" as const, width: DEFAULT_WIDTH, pins: ["page:home"] };
  assert.deepEqual(toggleCollapsed(s), { ...s, mode: "rail" });
  assert.deepEqual(setWidth(s, 300).pins, ["page:home"]);
});

test("the toggle button cycles expanded and the rail; ⌘\\ hides and restores", () => {
  let s = parseFrame(null);
  s = toggleCollapsed(s);
  assert.equal(s.mode, "rail");
  s = toggleCollapsed(s);
  assert.equal(s.mode, "expanded");
  s = toggleHidden(s);
  assert.equal(s.mode, "hidden");
  assert.equal(toggleCollapsed(s).mode, "expanded");
  assert.equal(toggleHidden(s).mode, "expanded");
  assert.equal(toggleHidden(setMode(s, "rail")).mode, "hidden");
});

test("a narrow window shows an expanded sidebar as the rail and leaves the choice alone", () => {
  assert.equal(effectiveMode("expanded", NARROW_BELOW - 1), "rail");
  assert.equal(effectiveMode("expanded", NARROW_BELOW), "expanded");
  assert.equal(effectiveMode("hidden", 600), "hidden");
  assert.equal(effectiveMode("rail", 2000), "rail");
  assert.equal(columnWidth("rail", 300), RAIL_WIDTH);
  assert.equal(columnWidth("hidden", 300), 0);
  assert.equal(columnWidth("expanded", 300), 300);
  assert.equal(columnWidth("expanded", 5), MIN_WIDTH);
});
