import { type Arrangement, parseArrangement } from "../frame/arrange.ts";
import { stored } from "./stored.ts";

// This page's arrangement: where the page and its dock sit and in which order (frame/arrange.ts, lib/stored.ts).
const arrangement = stored<Arrangement>("iohr-ui.arrangement", parseArrangement);

export const updateArrangement = arrangement.update;
export const useArrangement = arrangement.use;
