// A plan item's pull requests as one line each (PRD 0111): title, state, checks. Pure; the page
// reads the state with the app's own gh (decisions.pr, control_pr_status) and hands it in here.

import { prLabel } from "./planner-shared.ts";

export type PrNow = {
  title?: string;
  state: string;
  draft: boolean;
  review: string;
  passed: number;
  failed: number;
  pending: number;
};

/** "Add export · open · 4 passed" / "Add export · 1 check failing" / "Add export · merged". */
export function prLine(p: PrNow): string {
  const label = prLabel(p).text;
  const checks =
    p.state === "OPEN" && !p.draft && p.failed === 0 && p.pending === 0 && p.passed > 0 ? ` · ${p.passed} passed` : "";
  const title = (p.title ?? "").replace(/\s+/g, " ").trim().slice(0, 90);
  return `${title ? `${title} · ` : ""}${label}${checks}`;
}
