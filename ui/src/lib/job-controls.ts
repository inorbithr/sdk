// The Planner's per-item job controls (ADR 0075, Paused; RFC 0120): which moves a coordinator job's
// mode allows, what holds it, and the jobs a plan item names. Pure (no React, no Tauri) so the
// tests pin it; the coordinator's machine is still the one that decides and refuses the rest.
// The moves go through the app's allowlisted proxy (src-tauri/src/api.rs), five POSTs only.

import { NO_STAGES, prsOfStage, type StageMap } from "./plan-stage-core.ts";
import { type Item, JOB_ID, type PrRef } from "./planner.ts";
import type { JobLike } from "./planner-shared.ts";

/** The five machine moves, and refile: a new ask that names a stopped job. */
export type JobAction = "pause" | "resume" | "retry" | "release" | "cancel" | "refile";
export type JobControl = {
  action: JobAction;
  label: string;
  /** What the move does, for the button's title. */
  hint: string;
  /** The coordinator refuses these without a reason (check_reason), so the page asks for one first. */
  reasonRequired: true;
  tone?: "danger";
};

const TERMINAL = new Set(["done", "stopped"]);

const C: Record<JobAction, JobControl> = {
  pause: {
    action: "pause",
    label: "Pause",
    hint: "Stop the job where it is until you continue it; nothing moves it meanwhile",
    reasonRequired: true,
  },
  resume: {
    action: "resume",
    label: "Continue",
    hint: "Return the job to the mode it was paused from, with that mode's state as it was",
    reasonRequired: true,
  },
  retry: {
    action: "retry",
    label: "Retry now",
    hint: "Fix: run the job's one retry now instead of waiting out the backoff",
    reasonRequired: true,
  },
  release: {
    action: "release",
    label: "Release",
    hint: "Fix: you have dealt with what held it; the job carries on",
    reasonRequired: true,
  },
  cancel: {
    action: "cancel",
    label: "Cancel",
    hint: "Stop the job for good; its compensations run",
    reasonRequired: true,
    tone: "danger",
  },
  refile: {
    action: "refile",
    label: "Refile",
    hint: "Fix: queue a new ask that names this stopped job, so the planner files it again",
    reasonRequired: true,
  },
};

/** The moves this job's mode allows, the fix first. A mode or stage this build does not know offers nothing. */
export function controlsFor(job: JobLike): JobControl[] {
  if (job.stage === "stopped") return [C.refile];
  if (TERMINAL.has(job.stage)) return [];
  switch (job.mode) {
    case "active":
    case "awaiting":
      return [C.pause, C.cancel];
    case "held":
      return [C.release, C.pause, C.cancel];
    case "retry":
      return [C.retry, C.pause, C.cancel];
    case "paused":
      return [C.resume, C.cancel];
    default:
      return [];
  }
}

const stamp = (t: string) => `${t.slice(5, 16).replace("T", " ")}Z`;

/** What holds the job, in a few words; empty when nothing does. */
export function holdCause(job: JobLike): string {
  if (job.stage === "stopped") return "Stopped";
  switch (job.mode) {
    case "held":
      return job.holdReason ? `Held: ${job.holdReason}` : "Held";
    case "retry":
      return job.retryAt && Number.isFinite(Date.parse(job.retryAt))
        ? `Retrying at ${stamp(new Date(job.retryAt).toISOString())}`
        : "Retrying";
    case "awaiting":
      return job.awaitKind ? `Waiting for ${job.awaitKind}` : "Waiting";
    case "paused":
      return job.pausedFrom ? `Paused (was ${job.pausedFrom})` : "Paused";
    default:
      return "";
  }
}

/** The coordinator route of a machine move; null for refile, which queues an ask instead. */
export function actionPath(action: JobAction, jobId: string): string | null {
  return action === "refile" ? null : `/v1/coordinator/jobs/${encodeURIComponent(jobId)}/${action}`;
}

/** Every job an item is about: the ones its `job`/`job_id`/`jobs` name, then the ones its title and body name. */
export function jobRefs(i: Pick<Item, "title" | "body" | "jobs">): string[] {
  const out: string[] = [...(i.jobs ?? [])];
  const re = new RegExp(JOB_ID.source, "g");
  for (const text of [i.title, i.body])
    for (const m of text.matchAll(re)) if (out.length < 40 && !out.includes(m[0])) out.push(m[0]);
  return out;
}

/** ResolvePlanItems takes at most this many refs per call. */
export const PLAN_STATUS_MAX = 100;

/** The refs the plan asks the coordinator about in one ResolvePlanItems call: its items' jobs and PRs (the machine's one
 * parse of each item's pull requests, plan_state.rs `item_prs`, through its stages), dropped items left out. */
export function planStatusRefs(
  items: Pick<Item, "id" | "title" | "body" | "jobs" | "status">[],
  stages: StageMap = NO_STAGES,
): string[] {
  const out: string[] = [];
  const add = (r: string) => {
    if (out.length < PLAN_STATUS_MAX && !out.includes(r)) out.push(r);
  };
  for (const i of items) {
    if (i.status === "dropped") continue;
    for (const j of jobRefs(i)) add(j);
    for (const p of prsOfStage(stages.get(i.id))) add(`${p.repo}#${p.number}`);
  }
  return out;
}

/** The jobs an item is: the ones it names, then the ones its pull requests (`prs`, the machine's parse) resolve to
 * (ResolvePlanItems), each once. A dropped item has none. */
export function itemJobs(
  i: Pick<Item, "title" | "body" | "jobs" | "status">,
  statuses: { ref: string; jobId: string }[],
  prs: readonly PrRef[] = [],
): string[] {
  if (i.status === "dropped") return [];
  const by = new Map(statuses.map((s) => [s.ref, s.jobId]));
  const out: string[] = [];
  for (const r of [...jobRefs(i), ...prs.map((p) => `${p.repo}#${p.number}`)]) {
    const j = by.get(r) || (JOB_ID.test(r) ? r : "");
    if (j && !out.includes(j)) out.push(j);
  }
  return out;
}
