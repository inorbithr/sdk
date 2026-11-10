// The Planner's context menu as menu rows (frame/context-menu-core.ts), after the Mac app's plan and item menus
// (iohr-mac src/components/planner/*). Pure: the caller says what each row does, so node tests the rows and when
// they are off.

import { group, type MenuEntry } from "../frame/context-menu-core.ts";
import { MODE_LABEL, MODES, type PlannerMode, type PlannerRoute, plannerHref } from "./planner-route.ts";

export type PlannerActions = {
  go: (href: string) => void;
  copy: (text: string) => void;
};

/** The rows for the place the Planner is at: the modes (a check on this one), the plan canvas and Decisions, then copy the link. */
export function plannerMenu(r: PlannerRoute, act: PlannerActions): MenuEntry[] {
  const planId = r.planId;
  const none = "Open a plan first";
  const here = plannerHref(r);
  return group(
    MODES.map((m: PlannerMode) => ({
      id: `planner:mode:${m}`,
      label: MODE_LABEL[m],
      checked: planId !== null && m === r.mode,
      disabled: planId === null ? none : false,
      run: () => planId !== null && act.go(plannerHref({ planId, itemId: r.itemId, mode: m })),
    })),
    [
      { id: "planner:overview", label: "All plans", icon: "left", run: () => act.go(plannerHref({})) },
      { id: "planner:waiting", label: "What waits on you in Decisions", run: () => act.go("/decisions/~waiting") },
    ],
    [{ id: "planner:copy", label: "Copy the link", run: () => act.copy(here) }],
  );
}
