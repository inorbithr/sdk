// What the Planner's pure libraries borrow from the rest of the Mac app (iohr-mac origin/main): the pull request
// facts (src/lib/api.ts `PrLive`, decisions-core.ts `PrState` and `prLabel`) and the coordinator job rows
// (hacker-mode.ts `JobLike`, `parseJob`, `parseTransitions`, `dur`). They are copied unchanged so the ported
// planner.ts, plan-api.ts, job-controls.ts and the rest read as they do on the Mac. If the decisions agent lands the
// same `prLabel` and `PrLive`, this file re-exports theirs and the copies go.

export type PrLive = { repo: string; number: number; title: string; url: string; state: string; draft: boolean; review: string; passed: number; failed: number; pending: number; created_at?: string; merged_at?: string; merge_commit?: string };
export type PrState = { state: string; draft: boolean; review: string; passed: number; failed: number; pending: number };

export function prLabel(p: PrState): { text: string; tone: "good" | "warn" | "bad" | "idle" } {
  if (p.state === "MERGED") return { text: "merged", tone: "good" };
  if (p.state === "CLOSED") return { text: "closed", tone: "idle" };
  if (p.draft) return { text: "draft", tone: "idle" };
  if (p.failed > 0) return { text: `${p.failed} check${p.failed === 1 ? "" : "s"} failing`, tone: "bad" };
  if (p.review === "CHANGES_REQUESTED") return { text: "changes requested", tone: "bad" };
  if (p.pending > 0) return { text: "checks running", tone: "warn" };
  if (p.review === "APPROVED") return { text: "approved", tone: "good" };
  return { text: "open", tone: "warn" };
}

const str = (v: unknown) => (typeof v === "string" ? v : "");
const iso = (v: unknown) => (typeof v === "string" && v && !Number.isNaN(Date.parse(v)) ? v : null);
/** The coordinator answers in snake_case; a camelCase answer reads the same. */
function field(o: Record<string, unknown>, key: string): unknown {
  if (key in o) return o[key];
  return o[key.replace(/_([a-z])/g, (_, c: string) => c.toUpperCase())];
}
const obj = (v: unknown) => (v && typeof v === "object" && !Array.isArray(v) ? (v as Record<string, unknown>) : null);

export type JobLike = {
  jobId: string;
  stage: string;
  state: string;
  mode: string;
  awaitKind: string;
  leaseId: string;
  createdAt: string | null;
  updatedAt: string | null;
  /** Why a held job is held, and the ask that holds it (empty otherwise). */
  holdReason?: string;
  holdAsk?: string;
  /** When a job in retry is due to run again. */
  retryAt?: string | null;
  /** The mode a paused job left, and returns to on resume (ADR 0075). */
  pausedFrom?: string;
  /** The job this one replaces. */
  supersedes?: string;
  /** The pull request the job opened (its compliance record), and that PR's state; empty until it opens one. */
  pullRequest?: string;
  prState?: string;
};
export type TransitionLike = { seq: number; fromStage: string; toStage: string; kind: string; actor: string; detail: string; createdAt: string | null };

export function parseJob(v: unknown): JobLike | null {
  const o = obj(v);
  const jobId = o ? str(field(o, "job_id")) : "";
  if (!o || !jobId) return null;
  return {
    jobId,
    stage: str(field(o, "stage")),
    state: str(field(o, "state")),
    mode: str(field(o, "mode")),
    awaitKind: str(field(o, "await_kind")),
    leaseId: str(field(o, "lease_id")),
    createdAt: iso(field(o, "created_at")),
    updatedAt: iso(field(o, "updated_at")),
    holdReason: str(field(o, "hold_reason")),
    holdAsk: str(field(o, "hold_ask")),
    retryAt: iso(field(o, "retry_at")),
    pausedFrom: str(field(o, "paused_from")),
    supersedes: str(field(o, "supersedes")),
    pullRequest: str(field(obj(field(o, "compliance")) ?? {}, "pull_request")),
    prState: str(field(obj(field(o, "compliance")) ?? {}, "pull_request_state")),
  };
}

export function parseTransitions(v: unknown): TransitionLike[] {
  const list: unknown[] = Array.isArray(v) ? v : [];
  return list
    .map(obj)
    .filter((o): o is Record<string, unknown> => !!o)
    .map((o) => ({ seq: Number(field(o, "seq")) || 0, fromStage: str(field(o, "from_stage")), toStage: str(field(o, "to_stage")), kind: str(field(o, "kind")), actor: str(field(o, "actor")), detail: str(field(o, "detail")), createdAt: iso(field(o, "created_at")) }))
    .sort((a, b) => a.seq - b.seq);
}
