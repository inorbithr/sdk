// Consent for sensors: explicit, per person, and the service is the record (RFC 0217, PRD 0077).

import assert from "node:assert/strict";
import { test } from "node:test";
import type { Sensor } from "../src/lib/sensors-core.ts";
import { isOn, mineOf, pauseAllTargets, sentText, stateWord, switchAction } from "../src/lib/sensors-consent.ts";

const s = (id: string, extra: Partial<Sensor> = {}): Sensor => ({
  id, name: id, source_id: "agent", kind: "gauge", state: "active", health: "fine", interval_secs: 60, rate_per_min: 1,
  last_seen: 1, value: 1, unit: "", readings: [], revocable: true, ...extra,
});

test("only a strict active is on; unknown words are off", () => {
  assert.equal(isOn(s("a")), true);
  assert.equal(isOn(s("a", { state: "paused" })), false);
  assert.equal(isOn(s("a", { state: "revoked" })), false);
  assert.equal(isOn({ state: "banana" as Sensor["state"] }), false);
});

test("a person controls only the sensors they own, never built-ins, and nobody when not signed in", () => {
  const all = [s("a", { owner: "u1" }), s("b", { owner: "U1" }), s("c", { owner: "u2" }), s("d", { owner: "u1", builtin: true }), s("e")];
  assert.deepEqual(mineOf(all, "u1").map((x) => x.id), ["a", "b"]);
  assert.deepEqual(mineOf(all, ""), []);
  assert.deepEqual(mineOf(all, null), []);
  assert.deepEqual(mineOf(all, "  "), []);
});

test("a switch is a pause or a resume, never both, and a revoked sensor has none", () => {
  assert.equal(switchAction(s("a"), false), "pause");
  assert.equal(switchAction(s("a"), true), null);
  assert.equal(switchAction(s("a", { state: "paused" }), true), "resume");
  assert.equal(switchAction(s("a", { state: "paused" }), false), null);
  assert.equal(switchAction(s("a", { state: "revoked" }), true), null);
  assert.equal(switchAction(s("a", { state: "revoked" }), false), null);
});

test("pause all touches the sensors that are on, and nothing else", () => {
  const mine = [s("a"), s("b", { state: "paused" }), s("c", { state: "revoked" }), s("d")];
  assert.deepEqual(pauseAllTargets(mine).map((x) => x.id), ["a", "d"]);
  assert.deepEqual(pauseAllTargets([]), []);
  assert.deepEqual(["active", "paused", "revoked"].map((state) => stateWord({ state: state as Sensor["state"] })), ["On", "Paused", "Revoked"]);
});

test("what was sent, in words; 64-bit counts arrive as strings", () => {
  assert.equal(sentText(null), "");
  assert.equal(sentText({ readings: 0, rollups: 0 }), "Nothing sent yet.");
  assert.equal(sentText({ readings: "12", rollups: "3", first_at: "2026-11-01T10:00:00Z" }), "12 readings and 3 rollups since 2026-11-01.");
  assert.equal(sentText({ readings: 1, rollups: 0 }), "1 reading.");
  assert.equal(sentText({ readings: "junk", rollups: 2 }), "2 rollups.");
});
