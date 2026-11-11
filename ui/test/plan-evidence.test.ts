import assert from "node:assert/strict";
import { test } from "node:test";
import { ageOf, blockedItems, blockedLine, documentsOf, evidenceOf, missingOf } from "../src/lib/plan-evidence.ts";
import type { StageInfo } from "../src/lib/plan-stage-core.ts";
import { parsePlan } from "../src/lib/planner.ts";
import type { PrLive } from "../src/lib/planner-shared.ts";

const L = (...o: Record<string, unknown>[]) => o.map((x) => JSON.stringify(x)).join("\n") + "\n";
const T = (h: number) => new Date(Date.UTC(2026, 9, 21, h)).toISOString();
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
const doc = (kind: string, n: string) => ({ space_id: "lab_1", document_id: `ldoc_${kind}${n}`, kind, number: n });

const full = parsePlan(
  L(
    { t: "plan", title: "P", at: T(1) },
    { t: "item", id: "prd", kind: "prd", title: "PRD 0111", status: "done", doc: doc("prd", "0111"), at: T(1) },
    { t: "item", id: "rfc", kind: "rfc", title: "RFC 0171", status: "done", doc: doc("rfc", "0171"), at: T(1) },
    { t: "item", id: "adr", kind: "adr", title: "ADR 0132", status: "done", doc: doc("adr", "0132"), at: T(1) },
    {
      t: "item",
      id: "s",
      kind: "step",
      title: "Mac",
      status: "ready",
      diagram: { nodes: [{ id: "a" }], edges: [] },
      at: T(9),
    },
  ),
);
// The pull requests of an item are the plan machine's parse (StageInfo.prs), so the tests hand them in as it would.
const stagesOf = (id: string, ...prs: string[]) =>
  new Map([
    [id, { id, prs: prs.map((r) => ({ repo: r.split("#")[0], number: Number(r.split("#")[1]) })) } as StageInfo],
  ]);
const step = (p = full) => p.items.find((i) => i.id === "s")!;

test("a step finds the plan's PRD, RFC and ADR and is not blocked when it carries a diagram newer than the head", () => {
  const prNow = new Map([["o/r#7", pr({ created_at: T(8) })]]);
  const e = evidenceOf(full, step(), prNow, new Map([["ldoc_adr0132", "accepted"]]), stagesOf("s", "o/r#7"));
  assert.deepEqual(
    Object.values(e.docs).map((d) => d && [d.kind, d.ref?.number, d.status]),
    [
      ["prd", "0111", ""],
      ["rfc", "0171", ""],
      ["adr", "0132", "accepted"],
    ],
  );
  assert.deepEqual(
    e.prs.map((p) => [p.ref, p.state]),
    [["o/r#7", "open"]],
  );
  assert.equal(e.diagram.present, true);
  assert.equal(e.diagram.updatedAt, T(9));
  assert.equal(e.diagram.headAt, T(8));
  assert.equal(e.diagram.stale, false);
  assert.deepEqual(e.missing, []);
  assert.equal(e.blocked, null);
  assert.equal(e.claimed, true);
});

test("the refusal names the missing fact in the gate's words: no ADR", () => {
  const plan = parsePlan(
    L(
      { t: "plan", title: "P", at: T(1) },
      { t: "item", id: "prd", kind: "prd", status: "done", doc: doc("prd", "0111"), at: T(1) },
      { t: "item", id: "rfc", kind: "rfc", status: "done", doc: doc("rfc", "0171"), at: T(1) },
      {
        t: "item",
        id: "s",
        kind: "step",
        title: "Mac",
        status: "done",
        diagram: { nodes: [{ id: "a" }], edges: [] },
        at: T(9),
      },
    ),
  );
  const e = evidenceOf(plan, plan.items.find((i) => i.id === "s")!);
  assert.deepEqual(e.missing, ["no ADR"]);
  assert.equal(e.blocked, "blocked: no ADR");
  assert.equal(e.fromServer, false);
});

test("a diagram saved before the newest pull request head reads 'diagram older than head'; a missing one 'no diagram'", () => {
  const old = parsePlan(
    L(
      { t: "plan", title: "P", at: T(1) },
      { t: "item", id: "s", kind: "step", status: "ready", diagram: { nodes: [{ id: "a" }], edges: [] }, at: T(2) },
      { t: "item", id: "s", status: "ready", at: T(10) },
    ),
  );
  const prNow = new Map([["o/r#7", pr({ state: "MERGED", merged_at: T(6), created_at: T(3) })]]);
  const e = evidenceOf(old, old.items[0], prNow, new Map(), stagesOf("s", "o/r#7"));
  assert.equal(e.diagram.updatedAt, T(2), "the line that set the diagram, not the later status line");
  assert.equal(e.diagram.headAt, T(6), "a merge is the head once merged");
  assert.equal(e.diagram.stale, true);
  assert.deepEqual(e.missing, ["no PRD", "no RFC", "no ADR", "diagram older than head"]);
  assert.equal(e.blocked, "blocked: no PRD +3");
  const none = parsePlan(L({ t: "item", id: "s", kind: "step", status: "ready", at: T(2) }));
  assert.deepEqual(missingOf(documentsOf(none, none.items[0]), [], { present: false, stale: false }), [
    "no PRD",
    "no RFC",
    "no ADR",
    "no diagram",
  ]);
});

test("only a ready or done claim is blocked; a building step shows what is missing without a refusal", () => {
  const plan = parsePlan(L({ t: "item", id: "s", kind: "step", status: "building", at: T(2) }));
  const e = evidenceOf(plan, plan.items[0]);
  assert.equal(e.claimed, false);
  assert.equal(e.missing.length, 4);
  assert.equal(e.blocked, null);
  assert.deepEqual(blockedItems(plan), []);
});

test("the server's gate wins over the Mac's reading, word for word, and ok clears it", () => {
  const refused = parsePlan(
    L(
      { t: "item", id: "s", kind: "step", status: "ready", at: T(2) },
      {
        t: "item",
        id: "s",
        gate: { ok: false, reasons: ["PR #12 lacks Implements: RFC", "diagram older than head"] },
        at: T(3),
      },
    ),
  );
  const e = evidenceOf(refused, refused.items[0]);
  assert.equal(e.fromServer, true);
  assert.deepEqual(e.missing, ["PR #12 lacks Implements: RFC", "diagram older than head"]);
  assert.equal(e.blocked, "blocked: PR #12 lacks Implements: RFC +1");
  const text = parsePlan(
    L({ t: "item", id: "s", kind: "step", status: "done", gate: "no ADR; diagram older than head", at: T(2) }),
  );
  assert.deepEqual(text.items[0].gate, { ok: false, reasons: ["no ADR", "diagram older than head"] });
  const ok = parsePlan(L({ t: "item", id: "s", kind: "step", status: "done", gate: { ok: true }, at: T(2) }));
  const eo = evidenceOf(ok, ok.items[0]);
  assert.equal(eo.fromServer, true);
  assert.equal(eo.blocked, null, "the server said ok: no PRD on this Mac does not block");
  const cleared = parsePlan(
    L(
      { t: "item", id: "s", kind: "step", status: "done", gate: { ok: true }, at: T(2) },
      { t: "item", id: "s", gate: null, at: T(3) },
    ),
  );
  assert.equal(cleared.items[0].gate, undefined);
  assert.equal(evidenceOf(cleared, cleared.items[0]).fromServer, false);
});

test("a step's own document, then its parent's, then the plan's; links count; dropped items do not", () => {
  const plan = parsePlan(
    L(
      { t: "item", id: "g", kind: "goal", status: "building", doc: doc("prd", "0001"), at: T(1) },
      { t: "item", id: "rfcA", kind: "rfc", status: "done", doc: doc("rfc", "0010"), at: T(1) },
      { t: "item", id: "rfcB", kind: "rfc", status: "done", doc: doc("rfc", "0020"), at: T(1) },
      { t: "item", id: "gone", kind: "adr", status: "dropped", doc: doc("adr", "0099"), at: T(1) },
      { t: "item", id: "s", kind: "step", parent: "g", status: "ready", doc: doc("rfc", "0030"), at: T(1) },
      { t: "link", from: "rfcB", to: "s", label: "implements" },
    ),
  );
  const d = documentsOf(plan, plan.items.find((i) => i.id === "s")!);
  assert.equal(d.rfc?.ref?.number, "0030", "its own first");
  assert.equal(d.prd?.ref?.number, "0001", "the parent's");
  assert.equal(d.adr, null, "a dropped ADR is no ADR");
  // A plan item of the kind that is not linked to Decisions is named, as "not linked"; a linked one beats it.
  const u = parsePlan(
    L(
      { t: "item", id: "adr", kind: "adr", title: "ADR: local data", status: "proposed", at: T(1) },
      { t: "item", id: "s", kind: "step", status: "ready", at: T(1) },
    ),
  );
  const du = documentsOf(u, u.items[1]);
  assert.deepEqual(du.adr && [du.adr.ref, du.adr.title], [null, "ADR: local data"]);
  assert.deepEqual(missingOf(du, [], { present: true, stale: false }), ["no PRD", "no RFC", "ADR not linked"]);
  const both = parsePlan(
    L(
      { t: "item", id: "adr", kind: "adr", status: "proposed", at: T(1) },
      { t: "item", id: "adr2", kind: "adr", status: "done", doc: doc("adr", "0132"), at: T(1) },
      { t: "item", id: "s", kind: "step", status: "ready", at: T(1) },
    ),
  );
  assert.equal(documentsOf(both, both.items[2]).adr?.ref?.number, "0132", "the linked one wins");
  const t = parsePlan(
    L(
      { t: "item", id: "rfcB", kind: "rfc", status: "done", doc: doc("rfc", "0020"), at: T(1) },
      { t: "item", id: "s", kind: "step", status: "ready", at: T(1) },
      { t: "link", from: "rfcB", to: "s", label: "implements" },
    ),
  );
  assert.equal(documentsOf(t, t.items[1]).rfc?.item, "rfcB", "a linked document counts");
});

test("blockedItems lists the refused steps in plan order and skips goals, notes and dropped items", () => {
  const plan = parsePlan(
    L(
      { t: "item", id: "g", kind: "goal", status: "done", at: T(1) },
      { t: "item", id: "a", kind: "step", status: "done", at: T(1) },
      { t: "item", id: "n", kind: "note", status: "done", at: T(1) },
      { t: "item", id: "b", kind: "step", status: "ready", at: T(1) },
      { t: "item", id: "x", kind: "step", status: "dropped", at: T(1) },
    ),
  );
  assert.deepEqual(
    blockedItems(plan).map((r) => [r.item.id, r.blocked]),
    [
      ["a", "blocked: no PRD +3"],
      ["b", "blocked: no PRD +3"],
    ],
  );
  assert.equal(blockedLine([]), null);
});

test("a closed pull request is a miss; the diagram's age reads in minutes, hours, days", () => {
  const plan = parsePlan(L({ t: "item", id: "s", kind: "step", status: "ready", at: T(1) }));
  const e = evidenceOf(
    plan,
    plan.items[0],
    new Map([["o/r#7", pr({ state: "CLOSED" })]]),
    new Map(),
    stagesOf("s", "o/r#7"),
  );
  assert.ok(e.missing.includes("PR #7 closed without merging"));
  assert.equal(e.prs[0].state, "closed");
  const now = Date.parse(T(12));
  assert.equal(ageOf(T(12), now), "just now");
  assert.equal(ageOf(T(11), now), "60 min old".replace("60 min", "1 h"));
  assert.equal(ageOf(new Date(now - 5 * 60_000).toISOString(), now), "5 min old");
  assert.equal(ageOf(new Date(now - 3 * 86_400_000).toISOString(), now), "3 d old");
  assert.equal(ageOf(undefined, now), "age unknown");
});
