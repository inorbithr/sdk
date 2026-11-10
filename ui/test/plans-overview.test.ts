import assert from "node:assert/strict";
import { test } from "node:test";
import { parsePlan } from "../src/lib/planner.ts";
import { progressOf } from "../src/lib/planner-progress.ts";
import { emptyLive, filterCounts, foldFrame, isSampleTitle, keepRow, matchesSearch, prUrl, progressWord, readPlan, searchText, sortPlans, stateLine, summaryFromFile, teamsOf, totalsOf, zeroCounts, type OverviewPlan } from "../src/lib/plans-overview.ts";
import { snake } from "../src/lib/plan-api.ts";
import type { PrLive } from "../src/lib/planner-shared.ts";
import type { StageInfo, StageMap } from "../src/lib/plan-stage-core.ts";

const L = (...o: Record<string, unknown>[]) => o.map((x) => JSON.stringify(x)).join("\n") + "\n";
const T = (m: number) => new Date(Date.UTC(2026, 9, 18, 10, m)).toISOString();
const NOW = Date.parse(T(60));
const pr = (o: Partial<PrLive> = {}): PrLive => ({ repo: "o/r", number: 7, title: "t", url: "", state: "OPEN", draft: false, review: "", passed: 0, failed: 0, pending: 0, ...o });

const plan = (title: string, ...items: Record<string, unknown>[]) =>
  parsePlan(L({ t: "plan", title, at: T(0) }, ...items.map((i) => ({ t: "item", kind: "step", by: "c", at: T(1), ...i }))));
// The pull requests of an item come from the plan machine (Rust `item_prs`, read as StageInfo.prs); a test stands in for it by
// reading "owner/repo#12" out of the item's title.
const machine = (p: ReturnType<typeof plan>): StageMap =>
  new Map(
    p.items.filter((i) => /#\d+/.test(i.title)).map((i) => [
      i.id,
      {
        id: i.id,
        stage: "building",
        label: "Building",
        file_status: i.status,
        since: T(1),
        wait: null,
        wait_since: "",
        evidence: [],
        children: [],
        prs: [...i.title.matchAll(/([\w.-]+\/[\w.-]+)#(\d+)/g)].map((m) => ({ repo: m[1], number: Number(m[2]) })),
      } as StageInfo,
    ]),
  );
const sum = (title: string, items: Record<string, unknown>[], prs = new Map<string, PrLive>(), o = {}) => {
  const p = plan(title, ...items);
  return summaryFromFile("pid", p, progressOf(p, prs, NOW, machine(p)), { now: NOW, ...o });
};

test("progress word follows the RFC order: failed, done, person, PR, running, not started", () => {
  const c = (o: Partial<ReturnType<typeof zeroCounts>>) => ({ ...zeroCounts(), ...o });
  assert.equal(progressWord(c({})), "not_started");
  assert.equal(progressWord(c({ failed: 1, done: 2 })), "failed");
  assert.equal(progressWord(c({ failed: 1, running: 1 })), "running");
  assert.equal(progressWord(c({ done: 3 })), "done");
  assert.equal(progressWord(c({ done: 1, waiting_on_person: 1, waiting_on_pr: 1, running: 1 })), "waiting_on_person");
  assert.equal(progressWord(c({ waiting_on_pr: 1, running: 1 })), "waiting_on_pr");
  assert.equal(progressWord(c({ running: 1, not_started: 2 })), "running");
});

test("a summary from a file counts steps, names the waiting item and links the ask", () => {
  const s = sum("Plan A", [
    { id: "a", title: "Done thing", status: "done" },
    { id: "b", title: "Pick the deal stages", status: "building", needs: "Which stages?" },
    { id: "c", title: "Later", status: "proposed" },
  ], new Map(), { askFor: (i: { id: string }) => (i.id === "b" ? "apr_X" : undefined) });
  assert.equal(s.progress, "waiting_on_person");
  assert.deepEqual([s.counts.done, s.counts.waiting_on_person, s.counts.not_started], [1, 1, 1]);
  assert.equal(s.waiting_on.kind, "person");
  assert.equal(s.waiting_on.ask_id, "apr_X");
  assert.equal(s.release_stage, "not_observed");
  assert.equal(s.team, "");
  assert.equal(s.stub, false);
});

test("a closed pull request is stuck, an open one is counted, a stub read is marked and never merged", () => {
  const prs = new Map([["o/r#7", pr({ state: "CLOSED" })], ["o/r#8", pr({ number: 8 })]]);
  const s = sum("Plan B", [{ id: "a", title: "Ship it o/r#7", status: "building" }, { id: "b", title: "Other o/r#8", status: "building" }], prs, { prsStub: true });
  assert.deepEqual(s.stuck, ["a"]);
  assert.deepEqual(s.prs.map((x) => [x.ref, x.state, x.stub]), [["o/r#7", "closed", true], ["o/r#8", "open", true]]);
  assert.equal(s.stub, true);
  assert.equal(totalsOf([s]).open_prs, 0, "a stub read never counts as an open pull request");
  const real = sum("Plan B", [{ id: "b", title: "Other o/r#8", status: "building" }], new Map([["o/r#8", pr({ number: 8 })]]));
  assert.equal(totalsOf([real]).open_prs, 1);
  assert.ok(s.prs.every((x) => x.state !== "merged"));
});

test("an unread pull request is named, not read, and not stub", () => {
  const s = sum("Plan C", [{ id: "a", title: "Ship o/r#9", status: "building" }]);
  assert.deepEqual(s.prs, [{ ref: "o/r#9", state: "not_read", stub: false }]);
});

test("a plan titled Sample is stub", () => {
  assert.equal(isSampleTitle("Sample: invoice export"), true);
  assert.equal(isSampleTitle("Samples of work"), false);
  assert.equal(sum("Sample: x", [{ id: "a", title: "x", status: "proposed" }]).stub, true);
});

test("sort puts waiting on you first, then stuck, running, PR, not started, done", () => {
  const mk = (id: string, progress: OverviewPlan["progress"], at: string, stuck: string[] = []): OverviewPlan => ({ ...(readPlan({ plan_id: id }) as OverviewPlan), progress, updated_at: at, stuck });
  const out = sortPlans([mk("done", "done", T(9)), mk("pr", "waiting_on_pr", T(9)), mk("new", "not_started", T(9)), mk("run", "running", T(9)), mk("stk", "waiting_on_pr", T(9), ["i"]), mk("you-old", "waiting_on_person", T(1)), mk("you-new", "waiting_on_person", T(5))]);
  assert.deepEqual(out.map((p) => p.plan_id), ["you-new", "you-old", "stk", "run", "pr", "new", "done"]);
});

const snap = (plans: Record<string, unknown>[], seq = 5) => ({ snapshot: { plans, seq, tier: "full", generated_at: T(0), totals: { plans: plans.length } } });

test("a snapshot replaces, a newer delta upserts or removes, an old one is dropped", () => {
  let s = foldFrame(emptyLive(), snap([{ plan_id: "a", title: "A" }, { plan_id: "b", title: "B" }]), 1);
  assert.equal(s.ready, true);
  assert.deepEqual([...s.plans.keys()], ["a", "b"]);
  s = foldFrame(s, { delta: { seq: 6, op: "upsert", plan: { plan_id: "a", title: "A2" }, totals: { plans: 2, waiting_on_me: 3 } } });
  assert.equal(s.plans.get("a")?.title, "A2");
  assert.equal(s.totals?.waiting_on_me, 3);
  assert.equal(s.seq, 6);
  const same = foldFrame(s, { delta: { seq: 6, op: "remove", plan: { plan_id: "a" } } });
  assert.equal(same, s);
  s = foldFrame(s, { delta: { seq: 7, op: "remove", plan: { plan_id: "b" } } });
  assert.deepEqual([...s.plans.keys()], ["a"]);
});

test("a delta before any snapshot is ignored; ended with resume_seq 0 empties and waits for a snapshot", () => {
  const d = { delta: { seq: 1, op: "upsert", plan: { plan_id: "a" } } };
  assert.equal(foldFrame(emptyLive(), d).plans.size, 0);
  let s = foldFrame(emptyLive(), snap([{ plan_id: "a" }]));
  s = foldFrame(s, { ended: { resume_seq: 0 } });
  assert.equal(s.ready, false);
  assert.equal(s.plans.size, 0);
  s = foldFrame(s, snap([{ plan_id: "z" }], 40));
  assert.deepEqual([...s.plans.keys()], ["z"]);
  assert.equal(s.seq, 40);
});

test("ended with a resume point keeps the state; a heartbeat only stamps the time", () => {
  let s = foldFrame(emptyLive(), snap([{ plan_id: "a" }]), 1);
  const kept = foldFrame(s, { ended: { resume_seq: 5 } }, 2);
  assert.equal(kept.plans.size, 1);
  s = foldFrame(s, { heartbeat: {} }, 99);
  assert.equal(s.lastAt, 99);
  assert.equal(s.plans.size, 1);
});

test("camelCase keys and the gateway's result envelope read the same", () => {
  const s = foldFrame(emptyLive(), { result: { snapshot: { plans: [{ planId: "a", waitingOn: { kind: "person", askId: "qak_1" }, runningJobs: "2", releaseStage: "rolled" }], seq: "9" } } });
  const p = s.plans.get("a");
  assert.equal(p?.waiting_on.ask_id, "qak_1");
  assert.equal(p?.running_jobs, 2);
  assert.equal(p?.release_stage, "rolled");
  assert.equal(s.seq, 9);
  assert.deepEqual(snake({ aB: [{ cD: 1 }] }), { a_b: [{ c_d: 1 }] });
});

test("an unknown release stage is not observed, an unknown progress is not started", () => {
  const p = readPlan({ plan_id: "a", release_stage: "great", progress: "excellent" });
  assert.equal(p?.release_stage, "not_observed");
  assert.equal(p?.progress, "not_started");
  assert.equal(readPlan({ title: "no id" }), null);
});

// RFC 0188: the plan's state comes from its plan line, the search text names everything a person looks a plan up by,
// the totals leave archived plans out, and the page's filters and quick actions are pure.

test("a plan line with a state sets it; the search text holds title, id, owner, ask, PR refs and the extra item words", () => {
  const p = plan("Plan S", { id: "a", title: "Ship o/r#12", status: "building" });
  assert.equal(p.state, undefined);
  const archived = parsePlan(L({ t: "plan", title: "Old", at: T(0) }, { t: "plan", state: "archived", at: T(2), by: "you" }));
  assert.equal(archived.state, "archived");
  assert.equal(parsePlan(L({ t: "plan", title: "Odd", at: T(0) }, { t: "plan", state: "lost", at: T(2) })).state, undefined);
  const s = summaryFromFile("pid-1", archived, progressOf(archived, new Map(), NOW), { now: NOW });
  assert.equal(s.state, "archived");
  const text = searchText({ ...sum("Plan S", [{ id: "a", title: "Ship o/r#12", status: "building" }]), owner: "Emma", waiting_on: { kind: "person", ref: "Pick", ask_id: "apr_X", since: "" } }, ["extra words"]);
  for (const w of ["plan s", "pid", "emma", "apr_x", "o/r#12", "extra words"]) assert.ok(text.includes(w), w);
  assert.ok(matchesSearch(text, "EMMA #12"));
  assert.ok(!matchesSearch(text, "emma nowhere"));
  assert.ok(matchesSearch(text, "   "));
});

test("totals skip archived plans; the filters and the team pick keep the rows they name", () => {
  const base = sum("Plan A", [{ id: "a", title: "Step", status: "proposed" }]);
  const rows: OverviewPlan[] = [
    { ...base, plan_id: "run", running_jobs: 2, team: "core" },
    { ...base, plan_id: "wait", waiting_on: { kind: "person", ref: "Pick", ask_id: "apr_1", since: "" }, team: "apps" },
    { ...base, plan_id: "stuck", stuck: ["x"], prs: [{ ref: "o/r#1", state: "open", stub: false }] },
    { ...base, plan_id: "paused", state: "paused" },
    { ...base, plan_id: "gone", state: "archived", running_jobs: 9 },
  ];
  const t = totalsOf(rows);
  assert.equal(t.plans, 4);
  assert.equal(t.running_jobs, 2);
  assert.deepEqual(filterCounts(rows), { all: 5, waiting: 1, running: 2, stuck: 1, prs: 1, paused: 1 });
  assert.deepEqual(rows.filter((p) => keepRow(p, "all", "core")).map((p) => p.plan_id), ["run"]);
  assert.deepEqual(rows.filter((p) => keepRow(p, "waiting")).map((p) => p.plan_id), ["wait"]);
  assert.deepEqual(teamsOf(rows), ["apps", "core"]);
});

test("a PR ref opens on GitHub, anything else does not; the state line is a plan line", () => {
  assert.equal(prUrl("inorbithr/inorbit-desktop#421"), "https://github.com/inorbithr/inorbit-desktop/pull/421");
  assert.equal(prUrl("#421"), "");
  assert.equal(prUrl("o/r#x"), "");
  assert.deepEqual(JSON.parse(stateLine("archived", T(3))), { t: "plan", state: "archived", at: T(3), by: "you" });
});
