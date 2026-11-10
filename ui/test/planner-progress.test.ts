import assert from "node:assert/strict";
import { test } from "node:test";
import { parsePlan, applyLines } from "../src/lib/planner.ts";
import { decidablePrs, duration, progressOf, stageOf } from "../src/lib/planner-progress.ts";
import type { PrLive } from "../src/lib/planner-shared.ts";
import { prMap, type StageInfo } from "../src/lib/plan-stage-core.ts";
import { prLabel } from "../src/lib/planner-shared.ts";

const L = (...o: Record<string, unknown>[]) => o.map((x) => JSON.stringify(x)).join("\n") + "\n";
const T = (m: number) => new Date(Date.UTC(2026, 9, 14, 10, m)).toISOString();
const NOW = Date.parse(T(60));
const pr = (o: Partial<PrLive> = {}): PrLive => ({
  repo: "o/r",
  number: 7,
  title: "t",
  url: "",
  state: "OPEN",
  draft: false,
  review: "",
  passed: 0,
  failed: 0,
  pending: 0,
  ...o,
});

const base = L(
  { t: "plan", title: "P", at: T(0) },
  {
    t: "item",
    id: "g",
    kind: "goal",
    title: "Goal",
    status: "building",
    at: T(0),
  },
  {
    t: "item",
    id: "a",
    kind: "step",
    parent: "g",
    title: "A",
    status: "done",
    at: T(5),
  },
  {
    t: "item",
    id: "b",
    kind: "step",
    parent: "g",
    title: "B",
    status: "building",
    at: T(20),
  },
  {
    t: "item",
    id: "c",
    kind: "step",
    parent: "g",
    title: "C",
    status: "proposed",
    at: T(0),
  },
  { t: "item", id: "n", kind: "note", title: "scribble", at: T(1) },
);

test("goal and notes are not steps; counts, percent, current, elapsed", () => {
  const p = progressOf(parsePlan(base), new Map(), NOW);
  assert.deepEqual(
    p.steps.map((s) => [s.item.id, s.stage]),
    [
      ["a", "done"],
      ["b", "running"],
      ["c", "not-started"],
    ],
  );
  assert.equal(p.total, 3);
  assert.equal(p.done, 1);
  assert.equal(p.percent, 33);
  assert.equal(p.current?.item.id, "b");
  assert.equal(p.stepMs, 40 * 60_000); // B went to building at T(20), now is T(60)
  assert.equal(p.elapsedMs, 60 * 60_000); // first line T(0)
  assert.equal(p.complete, false);
});

test("a status line sets the time in the step; a body edit does not move it", () => {
  let pl = parsePlan(base);
  pl = applyLines(pl, L({ t: "item", id: "b", body: "more", at: T(30) }));
  assert.equal(pl.items.find((i) => i.id === "b")?.since, T(20));
  pl = applyLines(pl, L({ t: "item", id: "b", status: "ready", at: T(40) }));
  assert.equal(pl.items.find((i) => i.id === "b")?.since, T(40));
  assert.equal(pl.items.find((i) => i.id === "b")?.born, T(20));
});

test("needs puts a step in waiting-on-you from the needs line, and clears", () => {
  let pl = applyLines(parsePlan(base), L({ t: "item", id: "b", needs: "Approve the roll", at: T(45) }));
  let p = progressOf(pl, new Map(), NOW);
  assert.equal(p.steps[1].stage, "owner");
  assert.equal(p.steps[1].why, "Approve the roll");
  assert.equal(p.steps[1].since, T(45));
  // current stays the first running step; with B waiting, the first not done is B still
  assert.equal(p.current?.item.id, "b");
  assert.equal(p.stepMs, 15 * 60_000);
  assert.deepEqual(
    p.waiting.map((s) => s.item.id),
    ["b"],
  );
  pl = applyLines(pl, L({ t: "item", id: "b", needs: "", at: T(50) }));
  p = progressOf(pl, new Map(), NOW);
  assert.equal(p.steps[1].stage, "running");
});

const building = (wait: StageInfo["wait"] = null): StageInfo => ({ id: "b", stage: "building", label: "Building", file_status: "building", since: T(25), wait, wait_since: "", evidence: [], children: [], prs: [{ repo: "o/r", number: 7 }] });

test("pull request state: an open PR names its checks, closed fails, unread and merged change nothing", () => {
  const pl = applyLines(parsePlan(base), L({ t: "item", id: "b", status: "building", pr: "o/r#7", at: T(25) }));
  const step = (live?: PrLive, info = building()) => progressOf(pl, live ? new Map([["o/r#7", live]]) : new Map(), NOW, new Map([["b", info]])).steps[1];
  assert.equal(step().stage, "running");
  assert.equal(step().why, "Building");
  assert.equal(step(pr({ pending: 2 })).stage, "running");
  assert.match(step(pr({ pending: 2 })).why, /^#7: /);
  assert.equal(step(pr({ state: "CLOSED" })).stage, "failed");
  assert.equal(step(pr({ state: "MERGED" })).stage, "running"); // merged but the machine says building: its stage decides
  assert.equal(step(pr(), building({ who: "owner", what: "review" })).stage, "owner");
  // Before the machine answers, no PR is read and the file's status decides.
  assert.deepEqual(progressOf(pl, new Map([["o/r#7", pr({ state: "CLOSED" })]]), NOW).steps[1].prs, []);
  assert.equal(progressOf(pl, new Map([["o/r#7", pr({ state: "CLOSED" })]]), NOW).steps[1].stage, "running");
});

test("ready with no PR waits on the owner; a merged step stays done whatever its PR says", () => {
  const ready = applyLines(parsePlan(base), L({ t: "item", id: "b", status: "ready", at: T(30) }));
  assert.equal(progressOf(ready, new Map(), NOW).steps[1].stage, "owner");
  const merged = { ...building(), stage: "merged" as const };
  assert.equal(stageOf(parsePlan(base).items[1], [{ repo: "o/r", number: 7, live: pr({ state: "CLOSED", failed: 3 }) }], merged).stage, "done");
});

test("complete: 100, elapsed stops at the last done, no current", () => {
  const pl = applyLines(parsePlan(base), L({ t: "item", id: "b", status: "done", at: T(35) }, { t: "item", id: "c", status: "done", at: T(50) }));
  const p = progressOf(pl, new Map(), NOW);
  assert.equal(p.complete, true);
  assert.equal(p.percent, 100);
  assert.equal(p.current, null);
  assert.equal(p.finished, T(50));
  assert.equal(p.elapsedMs, 50 * 60_000);
});

test("dropped steps leave the total; an empty plan has no percent; undated files have no times", () => {
  const pl = applyLines(parsePlan(base), L({ t: "item", id: "c", status: "dropped", at: T(10) }));
  const p = progressOf(pl, new Map(), NOW);
  assert.equal(p.total, 2);
  assert.equal(p.dropped, 1);
  assert.equal(p.percent, 50);
  const empty = progressOf(parsePlan(""), new Map(), NOW);
  assert.equal(empty.percent, null);
  assert.equal(empty.elapsedMs, null);
  const undated = progressOf(parsePlan(L({ t: "item", id: "x", kind: "step", title: "X", status: "building" })), new Map(), NOW);
  assert.equal(undated.elapsedMs, null);
  assert.equal(undated.stepMs, null);
  assert.equal(undated.current?.item.id, "x");
});

test("nine of ten steps is 90, never 100", () => {
  const lines = [
    { t: "plan", title: "P", at: T(0) },
    ...Array.from({ length: 10 }, (_, i) => ({
      t: "item",
      id: `s${i}`,
      kind: "step",
      title: `S${i}`,
      status: i < 9 ? "done" : "building",
      at: T(1),
    })),
  ];
  assert.equal(progressOf(parsePlan(L(...lines)), new Map(), NOW).percent, 90);
  const lines2 = Array.from({ length: 200 }, (_, i) => ({
    t: "item",
    id: `s${i}`,
    kind: "step",
    title: `S${i}`,
    status: i < 199 ? "done" : "building",
    at: T(1),
  }));
  assert.equal(progressOf(parsePlan(L(...lines2)), new Map(), NOW).percent, 99);
});

test("duration", () => {
  assert.equal(duration(45_000), "45 s");
  assert.equal(duration(12 * 60_000), "12 min");
  assert.equal(duration(134 * 60_000), "2 h 14 min");
  assert.equal(duration(26 * 3600_000), "1 d 2 h");
});

test("only an open pull request offers Approve: merged, closed and unread ones do not", () => {
  const ref = (live?: PrLive) => ({ repo: "inorbithr/core", number: 1021, ...(live ? { live } : {}) });
  assert.equal(decidablePrs([ref(pr({ state: "OPEN" }))]).length, 1);
  assert.deepEqual(decidablePrs([ref(pr({ state: "MERGED" })), ref(pr({ state: "CLOSED" })), ref()]), []);
});

test("a pull request only Rust read shows merged on its row, the same as its stage", () => {
  const plan = parsePlan(L({ t: "plan", title: "dpcaas", at: T(0) }, { t: "item", id: "pr1021", kind: "task", status: "ready", pr: "inorbithr/core#1021", at: T(1) }));
  // plan_state names the item's pull requests (Rust's parser) and hands back what it read.
  const stage: StageInfo = { id: "pr1021", stage: "merged", label: "Merged, not rolled", file_status: "ready", since: "2026-10-16T20:10:45Z", wait: null, wait_since: "", evidence: [], children: [], prs: [{ repo: "inorbithr/core", number: 1021 }] };
  const merged = pr({ repo: "inorbithr/core", number: 1021, state: "MERGED", merged_at: "2026-10-16T20:10:45Z" });
  const stages = new Map([["pr1021", stage]]);
  const before = progressOf(plan, new Map(), NOW, stages).steps[0];
  assert.equal(before.prs[0].live, undefined, "without Rust's read the row has nothing to show");
  const step = progressOf(plan, prMap([merged]), NOW, stages).steps[0];
  assert.equal(step.stage, "done");
  assert.equal(step.prs[0].live?.state, "MERGED");
  assert.equal(prLabel(step.prs[0].live!).text, "merged");
  assert.deepEqual(decidablePrs(step.prs), []);
});
