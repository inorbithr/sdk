// The Planner's per-item job controls (ADR 0075, Paused): which buttons a job's mode allows, what
// holds it, and the jobs an item names. The coordinator refuses any move its machine does not
// have; these tests pin that the page never offers one.
import assert from "node:assert/strict";
import { test } from "node:test";
import { parseJob } from "../src/lib/planner-shared.ts";
import { actionPath, controlsFor, holdCause, itemJobs, jobRefs, planStatusRefs } from "../src/lib/job-controls.ts";
import { parsePlan } from "../src/lib/planner.ts";

const job = (o: Record<string, unknown>) => {
  const j = parseJob({ job_id: "job_01M57D8QKST6K4KFDRMPMNZE89", stage: "running", mode: "active", ...o });
  assert.ok(j);
  return j;
};
const names = (o: Record<string, unknown>) => controlsFor(job(o)).map((c) => c.action);

test("each mode offers only the moves the machine has", () => {
  assert.deepEqual(names({}), ["pause", "cancel"]);
  assert.deepEqual(names({ mode: "held", hold_reason: "the check failed twice" }), ["release", "pause", "cancel"]);
  assert.deepEqual(names({ mode: "retry", retried: true, retry_at: "2026-10-18T12:00:00Z" }), ["retry", "pause", "cancel"]);
  assert.deepEqual(names({ mode: "awaiting", await_kind: "pr_merged" }), ["pause", "cancel"]);
  assert.deepEqual(names({ mode: "paused", paused_from: "held" }), ["resume", "cancel"]);
  // A stopped job is refiled as a new ask that names it; a done job has nothing left.
  assert.deepEqual(names({ stage: "stopped" }), ["refile"]);
  assert.deepEqual(names({ stage: "done" }), []);
  // A mode this build does not know offers nothing rather than a guess.
  assert.deepEqual(names({ mode: "frozen" }), []);
});

test("every move asks for a reason; cancel is the one in red", () => {
  for (const c of controlsFor(job({ mode: "held" }))) assert.ok(c.reasonRequired, c.action);
  const cancel = controlsFor(job({})).find((c) => c.action === "cancel");
  assert.equal(cancel?.tone, "danger");
  assert.equal(controlsFor(job({ mode: "retry" }))[0]?.label, "Retry now");
  assert.equal(controlsFor(job({ mode: "paused" }))[0]?.label, "Continue");
});

test("the hold cause says what the job waits on", () => {
  assert.equal(holdCause(job({ mode: "held", hold_reason: "the check failed twice" })), "Held: the check failed twice");
  assert.equal(holdCause(job({ mode: "held" })), "Held");
  assert.match(holdCause(job({ mode: "retry", retry_at: "2026-10-18T12:00:00Z" })), /^Retrying at 10-18 12:00Z$/);
  assert.equal(holdCause(job({ mode: "awaiting", await_kind: "pr_merged" })), "Waiting for pr_merged");
  assert.equal(holdCause(job({ mode: "paused", paused_from: "retry" })), "Paused (was retry)");
  assert.equal(holdCause(job({ stage: "stopped" })), "Stopped");
  assert.equal(holdCause(job({})), "");
});

test("the five moves go to the five routes, ids encoded", () => {
  assert.equal(actionPath("pause", "job_1"), "/v1/coordinator/jobs/job_1/pause");
  assert.equal(actionPath("retry", "a/b"), "/v1/coordinator/jobs/a%2Fb/retry");
  assert.equal(actionPath("refile", "job_1"), null);
});

test("an item's jobs come from job, job_id, jobs and its text", () => {
  const p = parsePlan(
    [
      { t: "item", id: "a", title: "x", job_id: "job_01M57DAHC9BTZFSNXCCAB1MC6M" },
      { t: "item", id: "b", title: "y", jobs: ["job_01M57DAGAZC79SZ2FGFJXETHBK", "nope"], body: "see job_01M57DAGTZFRJDTM4JQWH4QT7N and job_01M57DAGAZC79SZ2FGFJXETHBK" },
      { t: "item", id: "c", title: "z", body: "job_short is not one" },
    ]
      .map((o) => JSON.stringify(o))
      .join("\n") + "\n",
  );
  const by = new Map(p.items.map((i) => [i.id, jobRefs(i)]));
  assert.deepEqual(by.get("a"), ["job_01M57DAHC9BTZFSNXCCAB1MC6M"]);
  assert.deepEqual(by.get("b"), ["job_01M57DAGAZC79SZ2FGFJXETHBK", "job_01M57DAGTZFRJDTM4JQWH4QT7N"]);
  assert.deepEqual(by.get("c"), []);
});

test("the plan asks the coordinator once for every job and PR it names, at most 100", () => {
  // The pull requests come from the machine's parse (plan_state.rs item_prs), one stage per item.
  const items = Array.from({ length: 120 }, (_, n) => ({ id: `i${n}`, title: `inorbithr/core#${n + 1}`, body: "", status: "building" as const }));
  const stage = (prs: { repo: string; number: number }[]) => ({ stage: "building", label: "", since: "", wait_since: "", evidence: [], children: [], prs }) as const;
  const stages = new Map([
    ["j", stage([{ repo: "inorbithr/core", number: 1 }])],
    ...items.map((i, n) => [i.id, stage([{ repo: "inorbithr/core", number: n + 1 }])] as const),
    ["d", stage([{ repo: "inorbithr/core", number: 999 }])],
  ]);
  const refs = planStatusRefs([{ id: "j", title: "job_01M57DAHC9BTZFSNXCCAB1MC6M", body: "", status: "building" }, ...items, { id: "d", title: "", body: "", status: "dropped" }], stages as never);
  assert.equal(refs.length, 100);
  assert.deepEqual(refs.slice(0, 2), ["job_01M57DAHC9BTZFSNXCCAB1MC6M", "inorbithr/core#1"]);
  assert.ok(!refs.includes("inorbithr/core#999"), "a dropped item is not asked about");
});

test("an item's jobs are the ones it names and the ones its pull requests resolve to", () => {
  const item = { title: "job_01M57DAHC9BTZFSNXCCAB1MC6M", body: "inorbithr/core#7 and inorbithr/core#8", status: "building" as const };
  const statuses = [
    { ref: "inorbithr/core#7", jobId: "job_01M57DAGAZC79SZ2FGFJXETHBK" },
    { ref: "inorbithr/core#8", jobId: "" },
    { ref: "job_01M57DAHC9BTZFSNXCCAB1MC6M", jobId: "job_01M57DAHC9BTZFSNXCCAB1MC6M" },
    { ref: "inorbithr/core#9", jobId: "job_01M57DAGTZFRJDTM4JQWH4QT7N" },
  ];
  const prs = [{ repo: "inorbithr/core", number: 7 }, { repo: "inorbithr/core", number: 8 }];
  assert.deepEqual(itemJobs(item, statuses, prs), ["job_01M57DAHC9BTZFSNXCCAB1MC6M", "job_01M57DAGAZC79SZ2FGFJXETHBK"]);
  assert.deepEqual(itemJobs({ ...item, status: "dropped" }, statuses, prs), []);
});
