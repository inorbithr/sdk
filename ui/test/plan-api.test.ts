import assert from "node:assert/strict";
import { test } from "node:test";
import { parsePlan } from "../src/lib/planner.ts";
import type { ApiPlanResponse } from "../src/lib/plan-api.ts";
import { emptyStream, foldServer, foldStream, serverLines, serverPlanFor, snake } from "../src/lib/plan-api.ts";
import { evidenceOf } from "../src/lib/plan-evidence.ts";

const L = (...o: Record<string, unknown>[]) => o.map((x) => JSON.stringify(x)).join("\n") + "\n";
const T = (h: number) => new Date(Date.UTC(2026, 9, 21, h)).toISOString();

const cache = parsePlan(
  L(
    { t: "plan", title: "Mac evidence", at: T(1) },
    { t: "item", id: "rfc", kind: "rfc", title: "RFC 0171", status: "done", doc: { space_id: "lab_1", document_id: "ldoc_01ABCDEFGHJKMNPQRSTVWXYZ01", kind: "rfc", number: "0171" }, at: T(1) },
    { t: "item", id: "s", kind: "step", title: "Old title", status: "building", diagram: { nodes: [{ id: "a", label: "A" }], edges: [] }, at: T(2) },
  ),
);

const server = {
  plan: { id: "plan_01X", title: "Mac evidence gate", externalId: "mac-evidence", version: 4, updatedAt: T(5) },
  items: [
    { itemId: "s", kind: "step", title: "Mac Planner shows the gate", status: "ready", body: "", prs: ["inorbithr/inorbit-desktop#400"], position: 2, updatedAt: T(5), updatedBy: "coordinator", gate: { ok: false, reasons: ["no ADR", "diagram older than head"] } },
    { itemId: "rfc", kind: "rfc", title: "RFC 0171", status: "done", doc: "ldoc_01ABCDEFGHJKMNPQRSTVWXYZ01", position: 1, updatedAt: T(4) },
    { itemId: "adr", kind: "adr", title: "ADR 0132", status: "done", doc: "ldoc_01ABCDEFGHJKMNPQRSTVWXYZ02", diagram: "ldia_01ABCDEFGHJKMNPQRSTVWXYZ03", position: 3, updatedAt: T(4) },
  ],
  notes: [{ id: "n1", itemId: "s", author: "worker", body: "PR opened", createdAt: T(5) }],
  // The gateway may spell keys camelCase; the type says snake_case, so the test says it on purpose.
} as unknown as ApiPlanResponse;

test("the server's plan becomes the file's own event lines, in position order, camelCase read as snake_case", () => {
  const lines = serverLines(server).trim().split("\n").map((l) => JSON.parse(l));
  assert.deepEqual(
    lines.map((l) => [l.t, l.id ?? l.title ?? l.text]),
    [
      ["plan", "Mac evidence gate"],
      ["item", "rfc"],
      ["item", "s"],
      ["item", "adr"],
      ["activity", "PR opened"],
    ],
  );
  assert.deepEqual(lines[2].prs, ["inorbithr/inorbit-desktop#400"]);
  assert.deepEqual(lines[2].gate, { ok: false, reasons: ["no ADR", "diagram older than head"] });
  assert.equal(lines[3].diagram, "ldia_01ABCDEFGHJKMNPQRSTVWXYZ03");
  assert.deepEqual(snake({ planId: "p", items: [{ itemId: "i" }] }), { plan_id: "p", items: [{ item_id: "i" }] });
});

test("folded over the cache the server's facts win and the cache keeps what the API names by id alone", () => {
  const plan = foldServer(cache, server);
  assert.equal(plan.title, "Mac evidence gate");
  const s = plan.items.find((i) => i.id === "s")!;
  assert.equal(s.title, "Mac Planner shows the gate");
  assert.equal(s.status, "ready");
  assert.deepEqual(s.diagram, { nodes: [{ id: "a", label: "A" }], edges: [] }, "the canvas diagram the file holds stays");
  assert.equal(s.diagramAt, T(2), "the diagram's time is the line that drew it");
  const rfc = plan.items.find((i) => i.id === "rfc")!;
  assert.equal(rfc.doc?.space_id, "lab_1", "the same document keeps the space the file knew");
  const adr = plan.items.find((i) => i.id === "adr")!;
  assert.deepEqual(adr.doc, { space_id: "", document_id: "ldoc_01ABCDEFGHJKMNPQRSTVWXYZ02" }, "a document the file never saw has no space yet");
  assert.equal(adr.diagramRef, "ldia_01ABCDEFGHJKMNPQRSTVWXYZ03");
  assert.equal(adr.diagramAt, T(4));
  assert.equal(plan.activity.at(-1)?.text, "PR opened");
  // The gate's word travels with the item and the card shows it as the server said it.
  const e = evidenceOf(plan, s);
  assert.equal(e.fromServer, true);
  assert.equal(e.blocked, "blocked: no ADR +1");
  assert.equal(e.docs.adr?.ref?.document_id, "ldoc_01ABCDEFGHJKMNPQRSTVWXYZ02", "the Mac's own reading still fills the column");
});

test("folding the same server plan twice changes nothing, and a file line after it still lands", () => {
  const once = foldServer(cache, server);
  const twice = foldServer(once, server);
  assert.deepEqual(twice.items, once.items);
  assert.equal(twice.next, once.next, "no new order numbers for the same items");
});

test("the server plan for a local id is the one whose external_id is that id, else the id itself", () => {
  const plans = [
    { id: "plan_01A", external_id: "other" },
    { id: "plan_01B", external_id: "mac-evidence" },
    { id: "mac-evidence" },
  ];
  assert.equal(serverPlanFor("mac-evidence", plans)?.id, "plan_01B");
  assert.equal(serverPlanFor("plan_01A", plans)?.id, "plan_01A");
  assert.equal(serverPlanFor("nope", plans), null);
});

test("the plan stream fold keeps the newest seq per plan: snapshot, upsert, remove, ended", () => {
  let s = foldStream(emptyStream(), { snapshot: { seq: 10, plans: [{ planId: "plan_01B", seq: 9, updatedAt: T(5) }] } });
  assert.equal(s.ready, true);
  assert.equal(s.seq, 10);
  assert.equal(s.plans.get("plan_01B")?.seq, 9);
  s = foldStream(s, { delta: { seq: 11, op: "upsert", plan: { plan_id: "plan_01B", seq: 11, updated_at: T(6) } } });
  assert.equal(s.plans.get("plan_01B")?.updated_at, T(6));
  assert.equal(s.seq, 11);
  s = foldStream(s, { heartbeat: { at: T(7) } });
  assert.equal(s.seq, 11, "a heartbeat changes nothing");
  s = foldStream(s, { delta: { seq: 12, op: "remove", plan: { plan_id: "plan_01B" } } });
  assert.equal(s.plans.has("plan_01B"), false);
  s = foldStream(s, { ended: { reason: "reset", resume_seq: 0 } });
  assert.equal(s.ready, false, "a fresh snapshot is awaited");
  assert.equal(foldStream(s, "garbage").seq, 12);
});
