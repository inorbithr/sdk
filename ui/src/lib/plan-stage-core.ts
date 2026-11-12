// The one stage of a plan item for every view, pure (no React, no Tauri) so the canvas and the tests can use it.
// The Rust plan machine (src-tauri/src/plan_machine.rs, plan_state.rs) computes it; plan-stage.ts reads it.

import type { PrRef } from "./planner.ts";
import { duration } from "./planner-progress.ts";
import type { PrLive } from "./planner-shared.ts";

export type StageId = "proposed" | "ready" | "building" | "on_preview" | "merged" | "rolled" | "verified" | "dropped";
export type StageInfo = {
  id: string;
  stage: StageId;
  label: string;
  file_status: string;
  since: string;
  wait: { who: string; what: string } | null;
  wait_since: string;
  evidence: string[];
  children: string[];
  /** Every pull request the item is about (Rust's one parse, plan_state.rs `item_prs`): its `pr`/`prs`, then its title and body. */
  prs: PrRef[];
  /** Stuck by the one rule (plan_state.rs `ItemStage::stuck`): a pull request closed without merging, the item not merged. */
  stuck?: boolean;
};
export type StageMap = ReadonlyMap<string, StageInfo>;
export const NO_STAGES: StageMap = new Map();

/** The pull requests of an item, as the machine parsed them; none while the machine has not answered. */
export const prsOfStage = (s: StageInfo | undefined): PrRef[] => s?.prs ?? [];

const prKey = (p: PrRef) => `${p.repo}#${p.number}`;

/** "repo#12" to what the machine read (plan_state's `prs`, the app's one PR cache): every view's PR rows read this, so a
 * row never says "not read yet" while its stage rests on a read. */
export const prMap = (read: readonly PrLive[]): Map<string, PrLive> => new Map(read.map((p) => [prKey(p), p]));

/** "On preview for 12 min · waiting on you: review". Same words in every view. */
export function stageLine(s: StageInfo | undefined, now = Date.now()): string {
  if (!s) return "";
  const t = Date.parse(s.since);
  const parts = [Number.isFinite(t) ? `${s.label} for ${duration(now - t)}` : s.label];
  if (s.wait) {
    const w = Date.parse(s.wait_since);
    parts.push(
      `waiting on ${s.wait.who}${s.wait.what ? `: ${s.wait.what}` : ""}${Number.isFinite(w) ? ` (${duration(now - w)})` : ""}`,
    );
  }
  return parts.join(" · ");
}

/** "On preview since 08:48": the stage and when it began, no running count (the canvas must not change each minute). */
export function stageStamp(s: StageInfo): string {
  const t = Date.parse(s.since);
  const at = Number.isFinite(t) ? ` since ${new Date(t).toISOString().slice(5, 16).replace("T", " ")}Z` : "";
  return `${s.label}${at}${s.wait ? ` · waiting on ${s.wait.who}${s.wait.what ? `: ${s.wait.what}` : ""}` : ""}`;
}

/** The stage label alone, for the tight spots. */
export const stageLabel = (s: StageInfo | undefined, fallback: string): string => s?.label ?? fallback;
