// The Planner Overview's data (RFC 0166 section 3): one summary per plan, the header totals, the
// sort, and the fold of the platform's plan stream (snapshot, delta, ended, heartbeat). Pure, so it
// is unit-tested (tests/plans-overview.test.ts); plans-live.ts reads the two sources and
// overview-mode.tsx draws it.
//
// Two sources fill the same shape. Live, the platform derives each summary (plan_summary::derive)
// and pushes it; this file only folds the frames. From files, the summary is derived here from the
// plan file with the same progress computation Progress mode uses (planner-progress.ts), and
// `source` says so. A field the files cannot know is left at its "unknown" value, never guessed:
// no team, no release stage ("not_observed"), no job runner.

import { snake } from "./plan-api.ts";
import type { PrLive } from "./planner-shared.ts";
import { line, liveItems, type Item, type Plan, type PlanState } from "./planner.ts";
import { STAGES, type Progress, type Stage } from "./planner-progress.ts";

export const PROGRESS = ["not_started", "running", "waiting_on_person", "waiting_on_pr", "done", "failed"] as const;
export type OverviewStage = (typeof PROGRESS)[number];
export type StageCounts = Record<OverviewStage, number>;

/** The Overview's six stages and the strip's (planner-progress.ts) are the same six, under two spellings. */
export const TO_STAGE: Record<OverviewStage, Stage> = {
  not_started: "not-started",
  running: "running",
  waiting_on_person: "owner",
  waiting_on_pr: "pr",
  done: "done",
  failed: "failed",
};
export const FROM_STAGE: Record<Stage, OverviewStage> = Object.fromEntries(Object.entries(TO_STAGE).map(([k, v]) => [v, k])) as Record<Stage, OverviewStage>;

export type WaitingOn = { kind: "none" | "person" | "pr" | "check" | "approval"; ref: string; ask_id: string; since: string };
export type PlanPull = { ref: string; state: string; stub: boolean };
export type ReleaseStage = "none" | "merged" | "rolled" | "verified" | "not_observed";

/** RFC 0166 `PlanSummary`. (Named for the Overview: planner-store.ts has an older `PlanSummary` for the file list.) */
export type OverviewPlan = {
  plan_id: string;
  title: string;
  owner: string;
  team: string;
  state: "active" | "paused" | "archived";
  progress: OverviewStage;
  counts: StageCounts;
  waiting_on: WaitingOn;
  running_jobs: number;
  running_tool: string;
  heartbeat_at: string;
  prs: PlanPull[];
  release_stage: ReleaseStage;
  stuck: string[];
  stub: boolean;
  updated_at: string;
  seq: number;
};

export type OverviewTotals = {
  plans: number;
  by_progress: StageCounts;
  waiting_on_me: number;
  running_jobs: number;
  open_prs: number;
  stuck: number;
};

/** Where the rows came from. Files is what this Mac reads today; live is the platform's plan stream. */
export type Source = "files" | "live";

/** A row as the local store keeps it (RFC 0188): the summary and the lowercase text one search runs over. */
export type PlanRow = OverviewPlan & { search: string };

/** What the store answers to `plans_overview` (src-tauri/src/plans.rs): the rows that match, the totals over every plan that is not archived, and how many plans there are in all. */
export type StoreOverview = { plans: unknown[]; totals: unknown; all: number; archived: number; sources: string[] };

/**
 * The text a search runs over, lowercase: the title, the plan id, the owner and team, what it waits
 * on and the ask's id, every pull request ref, the stuck item ids, and `extra` (from files: every
 * item's title, id, writer, jobs and needs). One space-joined text; the store matches every word.
 */
export function searchText(p: OverviewPlan, extra: string[] = []): string {
  const parts = [p.title, p.plan_id, p.owner, p.team, p.state, p.progress.replace(/_/g, " "), p.waiting_on.ref, p.waiting_on.ask_id, p.release_stage, ...p.prs.map((x) => x.ref), ...p.stuck, ...extra];
  return parts
    .filter((s) => typeof s === "string" && s.trim())
    .join(" ")
    .replace(/\s+/g, " ")
    .toLowerCase()
    .slice(0, 16_000);
}

/** The words of a plan's items a search should find: titles, ids, who wrote them, the jobs and what they need. */
export const itemWords = (plan: Plan): string[] => plan.items.flatMap((i) => [i.title, i.id, i.by, ...(i.jobs ?? []), i.needs ?? ""]);

/** A store row from a summary; from files the items' words go into the search text too. */
export const rowOf = (p: OverviewPlan, extra: string[] = []): PlanRow => ({ ...p, search: searchText(p, extra) });

/** The words of `q` every row must contain (any order, lowercase); the store applies the same rule (plans.rs `matches`). */
export const matchesSearch = (search: string, q: string): boolean => q.toLowerCase().split(/\s+/).filter(Boolean).every((w) => search.includes(w));

export const zeroCounts = (): StageCounts => ({ not_started: 0, running: 0, waiting_on_person: 0, waiting_on_pr: 0, done: 0, failed: 0 });

// ---------------------------------------------------------------- from a plan file

/** Decides the plan's single progress word, in the RFC's order: failed, done, waiting on a person, waiting on a PR, running, not started. */
export function progressWord(c: StageCounts): OverviewStage {
  const total = PROGRESS.reduce((n, s) => n + c[s], 0);
  if (total === 0) return "not_started";
  if (c.failed > 0 && c.running === 0) return "failed";
  if (c.done === total) return "done";
  if (c.waiting_on_person > 0) return "waiting_on_person";
  if (c.waiting_on_pr > 0) return "waiting_on_pr";
  if (c.running > 0) return "running";
  return "not_started";
}

export type FromFile = {
  /** The pull request state was a sample (the mock layer, outside the app), not read from GitHub. */
  prsStub?: boolean;
  /** The ask in the ledger that carries an item's wait, when there is one. */
  askFor?: (item: Item) => string | undefined;
  now?: number;
};

const prRef = (p: { repo: string; number: number }) => `${p.repo}#${p.number}`;
const prState = (l?: PrLive) => (!l ? "not_read" : l.state === "OPEN" ? (l.draft ? "draft" : "open") : l.state.toLowerCase());

/** A sample plan is labelled as one in its own title ("Sample: ..."): the Overview marks it stub whatever else is true. */
export const isSampleTitle = (t: string) => /^sample\b/i.test(t.trim());

/**
 * The summary of one plan from its file. `p` is the progress the plan already has (progressOf), so
 * the Overview and the Progress mode can never disagree on a step.
 */
export function summaryFromFile(planId: string, plan: Plan, p: Progress, o: FromFile = {}): OverviewPlan {
  const now = o.now ?? Date.now();
  const counts = zeroCounts();
  for (const s of STAGES) counts[FROM_STAGE[s]] = p.byStage[s];

  const waiting = p.waiting.find((s) => s.stage === "owner") ?? p.waiting.find((s) => s.stage === "pr");
  let waiting_on: WaitingOn = { kind: "none", ref: "", ask_id: "", since: "" };
  if (waiting?.stage === "owner") waiting_on = { kind: "person", ref: waiting.item.title, ask_id: o.askFor?.(waiting.item) ?? "", since: waiting.since ?? "" };
  else if (waiting?.stage === "pr") {
    const open = waiting.prs.find((x) => x.live?.state === "OPEN") ?? waiting.prs[0];
    waiting_on = { kind: "pr", ref: open ? prRef(open) : waiting.item.title, ask_id: "", since: waiting.since ?? "" };
  }

  const seen = new Map<string, PlanPull>();
  const stuck: string[] = [];
  for (const s of p.steps) {
    for (const x of s.prs) {
      const key = prRef(x);
      if (!seen.has(key)) seen.set(key, { ref: key, state: prState(x.live), stub: !!o.prsStub && !!x.live });
    }
    // Stuck is the machine's rule (plan_state.rs `stuck`), which stageOf turns into "failed": one rule for every view.
    if (s.stage === "failed") stuck.push(s.item.id);
  }

  const live = [...liveItems(plan, now).entries()];
  const heartbeat_at = live.map(([, v]) => v.at).sort().pop() ?? "";
  const stub = isSampleTitle(plan.title) || [...seen.values()].some((x) => x.stub);
  return {
    plan_id: planId,
    title: plan.title || planId,
    owner: "",
    team: "",
    state: plan.state ?? "active",
    progress: progressWord(counts),
    counts,
    waiting_on,
    running_jobs: live.length,
    running_tool: "",
    heartbeat_at,
    prs: [...seen.values()],
    release_stage: "not_observed",
    stuck,
    stub,
    updated_at: plan.updated,
    seq: 0,
  };
}

// ---------------------------------------------------------------- totals and sort

/** The header numbers over `plans`; an archived plan never counts; stub PR reads never count (a placeholder is not a measurement); `waiting_on_me` counts the items that wait on the signed-in person. */
export function totalsOf(plans: OverviewPlan[]): OverviewTotals {
  const by = zeroCounts();
  let waiting = 0;
  let jobs = 0;
  let prs = 0;
  let stuck = 0;
  let n = 0;
  for (const p of plans) {
    if (p.state === "archived") continue;
    n++;
    by[p.progress]++;
    waiting += p.counts.waiting_on_person;
    jobs += p.running_jobs;
    prs += p.prs.filter((x) => !x.stub && (x.state === "open" || x.state === "draft")).length;
    stuck += p.stuck.length;
  }
  return { plans: n, by_progress: by, waiting_on_me: waiting, running_jobs: jobs, open_prs: prs, stuck };
}

/** Rows first that need a person, then stuck or failed, running, waiting on a PR, not started, done; the newest first inside each. */
const rank = (p: OverviewPlan): number =>
  p.progress === "waiting_on_person" ? 0 : p.stuck.length > 0 || p.progress === "failed" ? 1 : p.progress === "running" ? 2 : p.progress === "waiting_on_pr" ? 3 : p.progress === "not_started" ? 4 : 5;

export function sortPlans(plans: OverviewPlan[]): OverviewPlan[] {
  return [...plans].sort((a, b) => rank(a) - rank(b) || Date.parse(b.updated_at || "") - Date.parse(a.updated_at || "") || a.plan_id.localeCompare(b.plan_id));
}

// ---------------------------------------------------------------- the platform's stream

export type Live = {
  plans: Map<string, OverviewPlan>;
  totals: OverviewTotals | null;
  /** The newest journal seq folded; resume point after a drop. */
  seq: number;
  tier: string;
  /** False until a snapshot arrived, and again after an `ended` that asks for a new one. */
  ready: boolean;
  generated_at: string;
  /** When the last frame (a heartbeat counts) arrived, ms. */
  lastAt: number;
};

export const emptyLive = (): Live => ({ plans: new Map(), totals: null, seq: 0, tier: "", ready: false, generated_at: "", lastAt: 0 });

const isObj = (v: unknown): v is Record<string, unknown> => !!v && typeof v === "object" && !Array.isArray(v);
const num = (v: unknown) => (typeof v === "number" ? v : typeof v === "string" && /^-?\d+$/.test(v) ? Number(v) : 0);
const str = (v: unknown) => (typeof v === "string" ? v : "");

function counts(v: unknown): StageCounts {
  const o = isObj(v) ? v : {};
  return Object.fromEntries(PROGRESS.map((s) => [s, num(o[s])])) as StageCounts;
}

/** One plan summary from a frame; null when it has no plan id (not a summary). Unknown words are kept as they are, never mapped to a good one. */
export function readPlan(raw: unknown): OverviewPlan | null {
  const o = snake(raw);
  if (!isObj(o) || !str(o.plan_id)) return null;
  const w = isObj(o.waiting_on) ? o.waiting_on : {};
  const progress = PROGRESS.find((s) => s === o.progress) ?? "not_started";
  return {
    plan_id: str(o.plan_id),
    title: str(o.title) || str(o.plan_id),
    owner: str(o.owner),
    team: str(o.team),
    state: o.state === "paused" || o.state === "archived" ? o.state : "active",
    progress,
    counts: counts(o.counts),
    waiting_on: { kind: (["person", "pr", "check", "approval"] as const).find((k) => k === w.kind) ?? "none", ref: str(w.ref), ask_id: str(w.ask_id), since: str(w.since) },
    running_jobs: num(o.running_jobs),
    running_tool: str(o.running_tool),
    heartbeat_at: str(o.heartbeat_at),
    prs: Array.isArray(o.prs) ? o.prs.filter(isObj).map((x) => ({ ref: str(x.ref), state: str(x.state), stub: x.stub === true })) : [],
    release_stage: (["none", "merged", "rolled", "verified"] as const).find((k) => k === o.release_stage) ?? "not_observed",
    stuck: Array.isArray(o.stuck) ? o.stuck.filter((x): x is string => typeof x === "string") : [],
    stub: o.stub === true,
    updated_at: str(o.updated_at),
    seq: num(o.seq),
  };
}

export function readTotals(raw: unknown): OverviewTotals | null {
  const o = snake(raw);
  if (!isObj(o)) return null;
  return { plans: num(o.plans), by_progress: counts(o.by_progress), waiting_on_me: num(o.waiting_on_me), running_jobs: num(o.running_jobs), open_prs: num(o.open_prs), stuck: num(o.stuck) };
}

/** Which arm a frame is: `{snapshot}`, `{delta}`, `{ended}`, `{heartbeat}`, optionally inside `{result}` (the gateway's envelope for a stream). */
function arm(frame: unknown): [string, Record<string, unknown>] | null {
  let f = snake(frame);
  if (isObj(f) && isObj(f.result) && !("snapshot" in f)) f = f.result;
  if (!isObj(f)) return null;
  for (const k of ["snapshot", "delta", "ended", "heartbeat"]) if (k in f) return [k, isObj(f[k]) ? (f[k] as Record<string, unknown>) : {}];
  return null;
}

/**
 * Folds one frame into the state. A snapshot replaces everything. A delta applies when its seq is
 * newer than the state's (an old or repeated one is dropped), as an upsert or a remove of one plan,
 * and carries the totals so the header cannot drift. `ended` with `resume_seq` 0 means the journal
 * was reset: the state empties and waits for the next snapshot. Anything else is ignored.
 */
export function foldFrame(s: Live, frame: unknown, now = Date.now()): Live {
  const a = arm(frame);
  if (!a) return s;
  const [kind, body] = a;
  if (kind === "heartbeat") return { ...s, lastAt: now };
  if (kind === "snapshot") {
    const plans = new Map<string, OverviewPlan>();
    for (const r of Array.isArray(body.plans) ? body.plans : []) {
      const p = readPlan(r);
      if (p) plans.set(p.plan_id, p);
    }
    return { plans, totals: readTotals(body.totals) ?? totalsOf([...plans.values()]), seq: num(body.seq), tier: str(body.tier), ready: true, generated_at: str(body.generated_at), lastAt: now };
  }
  if (kind === "ended") {
    return num(body.resume_seq) === 0 ? { ...emptyLive(), tier: s.tier, lastAt: now } : { ...s, lastAt: now };
  }
  const seq = num(body.seq);
  if (!s.ready || seq <= s.seq) return s;
  const plans = new Map(s.plans);
  const p = readPlan(body.plan);
  if (body.op === "remove") {
    if (p) plans.delete(p.plan_id);
  } else if (p) plans.set(p.plan_id, { ...p, seq });
  else return s;
  return { ...s, plans, seq, totals: readTotals(body.totals) ?? totalsOf([...plans.values()]), lastAt: now };
}

// The page's quick filters, its team pick and its quick actions (RFC 0188 section 3). Pure, so the
// tests cover them; overview-mode.tsx draws them. A filter narrows the rows the store answered (the
// search is the store's); the totals in the header stay over every plan.

export const FILTERS = [
  { id: "all", label: "All" },
  { id: "waiting", label: "Waiting on you" },
  { id: "running", label: "Running" },
  { id: "stuck", label: "Stuck" },
  { id: "prs", label: "Open PRs" },
  { id: "paused", label: "Paused" },
] as const;
export type OverviewFilter = (typeof FILTERS)[number]["id"];

const openPr = (x: PlanPull) => x.state === "open" || x.state === "draft";

/** Whether a row stays under the filter and the team pick ("" is every team). */
export function keepRow(p: OverviewPlan, f: OverviewFilter, team = ""): boolean {
  if (team && p.team !== team) return false;
  switch (f) {
    case "waiting":
      return p.waiting_on.kind === "person";
    case "running":
      return p.running_jobs > 0 || p.progress === "running";
    case "stuck":
      return p.stuck.length > 0 || p.progress === "failed";
    case "prs":
      return p.prs.some(openPr);
    case "paused":
      return p.state === "paused";
    default:
      return true;
  }
}

/** How many rows each filter would keep, for the chips. */
export const filterCounts = (plans: OverviewPlan[], team = ""): Record<OverviewFilter, number> =>
  Object.fromEntries(FILTERS.map((f) => [f.id, plans.filter((p) => keepRow(p, f.id, team)).length])) as Record<OverviewFilter, number>;

/** The teams the rows name, each once, sorted; nothing when no row has one (the files cannot know a team). */
export const teamsOf = (plans: OverviewPlan[]): string[] => [...new Set(plans.map((p) => p.team).filter(Boolean))].sort();

/** A pull request ref (`owner/repo#N`) as its GitHub page; "" when the ref is not one. */
export function prUrl(ref: string): string {
  const m = /^([A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+)#(\d{1,9})$/.exec(ref.trim());
  return m ? `https://github.com/${m[1]}/pull/${m[2]}` : "";
}

/** The plan line that archives a plan, pauses it or brings it back: a person's move, written to the
 * plan file the way the coordinator writes its own lines (ADR 0128: nothing is decided for the person). */
export const stateLine = (state: PlanState, at: string, by = "you"): string => line({ t: "plan", state, at, by });
