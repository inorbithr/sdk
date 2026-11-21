import assert from "node:assert/strict";
import { test } from "node:test";

import { label } from "./label.ts";

test("the city is in capitals", () => {
  assert.equal(label("Ana Marić", "Split"), "Ana Marić, SPLIT");
});
