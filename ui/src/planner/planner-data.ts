// The Planner's data on the web (RFC 0217): the plans overview over SSE (`GET /v1/coordinator/plan/events`), one plan read
// when the stream says it changed, the machine's stage for its pull requests and jobs (`POST /v1/coordinator/plan/status`),
// and the writes. Reads and writes go through lib/api.ts's `apiGet` and the live layer's `apiWrite`/`useLive`; nothing here
// calls `fetch` or `EventSource`.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { apiGet, type Read } from "../lib/api.ts";
import { itemJobs, planStatusRefs } from "../lib/job-controls.ts";
import { apiWrite, type LiveState, useLive } from "../lib/live-shim.ts";
import { type ApiItem, type ApiPlanResponse, foldServer } from "../lib/plan-api.ts";
import { prMap, type StageMap } from "../lib/plan-stage-core.ts";
import { prLives, type RefStatus, readStatuses, webStages } from "../lib/plan-stage-web.ts";
import { emptyPlan, type Plan } from "../lib/planner.ts";
import type { PrLive } from "../lib/planner-shared.ts";
import { emptyLive, foldFrame, type Live } from "../lib/plans-overview.ts";

/** The overview as the plan stream folds it, and the stream's state for the badge. */
export function useOverview(): { live: Live; state: LiveState; reason: string | null } {
  const { value, state, reason } = useLive<unknown, Live>("/v1/coordinator/plan/events", {
    reduce: (prev, frame) => foldFrame(prev, frame),
    initial: emptyLive(),
  });
  return { live: value, state, reason };
}

export type PlanData = {
  plan: Plan;
  /** Each item's pull requests as the server lists them, for the stage and the PR lines. */
  listed: ReadonlyMap<string, readonly string[]>;
  read: Read<ApiPlanResponse> | null;
  reload: () => void;
};

/** One plan, read again when the stream's `seq` for it moves (a delta for this plan), and after a write. */
export function usePlan(planId: string | null, seq: number): PlanData {
  const [read, setRead] = useState<Read<ApiPlanResponse> | null>(null);
  const [tick, setTick] = useState(0);
  // biome-ignore lint/correctness/useExhaustiveDependencies: `seq` and `tick` are the triggers
  useEffect(() => {
    if (planId === null) return;
    let live = true;
    void apiGet<ApiPlanResponse>(`/v1/coordinator/plans/${encodeURIComponent(planId)}`).then((r) => {
      if (live) setRead(r);
    });
    return () => {
      live = false;
    };
  }, [planId, seq, tick]);
  const data = read?.state === "ok" ? read.data : null;
  const plan = useMemo(
    () => (data ? foldServer(emptyPlan(data.plan?.title ?? planId ?? ""), data) : emptyPlan(planId ?? "")),
    [data, planId],
  );
  const listed = useMemo(
    () => new Map((data?.items ?? []).map((i: ApiItem) => [i.item_id, i.prs ?? []] as [string, readonly string[]])),
    [data],
  );
  const reload = useCallback(() => setTick((t) => t + 1), []);
  return { plan, listed, read, reload };
}

export type StageData = {
  stages: StageMap;
  statuses: RefStatus[];
  prs: Map<string, PrLive>;
  jobsOf: (itemId: string) => string[];
  reason: string;
};

const NONE: StageData = { stages: new Map(), statuses: [], prs: new Map(), jobsOf: () => [], reason: "" };

/** What the coordinator says about the plan's jobs and pull requests: one POST per change, never a poll. */
export function useStages({ plan, listed }: Pick<PlanData, "plan" | "listed">): StageData {
  const [out, setOut] = useState<StageData>(NONE);
  const asked = useRef("");
  useEffect(() => {
    // refs come from the stages of an empty answer (the machine's parse of each item's PRs), then the answer builds the real ones
    const first = webStages(plan, [], listed);
    const refs = planStatusRefs(plan.items, first);
    const key = refs.join(",");
    if (refs.length === 0) {
      asked.current = "";
      setOut({ ...NONE, stages: first });
      return;
    }
    if (key === asked.current) return;
    asked.current = key;
    let live = true;
    void apiWrite<unknown>("POST", "/v1/coordinator/plan/status", { refs }).then((r) => {
      if (!live) return;
      if (r.state !== "ok") {
        setOut({
          ...NONE,
          stages: first,
          reason: r.state === "signed-out" ? "Signed out: the stages are not read." : r.reason,
        });
        return;
      }
      const statuses = readStatuses(r.data);
      const stages = webStages(plan, statuses, listed);
      const resolved = statuses.map((s) => ({ ref: s.ref, jobId: s.jobId }));
      setOut({
        stages,
        statuses,
        prs: prMap(prLives(statuses)),
        jobsOf: (id) => {
          const item = plan.items.find((i) => i.id === id);
          return item ? itemJobs(item, resolved) : [];
        },
        reason: "",
      });
    });
    return () => {
      live = false;
    };
  }, [plan, listed]);
  return out;
}

/** A write's outcome in a person's words; `ok` runs the caller's reload. */
export type Wrote = { ok: true } | { ok: false; reason: string };

export async function send(method: "POST" | "PUT", path: string, body: unknown): Promise<Wrote> {
  const r = await apiWrite<unknown>(method, path, body);
  if (r.state === "ok") return { ok: true };
  if (r.state === "signed-out") return { ok: false, reason: "Signed out: sign in from Settings, then try again." };
  return {
    ok: false,
    reason:
      r.status === 409 || r.status === 412
        ? "Someone saved a newer version first. Reloaded; apply your change again."
        : r.reason,
  };
}
