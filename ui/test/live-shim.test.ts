import assert from "node:assert/strict";
import { test } from "node:test";
import { parseFrames, sseData } from "../src/lib/live-shim.ts";

test("sseData joins the data lines of a block and skips comments and heartbeats", () => {
  assert.equal(sseData('event: message\ndata: {"a":1}'), '{"a":1}');
  assert.equal(sseData("data: a\ndata: b"), "a\nb");
  assert.equal(sseData(": keep-alive"), null);
});

test("frames split on a blank line and the tail waits", () => {
  const a = parseFrames('id: 4\nevent: reading\ndata: {"a":1}\n\ndata: {"b"');
  assert.equal(a.frames.length, 1);
  assert.deepEqual(a.frames[0], { event: "reading", id: "4", data: '{"a":1}' });
  assert.equal(a.rest, 'data: {"b"');
  const b = parseFrames(`${a.rest}:2}\n\n`);
  assert.equal(b.frames[0]?.data, '{"b":2}');
});

test("comments and empty frames are skipped, CRLF is read", () => {
  const r = parseFrames(": hi\r\n\r\ndata: 1\r\ndata: 2\r\n\r\n");
  assert.equal(r.frames.length, 1);
  assert.equal(r.frames[0]?.data, "1\n2");
});
