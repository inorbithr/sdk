import assert from "node:assert/strict";
import { test } from "node:test";
import { MODES, modeOfDigit, plannerHref, plannerRoute } from "../src/lib/planner-route.ts";

test("the overview, a plan and an item", () => {
  assert.deepEqual(plannerRoute("/planner"), { planId: null, itemId: null, mode: "plan" });
  assert.deepEqual(plannerRoute("/planner/"), { planId: null, itemId: null, mode: "plan" });
  assert.deepEqual(plannerRoute("/planner/ask-1"), { planId: "ask-1", itemId: null, mode: "plan" });
  assert.deepEqual(plannerRoute("/planner/ask-1/ui-planner", "?mode=write"), {
    planId: "ask-1",
    itemId: "ui-planner",
    mode: "write",
  });
});

test("not the Planner's, or not an id", () => {
  assert.equal(plannerRoute("/planners"), null);
  assert.equal(plannerRoute("/settings/planner"), null);
  assert.equal(plannerRoute("/planner/a/b/c"), null);
  assert.equal(plannerRoute("/planner/%E0%A4%A"), null);
  assert.equal(plannerRoute("/planner/a%20b"), null);
});

test("an unknown mode falls back to Plan", () => {
  assert.equal(plannerRoute("/planner/p", "?mode=nope")?.mode, "plan");
});

test("href round trips every mode", () => {
  for (const mode of MODES) {
    const href = plannerHref({ planId: "p", itemId: "i", mode });
    const [path, q = ""] = href.split("?");
    assert.deepEqual(plannerRoute(path as string, q), { planId: "p", itemId: "i", mode });
  }
  assert.equal(plannerHref({}), "/planner");
  assert.equal(plannerHref({ planId: "p" }), "/planner/p");
});

test("digits pick the modes in order", () => {
  assert.equal(modeOfDigit("1"), "plan");
  assert.equal(modeOfDigit("7"), "explore");
  assert.equal(modeOfDigit("8"), null);
  assert.equal(modeOfDigit("a"), null);
});
