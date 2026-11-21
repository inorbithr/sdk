import assert from "node:assert/strict";
import { test } from "node:test";

import { kind } from "./kind.ts";

test("names a text file", async () => {
  assert.match(await kind("package.json"), /text|JSON/);
});
