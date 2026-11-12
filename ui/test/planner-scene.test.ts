import assert from "node:assert/strict";
import test from "node:test";
import { emptyPlan, type Plan } from "../src/lib/planner.ts";
import { planScene } from "../src/lib/planner-scene.ts";

const item = (id: string, parent: string | null, kind = "step", status = "todo") =>
  ({ id, title: id, kind, status, parent, body: "", by: "" }) as unknown as Plan["items"][number];

test("a plan becomes a centred tree: children one step deeper, edges from the parent, notes left out", () => {
  const plan = { ...emptyPlan("p"), items: [item("a", null), item("b", "a"), item("c", "a"), item("n", null, "note")] };
  const s = planScene(plan, new Map());
  assert.deepEqual(
    s.nodes.map((n) => n.id),
    ["a", "b", "c"],
  );
  assert.deepEqual(s.edges, [
    { from: "a", to: "b" },
    { from: "a", to: "c" },
  ]);
  assert.ok(s.nodes[0].x < s.nodes[1].x);
  assert.equal(s.nodes[0].height, 2);
  assert.equal(
    s.nodes.reduce((t, n) => t + n.z, 0),
    0,
  );
});

test("a stage the machine reports sets the tone; without one an item is idle, never verified", () => {
  const plan = { ...emptyPlan("p"), items: [item("a", null), item("b", null)] };
  const stages = new Map([["a", { id: "a", stage: "merged", label: "Merged" }]]) as never;
  const s = planScene(plan, stages);
  assert.equal(s.nodes[0].tone, "good");
  assert.equal(s.nodes[1].tone, "idle");
  assert.equal(s.nodes[1].label, "todo");
});

test("an empty plan has no nodes and still a frame to look at", () => {
  const s = planScene(emptyPlan("p"), new Map());
  assert.equal(s.nodes.length, 0);
  assert.ok(s.extent >= 4);
});
