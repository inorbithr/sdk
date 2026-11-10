import assert from "node:assert/strict";
import test from "node:test";
import { flatten } from "../src/frame/context-menu-core.ts";
import { plannerHandlers } from "../src/lib/planner-keys.ts";
import { plannerMenu } from "../src/lib/planner-menus.ts";
import { plannerRoute } from "../src/lib/planner-route.ts";

const at = (p: string) => plannerRoute(p) as NonNullable<ReturnType<typeof plannerRoute>>;

test("the menu's modes are off on the overview and go to the plan's mode on a plan", () => {
  const ran: string[] = [];
  const act = { go: (h: string) => ran.push(h), copy: (t: string) => ran.push(`copy ${t}`) };
  const off = flatten(plannerMenu(at("/planner"), act)).map((x) => x.action);
  assert.ok(off.filter((a) => a.id.startsWith("planner:mode:")).every((a) => a.disabled));
  const rows = flatten(plannerMenu(at("/planner/p1/i1"), act)).map((x) => x.action);
  rows.find((a) => a.id === "planner:mode:write")?.run?.();
  rows.find((a) => a.id === "planner:waiting")?.run?.();
  assert.match(ran[0] ?? "", /^\/planner\/p1\/i1\?mode=write$/);
  assert.equal(ran[1], "/decisions/~waiting");
});

test("the key handlers cover the seven modes on a plan and only search on the overview", () => {
  const act = { go: () => {}, focusSearch: () => {} };
  assert.deepEqual(Object.keys(plannerHandlers(at("/planner"), act)), ["planner-search"]);
  const keys = Object.keys(plannerHandlers(at("/planner/p1"), act));
  assert.equal(keys.filter((k) => k.startsWith("planner-mode-")).length, 7);
});
