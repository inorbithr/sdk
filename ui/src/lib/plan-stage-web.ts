// The web's stage of a plan item (RFC 0217, Planner). The Mac app gets every item's stage from the Rust plan machine
// over Tauri (`plan_state`); a browser has no such command, so the stage is derived here from what the coordinator
// answers to ResolvePlanItems (`POST /v1/coordinator/plan/status`) and from the item's own file status. Pure, so node
// tests it. Two stages cannot be observed from a browser and are never claimed: "on preview" (a branch preview is
// taken by the Mac app or a terminal) and "verified" (the roll's own check). An open pull request reads "PR open".

import { jobRefs } from "./job-controls.ts";
import type { StageInfo, StageMap } from "./plan-stage-core.ts";
import { type Item, type Plan, type PrRef, prOf } from "./planner.ts";
import type { PrLive } from "./planner-shared.ts";

/** One answer of ResolvePlanItems, in the page's words. */
export type RefStatus = {
  ref: string;
  status: string;
  reason: string;
  jobId: string;
  stage: string;
  prState: string;
  stub: boolean;
  /** Absent when the roll was not observed. */
  rolled?: boolean;
  affected: string[];
  notRolled: string[];
};

const isObj = (v: unknown): v is Record<string, unknown> => !!v && typeof v === "object" && !Array.isArray(v);
const str = (v: unknown) => (typeof v === "string" ? v : "");
const strs = (v: unknown) => (Array.isArray(v) ? v.filter((x): x is string => typeof x === "string") : []);
const pick = (o: Record<string, unknown>, snake: string) =>
  o[snake] ?? o[snake.replace(/_([a-z])/g, (_, c: string) => c.toUpperCase())];

/** The `items` of a ResolvePlanItems answer, either key spelling; anything that is not a row is dropped. */
export function readStatuses(raw: unknown): RefStatus[] {
  const list = isObj(raw) && Array.isArray(raw.items) ? raw.items : [];
  return list.filter(isObj).map((o) => {
    const rolled = pick(o, "rolled");
    return {
      ref: str(o.ref),
      status: str(o.status) || "unknown",
      reason: str(o.reason),
      jobId: str(pick(o, "job_id")),
      stage: str(o.stage),
      prState: str(pick(o, "pr_state")),
      stub: o.stub === true,
      ...(typeof rolled === "boolean" ? { rolled } : {}),
      affected: strs(o.affected),
      notRolled: strs(pick(o, "not_rolled")),
    };
  });
}

const REPO = "[A-Za-z0-9][A-Za-z0-9_.-]{0,99}/[A-Za-z0-9_.-]{1,100}";
const PR_IN_TEXT = new RegExp(`(?:https?://github\\.com/)?(${REPO})(?:#|/pull/)(\\d{1,9})(?!\\d)`, "g");
/** The most pull requests one item names. */
export const PRS_PER_ITEM = 12;

/** Every pull request an item is about: the plan API's `prs` first, then the ones its title and body name, each once. */
export function itemPrs(item: Pick<Item, "title" | "body">, listed: readonly string[] = []): PrRef[] {
  const out: PrRef[] = [];
  const add = (p: PrRef | null) => {
    if (p && out.length < PRS_PER_ITEM && !out.some((q) => q.repo === p.repo && q.number === p.number)) out.push(p);
  };
  for (const l of listed) add(prOf(l));
  for (const text of [item.title, item.body])
    for (const m of text.matchAll(PR_IN_TEXT)) add({ repo: m[1], number: Number(m[2]) });
  return out;
}

const key = (p: PrRef) => `${p.repo}#${p.number}`;

/** The pull request facts the answer carries, shaped as the Mac's PrLive. The web reads no checks or review, so those stay at zero and "" and the label says only open, merged or closed. */
export function prLives(statuses: readonly RefStatus[]): PrLive[] {
  const out: PrLive[] = [];
  for (const s of statuses) {
    const p = prOf(s.ref);
    if (!p || s.prState === "" || out.some((x) => x.repo === p.repo && x.number === p.number)) continue;
    out.push({
      repo: p.repo,
      number: p.number,
      title: "",
      url: `https://github.com/${p.repo}/pull/${p.number}`,
      state: s.prState.toUpperCase(),
      draft: false,
      review: "",
      passed: 0,
      failed: 0,
      pending: 0,
    });
  }
  return out;
}

/**
 * The stage of every item that is not dropped. The order is the machine's: a roll observed beats a merge, a merge beats
 * an open pull request, that beats a job that is building; with nothing resolved the item's file status decides. A
 * stub answer is labelled stub and never reads ready or done (the server already says so; this keeps the word).
 */
export function webStages(
  plan: Plan,
  statuses: readonly RefStatus[],
  listed: ReadonlyMap<string, readonly string[]> = new Map(),
): StageMap {
  const by = new Map(statuses.map((s) => [s.ref, s]));
  const kids = new Map<string, string[]>();
  for (const i of plan.items) if (i.parent) kids.set(i.parent, [...(kids.get(i.parent) ?? []), i.id]);
  const out = new Map<string, StageInfo>();
  for (const item of plan.items) {
    const prs = itemPrs(item, listed.get(item.id) ?? []);
    const mine = [...jobRefs(item), ...prs.map(key)].map((r) => by.get(r)).filter((s): s is RefStatus => !!s);
    const prRows = mine.filter((s) => s.prState !== "");
    const stub = mine.some((s) => s.stub);
    let stage: StageInfo["stage"];
    let label: string;
    if (item.status === "dropped") [stage, label] = ["dropped", "Dropped"];
    else if (mine.some((s) => s.rolled === true)) [stage, label] = ["rolled", "Rolled"];
    else if (prRows.some((s) => s.prState === "merged"))
      [stage, label] = ["merged", mine.some((s) => s.rolled === false) ? "Merged, not rolled" : "Merged"];
    else if (prRows.some((s) => s.prState === "open")) [stage, label] = ["on_preview", "PR open"];
    else if (mine.some((s) => s.status === "building")) [stage, label] = ["building", "Building"];
    else if (mine.some((s) => s.status === "ready") && !stub) [stage, label] = ["ready", "Ready"];
    else if (mine.some((s) => s.status === "done") && !stub) [stage, label] = ["merged", "Done"];
    else if (item.status === "done") [stage, label] = ["merged", "Marked done"];
    else if (item.status === "building") [stage, label] = ["building", "Building"];
    else if (item.status === "ready") [stage, label] = ["ready", "Ready"];
    else [stage, label] = ["proposed", "Proposed"];
    const closedOnly = prRows.length > 0 && prRows.every((s) => s.prState === "closed");
    out.set(item.id, {
      id: item.id,
      stage,
      label: stub ? `${label} (stub)` : label,
      file_status: item.status,
      since: item.since ?? "",
      wait: item.needs ? { who: "you", what: item.needs === "You" ? "" : item.needs } : null,
      wait_since: item.needsAt ?? "",
      evidence: mine.flatMap((s) => (s.jobId ? [s.jobId] : [])),
      children: kids.get(item.id) ?? [],
      prs,
      ...(closedOnly && stage !== "merged" && stage !== "rolled" ? { stuck: true } : {}),
    });
  }
  return out;
}
