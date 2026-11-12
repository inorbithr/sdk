// Where the orbit view puts things, in scene units on the x/z plane. Pure, so the 3D scene and the flat
// (no WebGL) view draw the same picture and a test can check the maths.
import type { WorkMoon, WorkPlanet } from "./workload-map.ts";

export const PLANET_GAP = 5;
const RING_SIZE = 8;

/** A planet's centre: the planets in a row, centred on the origin. */
export const planetAt = (index: number, count: number): { x: number; z: number } => ({
  x: (index - (count - 1) / 2) * PLANET_GAP,
  z: 0,
});

/** A planet's radius: more moons, a bigger planet, up to a cap. */
export const planetRadius = (p: WorkPlanet): number => 0.45 + Math.min(0.4, p.moons.length * 0.02);

/** A moon's ring (0 is the inner) and its orbit radius: eight to a ring, rings further out. */
export const moonRing = (k: number): number => Math.floor(k / RING_SIZE);
export const moonRadius = (p: WorkPlanet, k: number): number => planetRadius(p) + 0.55 + moonRing(k) * 0.4;

/** A moon's angle at time `t` seconds: its slot on the ring plus its speed. `t` stays 0 for reduced motion. */
export function moonAngle(p: WorkPlanet, moon: WorkMoon, k: number, t: number): number {
  const inRing = Math.min(RING_SIZE, p.moons.length - moonRing(k) * RING_SIZE);
  return ((k % RING_SIZE) / inRing) * Math.PI * 2 + moon.speed * t;
}

/** A moon's offset from its planet's centre at time `t`. */
export function moonOffset(p: WorkPlanet, moon: WorkMoon, k: number, t = 0): { x: number; z: number } {
  const a = moonAngle(p, moon, k, t);
  const r = moonRadius(p, k);
  return { x: Math.cos(a) * r, z: Math.sin(a) * r };
}

/** How far out the scene reaches, so the camera frames every planet and ring. */
export function sceneExtent(planets: readonly WorkPlanet[]): number {
  const widest = planets.reduce((m, p) => Math.max(m, moonRadius(p, Math.max(0, p.moons.length - 1))), 1);
  return ((Math.max(1, planets.length) - 1) / 2) * PLANET_GAP + widest + 0.5;
}
