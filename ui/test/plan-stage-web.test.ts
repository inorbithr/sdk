import assert from "node:assert/strict";
import { test } from "node:test";
import { itemPrs, prLives, readStatuses, webStages } from "../src/lib/plan-stage-web.ts";
import { applyLines, emptyPlan } from "../src/lib/planner.ts";

const plan = (...items: Record<string, unknown>[]) =>
  applyLines(
    emptyPlan("t"),
    items.map((i) => JSON.stringify({ t: "item", at: "2026-11-10T10:00:00Z", by: "a", ...i })).join("\n"),
  );

test("readStatuses reads either key spelling and keeps an unobserved roll absent", () => {
  const rows = readStatuses({
    items: [
      { ref: "a/b#1", status: "done", jobId: "job_1", prState: "merged", rolled: true, notRolled: [] },
      { ref: "a/b#2", status: "unknown", pr_state: "open", not_rolled: ["x"] },
      5,
    ],
  });
  assert.equal(rows.length, 2);
  assert.equal(rows[0].jobId, "job_1");
  assert.equal(rows[0].rolled, true);
  assert.equal(rows[1].prState, "open");
  assert.equal("rolled" in rows[1], false);
  assert.deepEqual(rows[1].notRolled, ["x"]);
});

test("itemPrs takes the listed ones, then title and body, each once", () => {
  const p = itemPrs(
    { title: "Fix inorbithr/sdk#4", body: "see https://github.com/inorbithr/core/pull/7 and inorbithr/sdk#4" },
    ["inorbithr/core#7"],
  );
  assert.deepEqual(
    p.map((x) => `${x.repo}#${x.number}`),
    ["inorbithr/core#7", "inorbithr/sdk#4"],
  );
});

test("a roll beats a merge beats an open PR; unread things never claim more", () => {
  const pl = plan(
    { id: "a", title: "x/y#1", status: "building" },
    { id: "b", title: "x/y#2", status: "building" },
    { id: "c", title: "x/y#3", status: "building" },
    { id: "d", title: "no refs", status: "ready" },
  );
  const st = readStatuses({
    items: [
      { ref: "x/y#1", status: "done", prState: "merged", rolled: true },
      { ref: "x/y#2", status: "done", prState: "merged", rolled: false },
      { ref: "x/y#3", status: "ready", prState: "open" },
    ],
  });
  const m = webStages(pl, st);
  assert.equal(m.get("a")?.stage, "rolled");
  assert.equal(m.get("b")?.label, "Merged, not rolled");
  assert.equal(m.get("c")?.label, "PR open");
  assert.equal(m.get("d")?.stage, "ready");
  // never "verified" and never "on preview" in words
  for (const i of m.values()) assert.notEqual(i.stage, "verified");
  assert.ok(![...m.values()].some((i) => /preview|verified/i.test(i.label)));
});

test("a stub answer is labelled stub and never reads ready or done", () => {
  const pl = plan({ id: "a", title: "job_01ABCDEFGHJKMNPQRSTVWXYZ01", status: "proposed" });
  const st = readStatuses({
    items: [
      { ref: "job_01ABCDEFGHJKMNPQRSTVWXYZ01", status: "done", stub: true, jobId: "job_01ABCDEFGHJKMNPQRSTVWXYZ01" },
    ],
  });
  const i = webStages(pl, st).get("a");
  assert.equal(i?.stage, "proposed");
  assert.match(i?.label ?? "", /stub/);
});

test("a PR closed without merging is stuck; needs becomes a wait on you; children are listed", () => {
  const pl = plan(
    { id: "g", kind: "goal", title: "g" },
    { id: "s", parent: "g", title: "x/y#9", status: "building", needs: "review it" },
  );
  const m = webStages(pl, readStatuses({ items: [{ ref: "x/y#9", status: "unknown", prState: "closed" }] }));
  assert.equal(m.get("s")?.stuck, true);
  assert.deepEqual(m.get("s")?.wait, { who: "you", what: "review it" });
  assert.deepEqual(m.get("g")?.children, ["s"]);
});

test("prLives shapes the PR rows with unread checks left at zero", () => {
  const l = prLives(
    readStatuses({
      items: [
        { ref: "x/y#3", prState: "open" },
        { ref: "job_1", status: "building" },
      ],
    }),
  );
  assert.equal(l.length, 1);
  assert.equal(l[0].state, "OPEN");
  assert.equal(l[0].url, "https://github.com/x/y/pull/3");
});
