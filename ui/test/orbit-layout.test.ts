import assert from "node:assert/strict";
import { test } from "node:test";
import { moonAngle, moonOffset, moonRadius, planetAt, sceneExtent } from "../src/lib/orbit-layout.ts";
import type { WorkPlanet } from "../src/lib/workload-map.ts";

const planet = (n: number, speed = 0.7): WorkPlanet => ({
  id: "p",
  name: "P",
  tone: "working",
  badge: "",
  say: "",
  moons: Array.from({ length: n }, (_, i) => ({
    id: `m${i}`,
    name: `m${i}`,
    tone: "working" as const,
    speed,
    say: "",
  })),
});

test("planets sit in a row centred on the origin", () => {
  assert.deepEqual(
    [0, 1, 2].map((i) => planetAt(i, 3).x),
    [-5, 0, 5],
  );
  assert.equal(planetAt(0, 1).x, 0);
});

test("a ring holds eight moons evenly; the ninth starts a wider ring", () => {
  const p = planet(9);
  assert.ok(Math.abs(moonAngle(p, p.moons[1]!, 1, 0) - Math.PI / 4) < 1e-9);
  assert.ok(moonRadius(p, 8) > moonRadius(p, 7));
  assert.equal(moonRadius(p, 0), moonRadius(p, 7));
});

test("time moves a working moon and leaves a still one; t=0 is the reduced-motion pose", () => {
  const p = planet(2);
  const a = moonOffset(p, p.moons[0]!, 0, 0);
  const b = moonOffset(p, p.moons[0]!, 0, 5);
  assert.notDeepEqual(a, b);
  const still = planet(2, 0);
  assert.deepEqual(moonOffset(still, still.moons[0]!, 0, 0), moonOffset(still, still.moons[0]!, 0, 99));
});

test("the extent reaches past the outermost planet and ring", () => {
  assert.ok(sceneExtent([planet(3), planet(3), planet(3)]) > 5 + moonRadius(planet(3), 2));
});
