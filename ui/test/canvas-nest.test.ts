// The canvas switcher's nests (canvas/nest.ts): what each canvas is decides where it sits, a `group`
// overrides it, a nest with one thing shows that thing inline, and the tree helpers agree.
import assert from "node:assert/strict";
import { test } from "node:test";
import { ancestorsOf, buildTree, groupPath, labelOf, searchRows, visibleRows } from "../src/canvas/nest.ts";

const e = (canvas_id: string, title: string, group?: string) => ({ canvas_id, title, ...(group ? { group } : {}) });

test("each kind of canvas lands in its nest", () => {
  assert.deepEqual(
    groupPath(e("cnv_coordinator-as-a-service", "Distributed and Programmable Coordinator as a Service")),
    ["Coordinator"],
  );
  assert.deepEqual(groupPath(e("cnv_coordinator-as-a-service_v3", "Coordinator as a Service (v3.0 layout, kept)")), [
    "Coordinator",
    "Older layouts, kept",
  ]);
  assert.deepEqual(groupPath(e("cnv_coordinator-understanding-v5", "Coordinator, as it really is (v5)")), [
    "Coordinator",
    "Understanding versions",
  ]);
  assert.deepEqual(groupPath(e("cnv_x1", "Coordinator, as it really is")), ["Coordinator", "Understanding versions"]);
  assert.deepEqual(groupPath(e("cnv_plan-abc", "Tonight's plan")), ["Plans"]);
  assert.deepEqual(groupPath(e("cnv_plnh-abc-12", "A long plan")), ["Plans"]);
  assert.deepEqual(groupPath(e("cnv_ask-apl_1", "Approve the thing")), ["Asks & approvals"]);
  assert.deepEqual(groupPath(e("cnv_a", "PRD 0130 The AI element")), ["Decisions", "PRDs"]);
  assert.deepEqual(groupPath(e("cnv_b", "RFC 0108 Departments")), ["Decisions", "RFCs"]);
  assert.deepEqual(groupPath(e("cnv_c", "Send our terms questions to Anthropic and OpenAI (study 0021)")), [
    "Decisions",
    "Studies",
  ]);
  assert.deepEqual(groupPath(e("cnv_dec-diag1", "A diagram")), ["Decisions"]);
  assert.deepEqual(groupPath(e("cnv_d", "TestFlight upload of latest main (b8394a6), build 1447")), ["Jobs"]);
  assert.deepEqual(groupPath(e("cnv_how-personalities-work", "How Personalities work")), ["Guides & demos"]);
  assert.deepEqual(groupPath(e("cnv_build-your-own-project-the-demo", "Build your own project: the demo")), [
    "Guides & demos",
  ]);
  assert.deepEqual(groupPath(e("cnv_crm", "Crm")), ["Other"]);
});

test("a group on the canvas wins, and nests to any depth", () => {
  assert.deepEqual(groupPath(e("cnv_plan-x", "A plan", "Coordinator/Older layouts")), ["Coordinator", "Older layouts"]);
  assert.deepEqual(groupPath(e("cnv_q", "Deep", " A / B / C/ ")), ["A", "B", "C"]);
});

test("a kept layout keeps which layout it is", () => {
  assert.equal(
    labelOf(e("cnv_coordinator-as-a-service_v3", "Coordinator as a Service (v3.0 layout, kept)")),
    "Coordinator as a Service, v3.0 layout",
  );
  assert.equal(
    labelOf(e("cnv_coordinator-as-a-service_v2", "Coordinator as a Service (first layout, kept)")),
    "Coordinator as a Service, first layout",
  );
  assert.equal(
    labelOf(e("cnv_coordinator-as-a-service_v7", "Coordinator as a Service (kept)")),
    "Coordinator as a Service, v7",
  );
  assert.equal(labelOf(e("cnv_p", "Plain")), "Plain");
});

const list = [
  e("cnv_a", "PRD 0130 The AI element"),
  e("cnv_coordinator-as-a-service_v3", "Coordinator as a Service (v3.0 layout, kept)"),
  e("cnv_ask-1", "Approve one"),
  e("cnv_coordinator-as-a-service", "Distributed and Programmable Coordinator as a Service"),
  e("cnv_coordinator-as-a-service_v2", "Coordinator as a Service (first layout, kept)"),
  e("cnv_coordinator-understanding-v5", "Coordinator, as it really is (v5)"),
  e("cnv_ask-2", "Approve two"),
  e("cnv_plan-a", "Tonight's plan"),
  e("cnv_plan-b", "Next week"),
  e("cnv_crm", "Crm"),
];

test("the tree: suggested order, canvases before nests, old layouts last, one-child nests inline", () => {
  const t = buildTree(list);
  // A nest of one shows that one where the nest would be: PRDs alone sits at Decisions' place, Other's Crm last.
  assert.deepEqual(
    t.map((n) => (n.kind === "group" ? n.label : `leaf:${n.label}`)),
    ["Coordinator", "leaf:PRD 0130 The AI element", "Plans", "Asks & approvals", "leaf:Crm"],
  );
  const co = t[0];
  assert.equal(co.kind, "group");
  if (co.kind !== "group") return;
  // The current Coordinator first; Understanding (one canvas) inline; the two old layouts in their nest, last.
  assert.deepEqual(
    co.children.map((c) => c.label),
    [
      "Distributed and Programmable Coordinator as a Service",
      "Coordinator, as it really is (v5)",
      "Older layouts, kept",
    ],
  );
  const old = co.children[2];
  assert.equal(old.kind === "group" && old.children.length, 2);
});

test("a nest with siblings stays a nest; a lone one is inline", () => {
  const t = buildTree([e("cnv_a", "PRD 1 x"), e("cnv_b", "RFC 2 y"), e("cnv_c", "PRD 3 z"), e("cnv_d", "RFC 4 w")]);
  assert.equal(t.length, 1);
  const d = t[0];
  assert.equal(d.kind === "group" && d.label, "Decisions");
  assert.deepEqual(d.kind === "group" && d.children.map((c) => c.label), ["PRDs", "RFCs"]);
  const lone = buildTree([e("cnv_a", "PRD 1 x"), e("cnv_b", "RFC 2 y"), e("cnv_c", "PRD 3 z")]);
  assert.deepEqual(lone[0].kind === "group" && lone[0].children.map((c) => c.label), ["PRDs", "RFC 2 y"]);
  assert.deepEqual(
    buildTree([e("cnv_a", "PRD 1 x")]).map((n) => n.kind),
    ["leaf"],
  );
});

test("ancestors, visible rows and search agree", () => {
  const t = buildTree(list);
  assert.deepEqual(ancestorsOf(t, "cnv_coordinator-as-a-service_v3"), [
    "Coordinator",
    "Coordinator/Older layouts, kept",
  ]);
  assert.deepEqual(ancestorsOf(t, "cnv_nope"), []);
  const closed = visibleRows(t, new Set());
  assert.ok(closed.every((r) => r.depth === 0));
  const opened = visibleRows(t, new Set(["Coordinator", "Coordinator/Older layouts, kept"]));
  assert.ok(opened.some((r) => r.depth === 2));
  const hits = searchRows(t, "older v3");
  assert.deepEqual(
    hits.map((h) => h.id),
    ["cnv_coordinator-as-a-service_v3"],
  );
  assert.deepEqual(hits[0].path, ["Coordinator", "Older layouts, kept"]);
  assert.deepEqual(searchRows(t, "  "), []);
});

test("the same canvas is listed once", () => {
  const t = buildTree([e("cnv_p", "Same"), e("cnv_p", "Same")]);
  assert.equal(t.length, 1);
});
