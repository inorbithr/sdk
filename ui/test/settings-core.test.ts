import assert from "node:assert/strict";
import { test } from "node:test";
import {
  DEFAULT_PANE,
  HELD,
  heldNow,
  matchPanes,
  PANES,
  paneById,
  paneHref,
  settingsRoute,
} from "../src/lib/settings-core.ts";

test("the panes are the Mac app's 22, in its order, with unique ids", () => {
  assert.equal(PANES.length, 22);
  assert.equal(new Set(PANES.map((p) => p.id)).size, 22);
  assert.deepEqual(PANES.map((p) => p.id).slice(0, 5), ["setup", "general", "appearance", "accounts", "team"]);
});

test("every pane either works here or says where it works", () => {
  for (const p of PANES) {
    assert.equal(p.built, p.where === "", p.id);
    assert.ok(p.label.length > 0 && p.blurb.length > 0, p.id);
  }
  assert.equal(PANES.filter((p) => p.built).length, 10);
});

test("the default pane exists and is built", () => {
  assert.equal(paneById(DEFAULT_PANE)?.built, true);
});

test("the route: Settings alone is the default pane, a name is its pane, a stranger is unknown, other paths are not Settings", () => {
  assert.deepEqual(settingsRoute("/settings"), { pane: paneById(DEFAULT_PANE) });
  assert.deepEqual(settingsRoute("/settings/"), { pane: paneById(DEFAULT_PANE) });
  assert.deepEqual(settingsRoute("/settings/team"), { pane: paneById("team") });
  assert.deepEqual(settingsRoute("/settings/nope"), { unknown: "nope" });
  assert.equal(settingsRoute("/"), null);
  assert.equal(settingsRoute("/ext/settings"), null);
  assert.equal(settingsRoute("/settings/team/x"), null);
});

test("every pane's address routes back to that pane", () => {
  for (const p of PANES) assert.deepEqual(settingsRoute(paneHref(p.id)), { pane: p });
});

test("search keeps panes by label, blurb or where; empty keeps all in order; no match is empty", () => {
  assert.equal(matchPanes("").length, 22);
  assert.equal(matchPanes("   ").length, 22);
  assert.deepEqual(
    matchPanes("KEYBOARD").map((p) => p.id),
    ["keyboard"],
  );
  assert.ok(matchPanes("mac app").every((p) => p.where === "the Mac app" || /mac app/i.test(p.blurb)));
  assert.deepEqual(matchPanes("zzzz"), []);
});

test("what is held lists the six keys, and heldNow keeps the ones that are stored", () => {
  assert.equal(HELD.length, 6);
  assert.deepEqual(
    heldNow(() => false),
    [],
  );
  assert.deepEqual(
    heldNow((k) => k === "iohr-ui.theme"),
    ["iohr-ui.theme"],
  );
  assert.equal(heldNow(() => true).length, 6);
});
