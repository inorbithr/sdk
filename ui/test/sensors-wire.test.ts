// The sensors service's and the coordinator's answers mapped onto the page's shapes, the live reading
// laid over the list, and the session list (RFC 0217).

import assert from "node:assert/strict";
import { test } from "node:test";
import { type Reading } from "../src/lib/sensors-core.ts";
import {
  askBucket,
  fromApi,
  healthCounts,
  liveHealthWord,
  pushReading,
  readingsOf,
  sessionsOf,
  streamReading,
  withReadings,
  wireCount,
} from "../src/lib/sensors-wire.ts";

const NOW = Date.parse("2026-10-14T12:00:00Z");
const iso = (msAgo: number) => new Date(NOW - msAgo).toISOString();

const wire = {
  groups: [
    {
      source: "server",
      sensors: [
        { sensor_id: "x1", name: "Temp", source: "server", health: "ok", state: "active", interval_seconds: 10, last_seen_at: iso(5_000), unit: "count", producer_kind: "agent", producer_ref: "box", owner: "u1", schema_version: 2 },
        { sensor_id: "x2", source: "server", health: "ok", state: "active", interval_seconds: 10, last_seen_at: iso(120_000) },
        { sensor_id: "", source: "server" },
      ],
    },
    { source: "monitors", sensors: [{ sensor_id: "mon_01ABC", name: "API", source: "monitors", health: "down", state: "active", interval_seconds: 60, last_seen_at: iso(5_000), builtin: true }] },
    { source: "asks", sensors: [] },
  ],
};

test("the list (groups by source) maps onto planets; stale sensors are silent; built-ins are read only", () => {
  assert.equal(wireCount(wire), 4);
  const s = fromApi(wire, NOW);
  assert.deepEqual(s.map((x) => [x.id, x.kind]), [["server", "machine"], ["monitors", "service"]]);
  assert.deepEqual(s[0].sensors.map((x) => [x.id, x.health]), [["x1", "fine"], ["x2", "silent"]]);
  assert.equal(s[0].sensors[0].producer, "agent · box");
  assert.equal(s[0].sensors[0].unit, "", "count is not a unit");
  assert.equal(s[0].sensors[0].owner, "u1");
  assert.equal(s[0].sensors[0].value, null, "the list carries no value");
  assert.equal(s[1].sensors[0].health, "failing");
  assert.equal(s[1].sensors[0].revocable, false);
  assert.equal(s[1].sensors[0].builtin, true);
  assert.equal(s[0].sensors[0].revocable, true);
  assert.deepEqual(healthCounts(s), { failing: 1, degraded: 0, fine: 1, silent: 1 });
});

test("readings come as produced_at, value and state, oldest first", () => {
  const r = readingsOf([{ produced_at: iso(1_000), value: 2, state: "ok" }, { produced_at: iso(3_000), value: 1, state: "failed" }, { produced_at: "" }]);
  assert.deepEqual(r.map((x) => [x.value, x.ok]), [[1, false], [2, true]]);
});

test("a live reading is a reading event only; a summary and junk are not", () => {
  const ev = { type: "reading", reading: { sensor_id: "x1", produced_at: iso(1000), received_at: iso(900), value: 7, state: "ok" } };
  const r = streamReading(ev);
  assert.equal(r?.sensor_id, "x1");
  assert.equal(r?.reading.value, 7);
  assert.equal(streamReading({ type: "summary", missed: 4 }), null);
  assert.equal(streamReading({ type: "reading", reading: { produced_at: iso(1) } }), null);
  assert.equal(streamReading({ type: "reading", reading: { sensor_id: "x1" } }), null, "no time, no reading");
  assert.equal(liveHealthWord("failed"), "down");
  assert.equal(liveHealthWord("ok"), "ok");
  assert.equal(liveHealthWord(undefined), undefined);
});

test("readings are kept oldest first, without repeats, newest N", () => {
  let by: Record<string, readonly Reading[]> = {};
  by = pushReading(by, "a", { at: 3, value: 3, ok: true });
  by = pushReading(by, "a", { at: 1, value: 1, ok: true });
  const same = pushReading(by, "a", { at: 3, value: 3, ok: true });
  assert.equal(same, by, "an identical reading changes nothing");
  assert.deepEqual(by.a.map((r) => r.at), [1, 3]);
  let big: Record<string, readonly Reading[]> = {};
  for (let i = 0; i < 10; i++) big = pushReading(big, "a", { at: i, value: i, ok: true }, 4);
  assert.deepEqual(big.a.map((r) => r.at), [6, 7, 8, 9]);
});

test("a live reading gives the value, moves last seen and wakes a stale sensor; a failed one turns it red", () => {
  const sources = fromApi(wire, NOW);
  assert.equal(sources[0].sensors[1].health, "silent");
  const by = { x2: [{ at: NOW - 1_000, value: 3, ok: true }], x1: [{ at: NOW - 500, value: 9, ok: false }] };
  const next = withReadings(sources, by, { x1: "down" }, NOW);
  const x1 = next[0].sensors[0];
  const x2 = next[0].sensors[1];
  assert.equal(x2.value, 3);
  assert.equal(x2.health, "fine", "it spoke, so it is no longer silent");
  assert.equal(x1.health, "failing");
  assert.equal(x1.last_seen, NOW - 500);
  assert.equal(next[0].health, "failing");
  assert.equal(sources[0].sensors[1].health, "silent", "the list is not changed in place");
  // Time alone silences it again: three missed intervals later.
  assert.equal(withReadings(sources, by, {}, NOW + 60_000)[0].sensors[1].health, "silent");
});

test("asks fall into five buckets and unknown words stay visible as queued", () => {
  assert.equal(askBucket("verified"), "closed");
  assert.equal(askBucket("pr-open"), "building");
  assert.equal(askBucket("in-preview"), "waiting");
  assert.equal(askBucket("stuck: conflict"), "stuck");
  assert.equal(askBucket("something new"), "queued");
});

test("sessions: live jobs, open threads without a job, open asks; whose they are; newest first", () => {
  const out = sessionsOf(
    [
      { job_id: "j1", thread_id: "t1", state: "running", created_by: "u1", brief: "Mine\nmore", updated_at: iso(1_000) },
      { job_id: "j2", state: "running", created_by: "u2", brief: "Theirs", updated_at: iso(2_000) },
      { job_id: "j3", state: "done" },
    ],
    [{ thread_id: "t1", title: "covered by j1" }, { thread_id: "t2", title: "Open thread", created_by: "u2", updated_at: iso(3_000) }, { thread_id: "t3", status: "closed" }],
    [{ id: "a1", state: "queued", title: "Ask", created_by: "u1", updated_at: iso(4_000) }, { id: "a2", state: "verified" }],
    { subject: "u1" },
  );
  assert.deepEqual(out.map((x) => [x.id, x.mine, x.title, x.kind]), [
    ["j1", true, "Mine", "session"],
    ["j2", false, "Theirs", "session"],
    ["t2", false, "Open thread", "session"],
    ["a1", true, "Ask", "ask"],
  ]);
  assert.equal(out[0].thread_id, "t1");
  assert.equal(out[3].thread_id, null, "an ask has no thread to ask about");
});
