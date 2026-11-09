import assert from "node:assert/strict";
import { test } from "node:test";
import { PANEL_FRAME } from "../src/frame/sandbox.ts";

test("a panel frame runs scripts in an opaque origin and gets nothing else", () => {
  assert.deepEqual(Object.keys(PANEL_FRAME).sort(), ["allow", "referrerPolicy", "sandbox"]);
  assert.equal(PANEL_FRAME.sandbox, "allow-scripts");
  assert.ok(!PANEL_FRAME.sandbox.includes("allow-same-origin"));
  assert.equal(PANEL_FRAME.referrerPolicy, "no-referrer");
  assert.equal(PANEL_FRAME.allow, "");
});
