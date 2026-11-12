// The pure parts of the Decisions reader's review side (src/lib/decisions-core.ts): decision
// state, statuses, ai_approval, the diff in hunks, linked pull requests and checks.
// Run with `pnpm test` (node:test; Node strips the types).
import assert from "node:assert/strict";
import { test } from "node:test";
import {
  aiApprovalText,
  aiMayApprove,
  byMonth,
  canDecide,
  checksLabel,
  compareBase,
  type DiffLine,
  decisionState,
  diffCounts,
  hunks,
  latestPerReviewer,
  myReviewState,
  otherStatuses,
  prLabel,
  prRefs,
  type ReviewRow,
  splitFindings,
} from "../src/lib/decisions-core.ts";

const r = (reviewer: string, decision: string, version: number, at: string): ReviewRow => ({
  review_id: `${reviewer}${version}${decision}`,
  reviewer,
  decision,
  version,
  decided_at: at,
});

test("decision state: approvals against the rule, on the current version only", () => {
  const ok = decisionState(3, {
    approvals: 2,
    approvals_required: 2,
    reviews: [r("a", "approve", 3, "1"), r("b", "approve", 3, "2")],
  });
  assert.equal(ok.blocked, "");
  const short = decisionState(3, { approvals: 1, approvals_required: 2, reviews: [r("a", "approve", 3, "1")] });
  assert.match(short.blocked, /Needs 2 approvals on v3; it has 1/);
  assert.match(
    decisionState(3, { approvals: 0, approvals_required: 1, reviews: [] }).blocked,
    /Needs 1 approval on v3/,
  );
  assert.equal(decisionState(3, null).blocked, "", "no rule, nothing blocks");
});

test("decision state: a reviewer's latest answer counts, and an old version's does not", () => {
  const asked = decisionState(3, {
    approvals: 1,
    approvals_required: 1,
    reviews: [r("a", "changes", 3, "5"), r("b", "approve", 3, "1")],
  });
  assert.equal(asked.changes.length, 1);
  assert.match(asked.blocked, /Changes were asked on v3/);
  const changed = decisionState(3, {
    approvals: 1,
    approvals_required: 1,
    reviews: [r("a", "approve", 3, "9"), r("a", "changes", 3, "5")],
  });
  assert.equal(changed.changes.length, 0, "a later approval replaces the earlier ask");
  const stale = decisionState(3, { approvals: 1, approvals_required: 1, reviews: [r("a", "changes", 2, "5")] });
  assert.equal(stale.blocked, "", "changes asked on v2 don't block v3");
  const pending = decisionState(3, {
    approvals: 1,
    approvals_required: 1,
    reviews: [{ review_id: "x", reviewer: "a", decision: "pending", version: 3 }],
  });
  assert.equal(pending.changes.length, 0);
});

test("one row per reviewer, newest first; the author never answers their own", () => {
  const rows = latestPerReviewer([r("a", "approve", 3, "9"), r("b", "changes", 3, "8"), r("a", "pending", 2, "1")]);
  assert.deepEqual(
    rows.map((x) => x.reviewer),
    ["a", "b"],
  );
  assert.equal(rows[0]!.decision, "approve");

  const own = myReviewState("me", "me", 3, []);
  assert.equal(own.canAnswer, false);
  const asked = myReviewState("me", "other", 3, [{ review_id: "p", reviewer: "me", decision: "pending", version: 3 }]);
  assert.equal(asked.canAnswer, true);
  assert.ok(asked.mine);
  const done = myReviewState("me", "other", 3, [r("me", "approve", 3, "1")]);
  assert.equal(done.canAnswer, false);
  assert.ok(done.answered);
  const newVersion = myReviewState("me", "other", 4, [r("me", "approve", 3, "1")]);
  assert.equal(newVersion.canAnswer, true, "a new version asks again");
});

test("statuses: each kind's menu, and when Decide is offered", () => {
  assert.deepEqual(otherStatuses("rfc", "draft"), ["in_review", "decided", "superseded"]);
  assert.deepEqual(otherStatuses("adr", "proposed"), ["accepted", "superseded"]);
  assert.deepEqual(otherStatuses("study", "running"), ["measured", "published"]);
  assert.equal(canDecide("rfc", "in_review"), true);
  assert.equal(canDecide("rfc", "decided"), false);
  assert.equal(canDecide("adr", "accepted"), false);
  assert.equal(canDecide("prd", "superseded"), false);
});

test("ai_approval: manual unless auto names the kind", () => {
  assert.equal(aiMayApprove("manual", "rfc"), false);
  assert.equal(aiMayApprove("auto", "rfc"), true);
  assert.equal(aiMayApprove({ mode: "auto", kinds: ["adr", "study"] }, "ADR"), true);
  assert.equal(aiMayApprove({ mode: "auto", kinds: ["adr"] }, "rfc"), false);
  assert.equal(aiMayApprove({ mode: "manual", kinds: ["rfc"] }, "rfc"), false);
  assert.equal(aiMayApprove(undefined, "rfc"), false);
  assert.match(aiApprovalText("manual", "rfc"), /Only people approve/);
  assert.match(aiApprovalText("auto", "rfc"), /on a person's behalf/);
});

const L = (kind: string, text: string): DiffLine => ({ kind, text });

test("the diff shows changes with two lines of context and counts what it hides", () => {
  const lines = [
    ...Array.from({ length: 6 }, (_, i) => L("same", `a${i}`)),
    L("removed", "old"),
    L("added", "new"),
    ...Array.from({ length: 10 }, (_, i) => L("same", `b${i}`)),
    L("added", "tail"),
  ];
  const h = hunks(lines, 2);
  assert.equal(h.length, 2);
  assert.equal(h[0]!.skipped, 4, "a0..a3 are hidden");
  assert.deepEqual(
    h[0]!.lines.map((l) => l.text),
    ["a4", "a5", "old", "new", "b0", "b1"],
  );
  assert.equal(h[1]!.skipped, 6, "b2..b7 are hidden");
  assert.deepEqual(
    h[1]!.lines.map((l) => l.text),
    ["b8", "b9", "tail"],
  );
  assert.deepEqual(diffCounts(lines), { added: 2, removed: 1, unchanged: 16 });
});

test("the diff: nothing changed is no hunks; changes close together are one hunk", () => {
  assert.deepEqual(hunks([L("same", "x"), L("same", "y")]), []);
  const close = hunks([L("added", "1"), L("same", "2"), L("same", "3"), L("removed", "4")], 2);
  assert.equal(close.length, 1);
  assert.equal(close[0]!.skipped, 0);
  assert.equal(close[0]!.lines.length, 4);
  assert.deepEqual(hunks([]), []);
});

test("compare base: the version before, or the one the reader last saw", () => {
  assert.equal(compareBase(1), 0);
  assert.equal(compareBase(4), 3);
  assert.equal(compareBase(4, 2), 2);
  assert.equal(compareBase(4, 4), 3, "seen already: just the previous");
});

test("linked pull requests: found by URL, once each, only in the configured repos", () => {
  const text =
    "See https://github.com/inorbithr/core/pull/812 and (https://github.com/inorbithr/core/pull/812), then https://github.com/other/x/pull/3.\nAlso https://github.com/inorbithr/sdk/pull/44.";
  assert.deepEqual(
    prRefs(text).map((p) => `${p.repo}#${p.number}`),
    ["inorbithr/core#812", "other/x#3", "inorbithr/sdk#44"],
  );
  assert.deepEqual(
    prRefs(text, ["inorbithr/core", "inorbithr/sdk"]).map((p) => p.number),
    [812, 44],
  );
  assert.deepEqual(prRefs("no links, https://github.com/o/r/issues/5"), []);
});

test("a pull request's live state reads as one word", () => {
  const p = { state: "OPEN", draft: false, review: "", passed: 3, failed: 0, pending: 0 };
  assert.deepEqual(prLabel({ ...p, state: "MERGED" }), { text: "merged", tone: "good" });
  assert.deepEqual(prLabel({ ...p, state: "CLOSED" }), { text: "closed", tone: "idle" });
  assert.equal(prLabel({ ...p, draft: true }).text, "draft");
  assert.deepEqual(prLabel({ ...p, failed: 2 }), { text: "2 checks failing", tone: "bad" });
  assert.equal(prLabel({ ...p, review: "CHANGES_REQUESTED" }).tone, "bad");
  assert.equal(prLabel({ ...p, pending: 1 }).text, "checks running");
  assert.equal(prLabel({ ...p, review: "APPROVED" }).text, "approved");
  assert.equal(prLabel(p).text, "open");
});

test("checks: redaction blocks and comes first; form does not", () => {
  const f = [
    { line: 3, rule: "status-log", message: "The status log is missing a date." },
    { line: 9, rule: "domain", message: "[domain] internal.example.com is not public." },
  ];
  const { redaction, form } = splitFindings(f);
  assert.equal(redaction.length, 1);
  assert.equal(form.length, 1);
  assert.deepEqual(checksLabel(f), { text: "1 redaction · 1 form", tone: "bad" });
  assert.deepEqual(checksLabel([f[0]!]), { text: "1 form", tone: "warn" });
  assert.deepEqual(checksLabel([]), { text: "No findings", tone: "good" });
});

test("the timeline groups by month in the order given", () => {
  const e = (date: string) => ({ document_id: "d", date });
  const g = byMonth([e("2026-10-10"), e("2026-10-02"), e("2026-09-30")]);
  assert.deepEqual(
    g.map((m) => [m.month, m.lines.length]),
    [
      ["2026-10", 2],
      ["2026-09", 1],
    ],
  );
});

test("what is wanted: sentences, questions, actions and context", async () => {
  const { waitSentence, suggestedQuestions, suggestedActions, docContext } = await import(
    "../src/lib/decisions-core.ts"
  );
  const o = { who: "Ana", version: 2, when: "3 days ago", blocked: "", required: 1, approvals: 1 };
  assert.equal(waitSentence({ kind: "review" }, o), "Ana asked you to review v2 · 3 days ago");
  assert.equal(waitSentence({ kind: "review" }, { ...o, who: "" }), "You were asked to review v2 · 3 days ago");
  assert.equal(
    waitSentence({ kind: "decide" }, { ...o, blocked: "x", approvals: 0 }),
    "You decide this: it needs 1 approval on v2 and has 0.",
  );
  assert.match(waitSentence({ kind: "decide" }, o), /you can take the decision/);
  assert.equal(
    waitSentence({ kind: "question", text: "Why?", unblocks: "the roll" }, o),
    "Question from Ana: Why? · unblocks the roll · 3 days ago",
  );
  const q = suggestedQuestions(
    { kind: "prd", status: "in_review", current_version: 3 },
    { waiting: ["review"], blocked: "" },
  );
  assert.equal(q.length, 5);
  assert.equal(q[0], "Summarise what I'm being asked to decide");
  assert.ok(
    !suggestedQuestions({ kind: "prd", current_version: 1 }, { waiting: [], blocked: "" }).includes(
      "What changed since the last version?",
    ),
  );
  assert.deepEqual(
    suggestedActions({ inBand: new Set(["review"]), canAnswer: true, author: false, canDecide: true, blocked: "" }),
    ["decide", "ask-author"],
  );
  assert.deepEqual(
    suggestedActions({ inBand: new Set(), canAnswer: true, author: true, canDecide: false, blocked: "" }),
    [],
  );
  const c = docContext({ kind: "prd", title: "T", status: "in_review", current_version: 2 }, "PRD 0143", ["review v2"]);
  assert.equal(c.chip, "PRD 0143 · T");
  assert.match(c.wrap("Hi"), /PRD 0143 "T", status in review, version 2, waiting on me: review v2\.\n\nHi$/);
});
