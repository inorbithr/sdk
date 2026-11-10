import assert from "node:assert/strict";
import { test } from "node:test";
import { sseFrames } from "../src/planner/live-shim.ts";

test("sseFrames splits frames, joins data lines, skips comments and keeps the unfinished tail", () => {
  const r = sseFrames(": hi\n\nevent: delta\nid: 7\ndata: {\"a\":\ndata: 1}\n\ndata: {\"b\":2}\r\n\r\ndata: {\"c\"");
  assert.equal(r.frames.length, 2);
  assert.deepEqual(r.frames[0], { event: "delta", id: "7", data: "{\"a\":\n1}" });
  assert.equal(r.frames[1].event, "message");
  assert.equal(r.rest, "data: {\"c\"");
});
