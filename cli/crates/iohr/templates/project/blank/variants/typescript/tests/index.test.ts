import assert from "node:assert/strict";
import { test } from "node:test";

import { hello } from "../src/index.ts";

test("hello", () => {
  assert.equal(hello(), "hello from @@NAME@@");
});
