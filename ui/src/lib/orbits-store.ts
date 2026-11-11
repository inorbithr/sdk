import { type Orbits, parseOrbits } from "./orbits-core.ts";
import { stored } from "./stored.ts";

// This page's orbits: the pages picked, in order (lib/orbits-core.ts, lib/stored.ts).
const orbits = stored<Orbits>("iohr-ui.orbits", (o) => parseOrbits(o));

export const updateOrbits = orbits.update;
export const useOrbits = orbits.use;
