// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 InOrbit d.o.o.
//
// The right dock's width limits, moved with src/frame/dock-frame.ts from iohr-mac tests/side-panels.test.ts
// (origin/main d3ac8759) under ADR 0230, and the one resize edge's arithmetic (src/frame/resize-handle-core.ts),
// folded from the Mac app's sidebar ResizeEdge and dock Resize.
import assert from "node:assert/strict";
import { test } from "node:test";
import {
  clampPanel,
  closeDock,
  FIRST_PANEL,
  MAX_PANEL,
  MIN_PANEL,
  maxPanel,
  parseDock,
  setDockWidth,
  toggleDock,
} from "../src/frame/dock-frame.ts";
import { dragWidth, keyWidth } from "../src/frame/resize-handle-core.ts";

test("panel widths stay between 300 and 720", () => {
  assert.equal(clampPanel(10), 300);
  assert.equal(clampPanel(9999), 720);
  assert.equal(clampPanel(412.4), 412);
});

test("a panel can grow with a wide window, leaving the page 420 px", () => {
  assert.equal(clampPanel(9999, 2400), 1980);
  assert.equal(clampPanel(9999, 1000), 720);
  assert.equal(maxPanel(800), MAX_PANEL);
});

test("a stored dock is made whole: junk falls back to closed at the first width", () => {
  assert.deepEqual(parseDock(null), { open: false, width: FIRST_PANEL });
  assert.deepEqual(parseDock({ open: "yes", width: "wide" }), { open: false, width: FIRST_PANEL });
  assert.deepEqual(parseDock({ open: true, width: 12 }), { open: true, width: MIN_PANEL });
  // A wide window's width is kept; the dock clamps it to the window it is drawn in.
  assert.deepEqual(parseDock({ open: true, width: 1500.6 }), { open: true, width: 1501 });
});

test("the dock opens, closes and takes clamped widths; no change keeps the same state", () => {
  const s = parseDock(null);
  assert.equal(toggleDock(s).open, true);
  assert.equal(toggleDock(toggleDock(s)).open, false);
  assert.equal(closeDock(s), s);
  assert.equal(setDockWidth(s, FIRST_PANEL, 1280), s);
  assert.equal(setDockWidth(s, 9999, 1280).width, 860);
  assert.equal(setDockWidth(s, 1, 1280).width, MIN_PANEL);
});

test("the arrow that points away from the box grows it, on either edge", () => {
  // The sidebar's edge is on its right; the dock's on its left.
  assert.equal(keyWidth("ArrowRight", 256, "right", 16), 272);
  assert.equal(keyWidth("ArrowLeft", 256, "right", 16), 240);
  assert.equal(keyWidth("ArrowLeft", 380, "left", 24), 404);
  assert.equal(keyWidth("ArrowRight", 380, "left", 24), 356);
  assert.equal(keyWidth("Enter", 380, "left", 24), null);
});

test("a drag measures the box from the side away from its edge", () => {
  const box = { left: 900, right: 1272 };
  assert.equal(dragWidth(800, box, "left"), 472);
  assert.equal(dragWidth(1100, { left: 0, right: 256 }, "right"), 1100);
});
