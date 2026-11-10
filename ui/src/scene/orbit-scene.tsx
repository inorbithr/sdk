import { useFrame } from "@react-three/fiber";
import { useRef } from "react";
import type { Group } from "three";
import { moonOffset, moonRadius, planetAt, planetRadius, sceneExtent } from "../lib/orbit-layout.ts";
import { STATE_COLOR, type WorkMoon, type WorkPlanet } from "../lib/workload-map.ts";
import SceneCanvas from "./scene-canvas.tsx";

function Moon({
  planet,
  moon,
  k,
  reduced,
  picked,
  onPick,
}: {
  planet: WorkPlanet;
  moon: WorkMoon;
  k: number;
  reduced: boolean;
  picked: boolean;
  onPick: (id: string) => void;
}) {
  const ref = useRef<Group>(null);
  const place = (t: number) => {
    const o = moonOffset(planet, moon, k, t);
    ref.current?.position.set(o.x, 0, o.z);
  };
  useFrame(({ clock }) => {
    if (!reduced) place(clock.elapsedTime);
  });
  const at = moonOffset(planet, moon, k, 0);
  return (
    <group ref={ref} position={[at.x, 0, at.z]}>
      {/* biome-ignore lint/a11y/noStaticElementInteractions: a three.js mesh, not a DOM element; the keyboard path is the room and tree views */}
      <mesh
        onClick={(e) => {
          e.stopPropagation();
          onPick(moon.id);
        }}
        scale={picked ? 1.6 : 1}
      >
        <sphereGeometry args={[0.14, 16, 16]} />
        <meshStandardMaterial
          color={STATE_COLOR[moon.tone]}
          emissive={STATE_COLOR[moon.tone]}
          emissiveIntensity={picked ? 0.9 : 0.25}
        />
      </mesh>
    </group>
  );
}

function Ring({ planet, k }: { planet: WorkPlanet; k: number }) {
  const r = moonRadius(planet, k);
  return (
    <mesh rotation={[Math.PI / 2, 0, 0]}>
      <ringGeometry args={[r - 0.008, r + 0.008, 64]} />
      <meshBasicMaterial color="#64748b" transparent opacity={0.35} />
    </mesh>
  );
}

/** The workload orbit in 3D: a planet per group, its work in orbit. Click a planet or a moon to pick it. */
export default function OrbitScene({
  planets,
  picked,
  onPick,
  reduced,
}: {
  planets: WorkPlanet[];
  picked: string | null;
  onPick: (id: string) => void;
  reduced: boolean;
}) {
  return (
    <SceneCanvas
      reduced={reduced}
      extent={sceneExtent(planets)}
      label={`Workload orbit: ${planets.map((p) => p.say).join(" ")}`}
    >
      {planets.map((p, i) => {
        const c = planetAt(i, planets.length);
        const rings = new Set(p.moons.map((_, k) => Math.floor(k / 8)));
        return (
          <group key={p.id} position={[c.x, 0, c.z]}>
            {/* biome-ignore lint/a11y/noStaticElementInteractions: a three.js mesh, not a DOM element; the keyboard path is the room and tree views */}
            <mesh
              onClick={(e) => {
                e.stopPropagation();
                onPick(p.id);
              }}
            >
              <sphereGeometry args={[planetRadius(p), 32, 32]} />
              <meshStandardMaterial
                color={STATE_COLOR[p.tone]}
                emissive={STATE_COLOR[p.tone]}
                emissiveIntensity={picked === p.id ? 0.7 : 0.2}
              />
            </mesh>
            {[...rings].map((ring) => (
              <Ring key={ring} planet={p} k={ring * 8} />
            ))}
            {p.moons.map((m, k) => (
              <Moon key={m.id} planet={p} moon={m} k={k} reduced={reduced} picked={picked === m.id} onPick={onPick} />
            ))}
          </group>
        );
      })}
    </SceneCanvas>
  );
}
