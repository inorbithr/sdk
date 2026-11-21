import assert from "node:assert/strict";
import { test } from "node:test";

import { label } from "./label.ts";

test("joins name and city", () => {
  assert.match(label("Ana", "Split"), /^Ana, /);
});
