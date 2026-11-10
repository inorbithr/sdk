// How far along a plan is (PRD 0111): every step in order with its stage, where the plan is now,
// percent and counts, elapsed time and time in the current step. Pure and tested
// (tests/planner-progress.test.ts); every value comes from the plan file (status, needs, the
// time of the line that set them) or from the pull request state the page already reads
// (plan_state, through the app's one PR cache). Nothing is estimated: what has no source is left out, not guessed.

import { needsYou, outline, type Item, type Plan } from "./planner.ts";
import { prLabel } from "./planner-shared.ts";
import type { PrLive } from "./planner-shared.ts";
import type { StageInfo, StageMap } from "./plan-stage-core.ts";

export const STAGES = ["not-started", "running", "owner", "pr", "done", "failed"] as const;
export type Stage = (typeof STAGES)[number];

export const STAGE_LABEL: Record<Stage, string> = {
  "not-started": "Not started",
  running: "Running",
  owner: "Waiting on you",
  pr: "Waiting on a PR or check",
  done: "Done",
  failed: "Failed",
};

/** The machine's stage (plan_machine.rs) onto the groups this view colours and counts. The one table. */
const GROUP: Record<string, Stage> = { proposed: "not-started", ready: "not-started", building: "running", on_preview: "pr", merged: "done", rolled: "done", verified: "done" };

/** Kinds that are containers or scratch, not work: the goal holds the steps; a note is yours to jot. */
const NOT_A_STEP = new Set(["goal", "note"]);

export type StepPr = { repo: string; number: number; live?: PrLive };

export type Step = {
  item: Item;
  depth: number;
  stage: Stage;
  /** One line on why it is in that stage, from the data that put it there. */
  why: string;
  /** When it entered this stage, when a line said so. */
  since?: string;
  prs: StepPr[];
  /** The machine's stage for this item (label, since, wait); absent when the plan_state command could not be read. */
  info?: StageInfo;
};

/** The pull requests a step offers Approve and Deny for: only those GitHub says are open. A merged,
 * closed or not yet read one has nothing to approve. */
export const decidablePrs = (prs: StepPr[]): StepPr[] => prs.filter((p) => p.live?.state === "OPEN");

export type Progress = {
  steps: Step[];
  /** Steps counted: dropped ones, goals and notes are not. */
  total: number;
  done: number;
  /** Whole percent done; null when there are no steps yet. Never 100 until every step is done. */
  percent: number | null;
  byStage: Record<Stage, number>;
  dropped: number;
  /** Where the plan is now: the first running step, else the first one not done. Null when all are done or there are none. */
  current: Step | null;
  /** The steps that wait on the owner, on a PR or check, or have failed. */
  waiting: Step[];
  complete: boolean;
  started?: string;
  /** When the last step was done, once the plan is complete. */
  finished?: string;
  /** Plan start to now (or to the finish); null when the file has no dated line. */
  elapsedMs: number | null;
  /** The current step in its stage until now; null when it has no time. */
  stepMs: number | null;
};

const key = (p: { repo: string; number: number }) => `${p.repo}#${p.number}`;

/** One item's stage and the reason for it, from the plan machine's stage when it has one. PR state counts only when it was read; an unread PR changes nothing. */
export function stageOf(item: Item, prs: StepPr[], info?: StageInfo): { stage: Stage; why: string } {
  if (info && GROUP[info.stage]) {
    // Stuck is the machine's one rule (plan_state.rs); an older machine without the field gets the same rule here.
    const closed = prs.find((p) => p.live?.state === "CLOSED");
    if (info.stuck ?? (!!closed && GROUP[info.stage] !== "done")) return { stage: "failed", why: closed ? `#${closed.number} was closed without merging` : "A pull request was closed without merging" };
    if (info.wait && GROUP[info.stage] !== "done") return { stage: "owner", why: `Waiting on ${info.wait.who}${info.wait.what ? `: ${info.wait.what}` : ""}` };
    const open = prs.find((p) => p.live?.state === "OPEN");
    return { stage: GROUP[info.stage], why: open?.live ? `#${open.number}: ${prLabel(open.live).text}` : info.label };
  }
  // Before the machine answers there are no pull requests to read (they come with its stages): the file's status decides.
  if (item.status === "done") return { stage: "done", why: "Marked done" };
  if (needsYou(item)) return { stage: "owner", why: item.needs === "You" ? "Waiting on you" : (item.needs ?? "Waiting on you") };
  if (item.status === "ready") return { stage: "owner", why: "Ready for your review" };
  if (item.status === "building") return { stage: "running", why: "Being built" };
  return { stage: "not-started", why: "Proposed" };
}

export function progressOf(plan: Plan, prNow: Map<string, PrLive> = new Map(), now = Date.now(), stages?: StageMap): Progress {
  const byStage = Object.fromEntries(STAGES.map((s) => [s, 0])) as Record<Stage, number>;
  let dropped = 0;
  const steps: Step[] = [];
  for (const { item, depth } of outline(plan)) {
    if (NOT_A_STEP.has(item.kind)) continue;
    if (item.status === "dropped") {
      dropped++;
      continue;
    }
    const info = stages?.get(item.id);
    // The pull requests come from the machine's one parse (plan_state.rs); none until it answers.
    const prs = (info?.prs ?? []).map((p) => ({ ...p, live: prNow.get(key(p)) }));
    const { stage, why } = stageOf(item, prs, info);
    // The time it entered this stage: the machine's when it has one; else waiting on the owner counts from the needs line, else from the status line.
    const since = info?.since || (stage === "owner" ? (item.needsAt ?? item.since) : item.since);
    byStage[stage]++;
    steps.push({ item, depth, stage, why, ...(since ? { since } : {}), prs, ...(info ? { info } : {}) });
  }
  const total = steps.length;
  const done = byStage.done;
  const complete = total > 0 && done === total;
  const current = steps.find((s) => s.stage === "running") ?? steps.find((s) => s.stage !== "done") ?? null;
  const waiting = steps.filter((s) => s.stage === "owner" || s.stage === "pr" || s.stage === "failed");
  const doneTimes = steps.flatMap((s) => (s.since && Number.isFinite(Date.parse(s.since)) ? [Date.parse(s.since)] : []));
  const finished = complete && doneTimes.length === total ? new Date(Math.max(...doneTimes)).toISOString() : undefined;
  const t0 = plan.started ? Date.parse(plan.started) : NaN;
  const end = finished ? Date.parse(finished) : now;
  const t1 = current?.since ? Date.parse(current.since) : NaN;
  return {
    steps,
    total,
    done,
    percent: total === 0 ? null : complete ? 100 : Math.min(99, Math.floor((done / total) * 100)),
    byStage,
    dropped,
    current,
    waiting,
    complete,
    ...(plan.started ? { started: plan.started } : {}),
    ...(finished ? { finished } : {}),
    elapsedMs: Number.isFinite(t0) ? Math.max(0, end - t0) : null,
    stepMs: current && Number.isFinite(t1) ? Math.max(0, now - t1) : null,
  };
}

/** "45 s", "12 min", "2 h 14 min", "3 d 4 h": the two largest units, whole numbers. */
export function duration(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000));
  if (s < 60) return `${s} s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m} min`;
  const h = Math.floor(m / 60);
  if (h < 24) return m % 60 ? `${h} h ${m % 60} min` : `${h} h`;
  const d = Math.floor(h / 24);
  return h % 24 ? `${d} d ${h % 24} h` : `${d} d`;
}
