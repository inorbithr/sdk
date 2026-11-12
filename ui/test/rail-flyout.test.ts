// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 InOrbit d.o.o.
//
// Moved with src/frame/rail-flyout-core.ts from iohr-mac tests/rail-flyout.test.ts (origin/main d3ac8759) under ADR 0230.
import assert from "node:assert/strict";
import { test } from "node:test";
import {
  CLOSE_DELAY,
  type FlyoutEvent,
  type FlyoutState,
  initial,
  inSafeTriangle,
  OPEN_DELAY,
  placeFlyout,
  step,
} from "../src/frame/rail-flyout-core.ts";

const run = (s: FlyoutState, e: FlyoutEvent, now: number) => step(s, e, now);

test("hovering an icon opens its flyout only after the delay", () => {
  let s = run(initial, { type: "iconEnter", id: "a" }, 0).state;
  assert.equal(s.open, null);
  assert.deepEqual(s.pending, { op: "open", id: "a", at: OPEN_DELAY });
  s = run(s, { type: "tick" }, OPEN_DELAY - 1).state;
  assert.equal(s.open, null);
  s = run(s, { type: "tick" }, OPEN_DELAY).state;
  assert.equal(s.open, "a");
  assert.equal(s.pinned, false);
});

test("passing over an icon without resting opens nothing", () => {
  let s = run(initial, { type: "iconEnter", id: "a" }, 0).state;
  s = run(s, { type: "iconLeave", id: "a" }, 100).state;
  s = run(s, { type: "tick" }, 500).state;
  assert.equal(s.open, null);
});

test("moving between icons switches at once, with no new delay", () => {
  let s = run(initial, { type: "iconEnter", id: "a" }, 0).state;
  s = run(s, { type: "tick" }, 150).state;
  s = run(s, { type: "iconLeave", id: "a" }, 300).state;
  s = run(s, { type: "iconEnter", id: "b" }, 310).state;
  assert.equal(s.open, "b");
  assert.equal(s.pending, null);
  s = run(s, { type: "tick" }, 900).state; // the old close timer is gone
  assert.equal(s.open, "b");
});

test("leaving closes after 200 ms; entering the flyout first keeps it", () => {
  let s: FlyoutState = { ...initial, open: "a" };
  s = run(s, { type: "iconLeave", id: "a" }, 1000).state;
  assert.deepEqual(s.pending, { op: "close", at: 1000 + CLOSE_DELAY });
  s = run(s, { type: "flyoutEnter" }, 1100).state;
  s = run(s, { type: "tick" }, 2000).state;
  assert.equal(s.open, "a");
  s = run(s, { type: "flyoutLeave" }, 2000).state;
  s = run(s, { type: "tick" }, 2199).state;
  assert.equal(s.open, "a");
  s = run(s, { type: "tick" }, 2200).state;
  assert.equal(s.open, null);
});

test("coming back to the same icon cancels the close", () => {
  let s: FlyoutState = { ...initial, open: "a" };
  s = run(s, { type: "iconLeave", id: "a" }, 0).state;
  s = run(s, { type: "iconEnter", id: "a" }, 100).state;
  s = run(s, { type: "tick" }, 1000).state;
  assert.equal(s.open, "a");
});

test("moving inside the safe triangle holds the close; outside it lets it run", () => {
  let s: FlyoutState = { ...initial, open: "a" };
  s = run(s, { type: "iconLeave", id: "a" }, 0).state;
  s = run(s, { type: "pointerMove", safe: true }, 150).state;
  s = run(s, { type: "tick" }, 250).state; // would have closed at 200
  assert.equal(s.open, "a");
  s = run(s, { type: "pointerMove", safe: false }, 260).state;
  s = run(s, { type: "tick" }, 349).state; // the hold set at 150 runs to 350
  assert.equal(s.open, "a");
  s = run(s, { type: "tick" }, 350).state;
  assert.equal(s.open, null);
});

test("a click pins; the pointer leaving no longer closes; a second click on the icon goes to the page", () => {
  let r = run(initial, { type: "iconClick", id: "a", hasItems: true }, 0);
  assert.equal(r.state.open, "a");
  assert.equal(r.state.pinned, true);
  assert.equal(r.go, undefined);
  let s = run(r.state, { type: "iconLeave", id: "a" }, 10).state;
  s = run(s, { type: "flyoutLeave" }, 20).state;
  s = run(s, { type: "tick" }, 5000).state;
  assert.equal(s.open, "a");
  r = run(s, { type: "iconClick", id: "a", hasItems: true }, 6000);
  assert.equal(r.go, "a");
  assert.equal(r.state.open, null);
});

test("clicking a hover-opened flyout's icon pins it rather than leaving the page", () => {
  let s = run(initial, { type: "iconEnter", id: "a" }, 0).state;
  s = run(s, { type: "tick" }, 150).state;
  const r = run(s, { type: "iconClick", id: "a", hasItems: true }, 200);
  assert.equal(r.go, undefined);
  assert.equal(r.state.pinned, true);
});

test("an icon with no items goes straight to its page when clicked", () => {
  const r = run(initial, { type: "iconClick", id: "a", hasItems: false }, 0);
  assert.equal(r.go, "a");
  assert.equal(r.state.open, null);
});

test("a pinned flyout switches on hover and stays pinned; clicking elsewhere closes it", () => {
  let s = run(initial, { type: "iconClick", id: "a", hasItems: true }, 0).state;
  s = run(s, { type: "iconEnter", id: "b" }, 10).state;
  assert.equal(s.open, "b");
  assert.equal(s.pinned, true);
  s = run(s, { type: "outsideClick" }, 20).state;
  assert.equal(s.open, null);
  assert.equal(s.pinned, false);
});

test("Esc closes and returns focus to the icon", () => {
  let s = run(initial, { type: "keyOpen", id: "a", items: 3 }, 0).state;
  const seq = s.focusSeq;
  s = run(s, { type: "escape" }, 1).state;
  assert.equal(s.open, null);
  assert.equal(s.focus, "icon");
  assert.ok(s.focusSeq > seq);
});

test("→ or Enter opens pinned and focuses the first item; with no items, the name", () => {
  assert.equal(run(initial, { type: "keyOpen", id: "a", items: 3 }, 0).state.focus, 1);
  assert.equal(run(initial, { type: "keyOpen", id: "a", items: 0 }, 0).state.focus, 0);
  assert.equal(run(initial, { type: "keyOpen", id: "a", items: 3 }, 0).state.pinned, true);
});

test("↑ ↓ move through the name and the items, wrapping; ← closes to the icon", () => {
  let s = run(initial, { type: "keyOpen", id: "a", items: 2 }, 0).state; // focus 1
  s = run(s, { type: "key", key: "ArrowDown", items: 2 }, 1).state;
  assert.equal(s.focus, 2);
  s = run(s, { type: "key", key: "ArrowDown", items: 2 }, 2).state;
  assert.equal(s.focus, 0);
  s = run(s, { type: "key", key: "ArrowUp", items: 2 }, 3).state;
  assert.equal(s.focus, 2);
  s = run(s, { type: "key", key: "ArrowLeft", items: 2 }, 4).state;
  assert.equal(s.open, null);
  assert.equal(s.focus, "icon");
});

test("keys do nothing while no flyout is open", () => {
  const s = run(initial, { type: "key", key: "ArrowDown", items: 2 }, 0).state;
  assert.deepEqual(s, initial);
});

test("the safe triangle: a diagonal move toward the flyout is safe, a move away is not", () => {
  const flyout = { left: 70, top: 100, right: 300, bottom: 400 };
  const from = { x: 40, y: 150 };
  assert.equal(inSafeTriangle({ x: 55, y: 200 }, from, flyout), true);
  assert.equal(inSafeTriangle({ x: 150, y: 200 }, from, flyout), true); // inside the flyout
  assert.equal(inSafeTriangle({ x: 20, y: 150 }, from, flyout), false); // back left
  assert.equal(inSafeTriangle({ x: 55, y: 600 }, from, flyout), false); // far below
});

test("the flyout stays inside the window: near the bottom edge it moves up", () => {
  const icon = { left: 12, top: 700, right: 52, bottom: 740 };
  const p = placeFlyout(icon, 300, { w: 1200, h: 800 });
  assert.equal(p.top, 800 - 8 - 300);
  assert.equal(p.left, 58);
  assert.equal(placeFlyout({ ...icon, top: 100 }, 300, { w: 1200, h: 800 }).top, 100);
  assert.equal(placeFlyout({ ...icon, top: 2 }, 300, { w: 1200, h: 800 }).top, 8);
  assert.equal(placeFlyout(icon, 2000, { w: 1200, h: 800 }).maxHeight, 784);
});

test("Esc during the hover delay cancels the open", () => {
  let s = run(initial, { type: "iconEnter", id: "a" }, 0).state;
  s = run(s, { type: "escape" }, 50).state;
  s = run(s, { type: "tick" }, 500).state;
  assert.equal(s.open, null);
});

test("switching icons by hover drops the old focus index, so ↑ cannot land past the new items", () => {
  let s = run(initial, { type: "keyOpen", id: "a", items: 5 }, 0).state;
  s = run(s, { type: "key", key: "End", items: 5 }, 1).state; // focus 5
  s = run(s, { type: "iconEnter", id: "b" }, 2).state;
  assert.equal(s.open, "b");
  assert.equal(s.focus, null);
  s = run(s, { type: "key", key: "ArrowUp", items: 1 }, 3).state;
  assert.equal(s.focus, 1);
});
