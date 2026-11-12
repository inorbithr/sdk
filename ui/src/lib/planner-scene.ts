// The Planner's 3D view as a pure model (RFC 0217): each item of the plan is a node on a tree laid out on the ground
// plane (depth along x, outline order along z), its colour the stage the machine reports, its height the number of
// children below it. No three.js here: the page hands `nodes` and `edges` to the one scene canvas (src/scene, #523)
// once that has landed, and until then draws the same plan as the outline. Node colours never claim "on preview" or
// "verified" unless the machine's stage says so. Node tests cover it.

import type { StageId, StageMap } from "./plan-stage-core.ts";
import { type Item, outline, type Plan } from "./planner.ts";

export type SceneNode = {
  id: string;
  title: string;
  x: number;
  z: number;
  height: number;
  tone: "idle" | "good" | "warn" | "bad";
  label: string;
};
export type SceneEdge = { from: string; to: string };
export type PlanScene = { nodes: SceneNode[]; edges: SceneEdge[]; extent: number };

/** Spacing between neighbours on the ground, in scene units. */
export const GAP = 2;

const TONE: Record<StageId, SceneNode["tone"]> = {
  proposed: "idle",
  ready: "warn",
  building: "warn",
  on_preview: "good",
  merged: "good",
  rolled: "good",
  verified: "good",
  dropped: "bad",
};

const isWork = (i: Item) => i.kind !== "note";

/** The plan as a scene: nodes centred on the origin so the camera frames them, and the parent-to-child edges. */
export function planScene(plan: Plan, stages: StageMap): PlanScene {
  const rows = outline(plan).filter((r) => isWork(r.item));
  const ids = new Set(rows.map((r) => r.item.id));
  const kids = new Map<string, number>();
  const edges: SceneEdge[] = [];
  for (const { item } of rows) {
    if (item.parent && ids.has(item.parent) && item.parent !== item.id) {
      kids.set(item.parent, (kids.get(item.parent) ?? 0) + 1);
      edges.push({ from: item.parent, to: item.id });
    }
  }
  const depth = Math.max(0, ...rows.map((r) => r.depth));
  const nodes = rows.map(({ item, depth: d }, n) => {
    const st = stages.get(item.id);
    return {
      id: item.id,
      title: item.title,
      x: (d - depth / 2) * GAP,
      z: (n - (rows.length - 1) / 2) * GAP,
      height: 1 + (kids.get(item.id) ?? 0) * 0.5,
      tone: st ? TONE[st.stage] : item.status === "done" ? "good" : "idle",
      label: st?.label ?? item.status,
    } satisfies SceneNode;
  });
  return { nodes, edges, extent: Math.max(4, rows.length * GAP * 0.6, depth * GAP) };
}
