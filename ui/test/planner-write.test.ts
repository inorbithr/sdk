import assert from "node:assert/strict";
import { test } from "node:test";
import { claimBody, editProblem, itemPath, newItemId, noteBody, reasonBody, upsertBody } from "../src/lib/planner-write.ts";

test("an edit is sent as the whole item with its version", () => {
  const had = { item_id: "a", kind: "step", title: "Old", status: "building", body: "text", doc: "d", diagram: "g", preview: "p", prs: ["o/r#1"], position: 3, version: 7 };
  const b = upsertBody(had, { title: " New ", status: "done" }, "me");
  assert.deepEqual(b, {
    item: { kind: "step", parent: "", title: "New", status: "done", body: "text", doc: "d", diagram: "g", preview: "p", prs: ["o/r#1"], position: 3 },
    expected_version: 7,
    holder: "me",
  });
});

test("a new item has no version and starts proposed", () => {
  const b = upsertBody(undefined, { title: "Add it" });
  assert.equal(b.expected_version, undefined);
  assert.equal(b.item.status, "proposed");
  assert.equal(b.item.kind, "step");
  assert.equal("holder" in b, false);
});

test("a problem is named before the send", () => {
  assert.match(editProblem({ title: "  " }) ?? "", /empty/);
  assert.match(editProblem({ status: "shipped" }) ?? "", /status/);
  assert.match(editProblem({ kind: "x" }) ?? "", /kind/);
  assert.equal(editProblem({ title: "ok", status: "done" }), null);
});

test("an item id is the title's words and never one that is taken", () => {
  assert.equal(newItemId("Add the Export!", new Set()), "add-the-export");
  assert.equal(newItemId("", new Set()), "item");
  const id = newItemId("Add", new Set(["add"]), 123456);
  assert.notEqual(id, "add");
  assert.match(id, /^add-/);
});

test("bodies and paths", () => {
  assert.deepEqual(noteBody("  hi "), { body: "hi" });
  assert.deepEqual(claimBody("me"), { holder: "me", seconds: 900 });
  assert.deepEqual(reasonBody(" why "), { reason: "why" });
  assert.equal(itemPath("p 1", "i/2", "/notes"), "/v1/coordinator/plans/p%201/items/i%2F2/notes");
});
