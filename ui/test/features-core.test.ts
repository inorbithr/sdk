// The pages that can be put in maintenance (src/lib/features-core.ts).
import assert from "node:assert/strict";
import { test } from "node:test";
import {
  bringBack,
  bringEverythingOnline,
  closedIds,
  entryOf,
  isDown,
  parseStates,
  putInMaintenance,
  UNDO_MS,
  undoLive,
  undoOf,
  whyClosed,
} from "../src/lib/features-core.ts";

test("every page starts online", () => {
  assert.equal(isDown(entryOf({}, "home")), false);
  assert.deepEqual(closedIds({}), []);
});

test("putting a page in maintenance keeps its reason and ETA, trimmed; the same again changes nothing", () => {
  const s = putInMaintenance({}, "home", { reason: "  moving the data  ", eta: " 15:00 " });
  assert.deepEqual(entryOf(s, "home"), { mode: "maintenance", reason: "moving the data", eta: "15:00" });
  assert.deepEqual(closedIds(s), ["home"]);
  assert.equal(putInMaintenance(s, "home", { reason: "moving the data", eta: "15:00" }), s);
  assert.equal(whyClosed(entryOf(s, "home")), "moving the data");
  assert.equal(
    whyClosed(entryOf(putInMaintenance({}, "home", { reason: "", eta: "" }), "home")),
    "Put in maintenance.",
  );
});

test("an unknown page cannot be put in maintenance", () => {
  const s = {};
  assert.equal(putInMaintenance(s, "nope", { reason: "x", eta: "" }), s);
});

test("bring back and bring everything online", () => {
  const s = putInMaintenance(putInMaintenance({}, "home", { reason: "", eta: "" }), "extensions", {
    reason: "",
    eta: "",
  });
  assert.deepEqual(closedIds(bringBack(s, "home")), ["extensions"]);
  const none = {};
  assert.equal(bringBack(none, "home"), none);
  assert.deepEqual(bringEverythingOnline(s), {});
  assert.equal(bringEverythingOnline(none), none);
});

test("a change can be taken back for ten seconds", () => {
  const u = undoOf("Put Home in maintenance", {}, 1000);
  assert.equal(undoLive(u, 1000 + UNDO_MS - 1), true);
  assert.equal(undoLive(u, 1000 + UNDO_MS), false);
  assert.equal(undoLive(null, 0), false);
});

test("stored states are made whole", () => {
  assert.deepEqual(parseStates(null), {});
  assert.deepEqual(parseStates({ home: { mode: "online" }, ghost: { mode: "maintenance" } }), {});
  assert.deepEqual(parseStates({ home: { mode: "maintenance", reason: 5, eta: "soon" } }), {
    home: { mode: "maintenance", reason: "", eta: "soon" },
  });
});
