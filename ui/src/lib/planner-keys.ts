// The Planner's key handlers, keyed by the binding ids of src/shortcuts/pages/planner.keys.json (the shortcuts
// registry, #524): 1 to 7 switch the mode, `/` goes to the overview's search. Pure: the page passes `go` and `focus`
// and hands the result to `useShortcuts("planner", ...)`. The terminal's `p` opens Approvals; the web's Planner key is
// the frame's `g p`, not set here.

import { MODES, plannerHref, type PlannerRoute } from "./planner-route.ts";

export type PlannerKeyActions = { go: (href: string) => void; focusSearch: () => void };

export function plannerHandlers(r: PlannerRoute, act: PlannerKeyActions): Record<string, () => void> {
  const out: Record<string, () => void> = { "planner-search": act.focusSearch };
  if (r.planId === null) return out;
  const planId = r.planId;
  for (const m of MODES) out[`planner-mode-${m}`] = () => act.go(plannerHref({ planId, itemId: r.itemId, mode: m }));
  return out;
}
