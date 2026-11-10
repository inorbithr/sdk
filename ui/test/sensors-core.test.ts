// Sensors, ported from the Mac app (PRD 0117; RFC 0217): health and rate mapping, reduced motion, pins,
// the signal stream and the session redaction.

import assert from "node:assert/strict";
import { test } from "node:test";
import { filterStream } from "../src/lib/event-stream-core.ts";
import {
  diffEvents,
  ease,
  healthOf,
  healthWord,
  hitPlanet,
  isMine,
  isStale,
  mergeEvents,
  moonSlot,
  orbitSpeed,
  ratePerMin,
  rateText,
  redact,
  rollup,
  type Sensor,
  type SessionPreview,
  type Snapshot,
  sparkPoints,
  streamEvent,
  systemLayout,
  tileOrder,
  togglePin,
  valueText,
  zoomFor,
} from "../src/lib/sensors-core.ts";

const NOW = Date.parse("2026-10-14T12:00:00Z");

const sensor = (id: string, health: Sensor["health"], extra: Partial<Sensor> = {}): Sensor => ({
  id,
  name: id,
  source_id: "s",
  kind: "check",
  state: "active",
  health,
  interval_secs: 60,
  rate_per_min: 1,
  last_seen: NOW,
  value: 1,
  unit: "",
  readings: [],
  revocable: false,
  ...extra,
});

// ---------------------------------------------------------------- health

test("health words map onto four colours; unknown and empty are silent", () => {
  assert.equal(healthWord("UP"), "fine");
  assert.equal(healthWord("healthy"), "fine");
  assert.equal(healthWord("warning"), "degraded");
  assert.equal(healthWord("down"), "failing");
  assert.equal(healthWord("critical"), "failing");
  assert.equal(healthWord("banana"), "silent");
  assert.equal(healthWord(""), "silent");
  assert.equal(healthWord(null), "silent");
});

test("stale after three missed intervals; never seen is stale; no interval never goes stale by time", () => {
  assert.equal(isStale(null, 60, NOW), true);
  assert.equal(isStale(NOW - 179_000, 60, NOW), false);
  assert.equal(isStale(NOW - 181_000, 60, NOW), true);
  assert.equal(isStale(NOW - 86_400_000, null, NOW), false);
});

test("paused, revoked and stale sensors are silent whatever they last said", () => {
  const base = { raw: "down", last_seen: NOW, interval_secs: 60 };
  assert.equal(healthOf({ ...base, state: "active" }, NOW), "failing");
  assert.equal(healthOf({ ...base, state: "paused" }, NOW), "silent");
  assert.equal(healthOf({ ...base, state: "revoked" }, NOW), "silent");
  assert.equal(healthOf({ ...base, state: "active", last_seen: NOW - 10 * 60_000 }, NOW), "silent");
});

test("a planet takes its worst speaking sensor", () => {
  assert.equal(rollup(["fine", "degraded", "silent"]), "degraded");
  assert.equal(rollup(["fine", "failing", "degraded"]), "failing");
  assert.equal(rollup(["silent", "fine"]), "fine");
  assert.equal(rollup(["silent"]), "silent");
  assert.equal(rollup([]), "silent");
});

// ---------------------------------------------------------------- rate and motion

test("rate counts the last hour's readings, else 60 / interval, else none", () => {
  const readings = Array.from({ length: 10 }, (_, i) => ({ at: NOW - i * 60_000, value: 1, ok: true }));
  // 10 readings over 9 minutes: 10 / 9 a minute.
  assert.ok(Math.abs(ratePerMin(300, readings, NOW) - 10 / 9) < 1e-9);
  assert.equal(ratePerMin(30, [], NOW), 2);
  assert.equal(ratePerMin(null, [], NOW), 0);
  // Readings older than an hour do not count.
  assert.equal(
    ratePerMin(
      60,
      [
        { at: NOW - 2 * 3_600_000, value: 1, ok: true },
        { at: NOW - 3 * 3_600_000, value: 1, ok: true },
      ],
      NOW,
    ),
    1,
  );
});

test("orbit speed rises with the rate, is capped, and stops for silent sensors", () => {
  const rates = [0.1, 1, 6, 60, 600];
  const speeds = rates.map((r) => orbitSpeed(r, "fine", false));
  for (let i = 1; i < speeds.length; i++)
    assert.ok(speeds[i] >= speeds[i - 1], `speed at ${rates[i]} >= at ${rates[i - 1]}`);
  assert.ok(Math.abs(speeds[3] - 0.98) < 1e-9, "60 a minute is the cap");
  assert.equal(speeds[4], speeds[3], "faster than 60 a minute does not blur further");
  assert.ok(speeds[0] > 0.08);
  assert.equal(orbitSpeed(5, "silent", false), 0);
  assert.equal(orbitSpeed(0, "fine", false), 0);
  assert.equal(orbitSpeed(Number.NaN, "fine", false), 0);
});

test("reduced motion: nothing orbits and the camera jumps", () => {
  for (const h of ["fine", "degraded", "failing"] as const) assert.equal(orbitSpeed(30, h, true), 0);
  assert.equal(ease(0, true), 1);
  assert.equal(ease(0.3, true), 1);
  assert.equal(ease(0, false), 0);
  assert.equal(ease(1, false), 1);
  assert.ok(ease(0.5, false) > 0 && ease(0.5, false) < 1);
});

test("the rate in words", () => {
  assert.equal(rateText(0), "none lately");
  assert.equal(rateText(4), "4 a minute");
  assert.equal(rateText(12.4), "12 a minute");
  assert.equal(rateText(0.2), "1 every 5 min");
  assert.equal(rateText(1 / 120), "1 every 2 h");
});

test("zooming on a planet centres it and never zooms past 6x", () => {
  const z = zoomFor({ id: "a", x: 100, y: 50, r: 10, orbit: 1 }, 800, 600);
  assert.equal(z.k, 6);
  assert.equal(z.x, 100);
  const big = zoomFor({ id: "b", x: 0, y: 0, r: 10, orbit: 200 }, 800, 600);
  assert.ok(Math.abs(big.k - (0.35 * 600) / 200) < 1e-9);
});

// ---------------------------------------------------------------- sessions

test("someone else's session shows only that it runs", () => {
  const s: SessionPreview = {
    id: "j",
    thread_id: "t",
    title: "Secret plan",
    state: "running",
    last_step: "reading",
    starter: "ana@x",
    at: NOW,
    mine: false,
    kind: "session",
  };
  const r = redact(s);
  assert.equal(r.title, "A session is running");
  assert.equal(r.last_step, "");
  assert.equal(r.starter, "");
  assert.equal(redact({ ...s, mine: true }).title, "Secret plan");
  assert.equal(isMine(["NEVIO@inorbit.hr"], { email: "nevio@inorbit.hr" }), true);
  assert.equal(isMine(["ana@x", null], { email: "nevio@inorbit.hr", subject: "u1" }), false);
});

// ---------------------------------------------------------------- control room

test("pinned tiles come first in pin order, then worst first", () => {
  const all = [sensor("a", "fine"), sensor("b", "failing"), sensor("c", "silent"), sensor("d", "degraded")];
  let pins = togglePin([], "c");
  pins = togglePin(pins, "a");
  assert.deepEqual(pins, ["a", "c"]);
  assert.deepEqual(
    tileOrder(all, pins).map((s) => s.id),
    ["a", "c", "b", "d"],
  );
  assert.deepEqual(togglePin(pins, "a"), ["c"]);
  assert.deepEqual(
    tileOrder(all, ["gone"]).map((s) => s.id),
    ["b", "d", "a", "c"],
  );
});

test("the stream: nothing on first look, then new sensors, changes of health and signals", () => {
  const snap = (sensors: Sensor[], at: number): Snapshot => ({
    origin: "api",
    label: "",
    at,
    problems: [],
    sources: [
      { id: "s", name: "Box", kind: "machine", detail: "", health: rollup(sensors.map((s) => s.health)), sensors },
    ],
  });
  const first = snap([sensor("a", "fine", { last_seen: NOW - 1000 })], NOW);
  assert.deepEqual(diffEvents(null, first), []);
  const next = snap([sensor("a", "failing", { last_seen: NOW + 5000 }), sensor("b", "fine")], NOW + 8000);
  const ev = diffEvents(first, next);
  assert.ok(ev.some((e) => e.kind === "change" && e.text === "a on Box: fine → failing"));
  assert.ok(ev.some((e) => e.kind === "change" && e.text === "b appeared on Box"));
  assert.ok(ev.some((e) => e.kind === "signal" && e.sensor_id === "a"));
  for (let i = 1; i < ev.length; i++) assert.ok(ev[i - 1].at >= ev[i].at, "newest first");
  const merged = mergeEvents(ev, ev);
  assert.equal(merged.length, ev.length, "no repeats");
  assert.deepEqual(
    filterStream(merged.map(streamEvent), "s", ["failing"]).map((e) => e.tone),
    filterStream(merged.map(streamEvent), null, ["failing"]).map(() => "failing"),
  );
  assert.deepEqual(filterStream(merged.map(streamEvent), "other", []), []);
  assert.ok(
    merged.map(streamEvent).some((e) => e.strong),
    "a change is said in bold",
  );
});

test("a sparkline needs two values", () => {
  assert.equal(sparkPoints([], 100, 20), null);
  assert.equal(
    sparkPoints(
      [
        { at: 1, value: 3, ok: true },
        { at: 2, value: null, ok: null },
      ],
      100,
      20,
    ),
    null,
  );
  assert.equal(
    sparkPoints(
      [
        { at: 1, value: 1, ok: true },
        { at: 2, value: 3, ok: true },
      ],
      100,
      20,
    ),
    "0.0,19.0 100.0,1.0",
  );
});

test("the layout puts every planet inside the frame and a click finds the planet under it", () => {
  const { placed, scale, tracks } = systemLayout(
    [
      { id: "a", count: 3 },
      { id: "b", count: 12 },
      { id: "c", count: 1 },
      { id: "d", count: 30 },
    ],
    800,
    600,
  );
  assert.equal(placed.length, 4);
  assert.ok(scale > 0 && tracks.length >= 1);
  for (const p of placed)
    assert.ok(
      p.x - p.orbit >= -1 && p.x + p.orbit <= 801 && p.y - p.orbit >= -1 && p.y + p.orbit <= 601,
      `${p.id} fits`,
    );
  assert.equal(hitPlanet(placed, placed[1].x, placed[1].y), placed[1].id);
  assert.equal(hitPlanet(placed, -500, -500), null);
  assert.deepEqual(systemLayout([], 800, 600).placed, []);
  assert.deepEqual(moonSlot(0, 3), { ring: 0, phase: 0 });
  assert.equal(moonSlot(9, 20).ring, 1);
});

test("values read in their unit, and text wins when a number would mislead", () => {
  assert.equal(valueText({ value: 1500, unit: "ms" }), "1.5 s");
  assert.equal(valueText({ value: 90, unit: "ms" }), "90 ms");
  assert.equal(valueText({ value: 41.6, unit: "%" }), "42%");
  assert.equal(valueText({ value: null, unit: "" }), "no reading");
  assert.equal(valueText({ value: 1, unit: "", text: "Normal" }), "Normal");
});
