// The Planner's address in the web UI (RFC 0217): `/planner` is the overview, `/planner/<plan>` one plan and
// `/planner/<plan>/<item>` one item of it. The mode is the query's `mode`. The Mac app's modes are overview,
// progress, write, decisions, explore and the plan canvas; the web adds Plan (the outline) and Preview. Pure.

export const PLANNER = "/planner";

export const MODES = ["plan", "progress", "canvas", "preview", "write", "decisions", "explore"] as const;
export type PlannerMode = (typeof MODES)[number];
export const MODE_LABEL: Record<PlannerMode, string> = {
  plan: "Plan",
  progress: "Progress",
  canvas: "Canvas",
  preview: "Preview",
  write: "Write",
  decisions: "Decisions",
  explore: "Explore",
};
export const DEFAULT_MODE: PlannerMode = "plan";

export type PlannerRoute = { planId: string | null; itemId: string | null; mode: PlannerMode };

const ID = /^[A-Za-z0-9_.:-]{1,120}$/;
const isMode = (v: string | null): v is PlannerMode => v !== null && (MODES as readonly string[]).includes(v);

/** Where a path and query point in the Planner; null when the path is not the Planner's. An id that is no id is not a route. */
export function plannerRoute(pathname: string, search = ""): PlannerRoute | null {
  const m = pathname.match(/^\/planner(?:\/([^/]+))?(?:\/([^/]+))?\/?$/);
  if (!m) return null;
  let planId: string | null = null;
  let itemId: string | null = null;
  try {
    planId = m[1] === undefined ? null : decodeURIComponent(m[1]);
    itemId = m[2] === undefined ? null : decodeURIComponent(m[2]);
  } catch {
    return null;
  }
  if ((planId !== null && !ID.test(planId)) || (itemId !== null && !ID.test(itemId))) return null;
  const q = new URLSearchParams(search).get("mode");
  return { planId, itemId, mode: isMode(q) ? q : DEFAULT_MODE };
}

/** The address of a route: the default mode leaves the query out. */
export function plannerHref(r: Partial<PlannerRoute>): string {
  const base = r.planId
    ? `${PLANNER}/${encodeURIComponent(r.planId)}${r.itemId ? `/${encodeURIComponent(r.itemId)}` : ""}`
    : PLANNER;
  return r.planId && r.mode && r.mode !== DEFAULT_MODE ? `${base}?mode=${r.mode}` : base;
}

/** The mode a digit key picks (1 is Plan ... 7 is Explore); null for any other key. */
export const modeOfDigit = (key: string): PlannerMode | null =>
  /^[1-7]$/.test(key) ? (MODES[Number(key) - 1] as PlannerMode) : null;
