import assert from "node:assert/strict";
import { test } from "node:test";
import {
  docLabel,
  filterDocs,
  kindMix,
  type ListDoc,
  NO_FILTERS,
  parseRoute,
  rankEntries,
  sortDocs,
  statusMix,
  step,
} from "../src/lib/decisions-views.ts";

const now = Date.parse("2026-10-14T12:00:00Z");
const day = (n: number) => new Date(now - n * 86_400_000).toISOString();
const docs: ListDoc[] = [
  {
    document_id: "a",
    space_id: "s1",
    kind: "prd",
    number: 29,
    display_number: "0029",
    title: "A home for agents",
    status: "draft",
    created_by: "me",
    updated_at: day(1),
  },
  {
    document_id: "b",
    space_id: "s1",
    kind: "rfc",
    number: 113,
    display_number: "0113",
    title: "Approvals",
    status: "in_review",
    stage: "build",
    created_by: "emma",
    decider: "me",
    updated_at: day(10),
  },
  {
    document_id: "c",
    space_id: "s2",
    kind: "adr",
    number: 65,
    display_number: "0065",
    title: "Tauri",
    status: "accepted",
    created_by: "emma",
    updated_at: day(40),
  },
  { document_id: "d", space_id: "s2", kind: "study", number: 31, title: "Editor study", status: "draft" },
];

test("labels use the kind and the padded number", () => {
  assert.equal(docLabel(docs[0]!), "PRD 0029");
  assert.equal(docLabel(docs[3]!), "Study 0031");
});

test("filters combine, and mine means created or decides", () => {
  const f = (x: Partial<typeof NO_FILTERS>) =>
    filterDocs(docs, { ...NO_FILTERS, ...x }, now)
      .map((d) => d.document_id)
      .join("");
  assert.equal(f({}), "abcd");
  assert.equal(f({ kind: "rfc" }), "b");
  assert.equal(f({ status: "draft" }), "ad");
  assert.equal(f({ mine: "me" }), "ab");
  assert.equal(f({ owner: "emma" }), "bc");
  assert.equal(f({ stage: "build" }), "b");
  assert.equal(f({ space: "s2" }), "cd");
  assert.equal(f({ query: "0113" }), "b");
  assert.equal(f({ query: "tauri" }), "c");
});

test("the updated filter drops what has no date", () => {
  const f = (days: number) =>
    filterDocs(docs, { ...NO_FILTERS, days }, now)
      .map((d) => d.document_id)
      .join("");
  assert.equal(f(7), "a");
  assert.equal(f(30), "ab");
  assert.equal(f(90), "abc");
});

test("sorting by updated puts the undated last; by number groups by kind", () => {
  assert.equal(
    sortDocs(docs, "updated")
      .map((d) => d.document_id)
      .join(""),
    "abcd",
  );
  assert.equal(
    sortDocs(docs, "number")
      .map((d) => d.document_id)
      .join(""),
    "abcd",
  );
  assert.equal(sortDocs(docs, "title")[0]!.document_id, "a");
});

test("figures", () => {
  assert.deepEqual(statusMix(docs), [
    { status: "draft", count: 2 },
    { status: "accepted", count: 1 },
    { status: "in_review", count: 1 },
  ]);
  assert.deepEqual(
    kindMix(docs).map((k) => k.kind),
    ["prd", "rfc", "adr", "study"],
  );
});

test("the palette needs every word and ranks a start first", () => {
  const e = [
    { id: "1", label: "Waiting on me", group: "View" as const, go: "~waiting" },
    { id: "2", label: "RFC 0113 Approvals", hint: "Platform", group: "Document" as const, go: "x" },
    { id: "3", label: "Platform", group: "Space" as const, go: "p" },
  ];
  assert.deepEqual(
    rankEntries(e, "plat").map((x) => x.id),
    ["3", "2"],
  );
  assert.deepEqual(
    rankEntries(e, "appr 0113").map((x) => x.id),
    ["2"],
  );
  assert.deepEqual(rankEntries(e, "zzz"), []);
  assert.equal(rankEntries(e, "").length, 3);
});

test("arrow keys stay inside the rows", () => {
  assert.equal(step(0, -1, 5), 0);
  assert.equal(step(4, 1, 5), 4);
  assert.equal(step(2, 1, 5), 3);
  assert.equal(step(0, 1, 0), 0);
});

test("routes: the old review queue still opens the waiting view", () => {
  assert.deepEqual(parseRoute(""), { view: "overview" });
  assert.deepEqual(parseRoute("~review"), { view: "waiting" });
  assert.deepEqual(parseRoute("~waiting"), { view: "waiting" });
  assert.deepEqual(parseRoute("~new/lab_1"), { view: "new", space: "lab_1" });
  assert.deepEqual(parseRoute("lab_1/~timeline"), { view: "timeline", space: "lab_1" });
  assert.deepEqual(parseRoute("lab_1/~settings"), { view: "settings", space: "lab_1" });
  assert.deepEqual(parseRoute("lab_1/ldoc_2"), { view: "doc", space: "lab_1", doc: "ldoc_2" });
  assert.deepEqual(parseRoute("~diagrams/lab_1/ldia_3"), { view: "diagrams", space: "lab_1", id: "ldia_3" });
  assert.deepEqual(parseRoute("lab_1"), { view: "space", space: "lab_1" });
});

import { decisionsPath, decisionsRoute } from "../src/lib/decisions-route.ts";

test("decisionsRoute reads the path, null outside Decisions", () => {
  assert.equal(decisionsRoute("/settings/team"), null);
  assert.deepEqual(decisionsRoute("/decisions"), { view: "overview" });
  assert.deepEqual(decisionsRoute("/decisions/lab_1/ldoc_2"), { view: "doc", space: "lab_1", doc: "ldoc_2" });
  assert.deepEqual(decisionsRoute("/decisions/~all/kind:rfc"), { view: "all", kind: "rfc" });
  assert.equal(decisionsPath("lab_1", "ldoc_2"), "/decisions/lab_1/ldoc_2");
});
