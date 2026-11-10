import assert from "node:assert/strict";
import { test } from "node:test";
import { sseData } from "../src/lib/live-shim.ts";

test("sseData joins the data lines of a block and skips comments and heartbeats", () => {
  assert.equal(sseData('event: message\ndata: {"a":1}'), '{"a":1}');
  assert.equal(sseData("data: a\ndata: b"), "a\nb");
  assert.equal(sseData(": keep-alive"), null);
});
