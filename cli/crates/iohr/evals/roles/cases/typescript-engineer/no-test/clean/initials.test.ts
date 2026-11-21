import assert from "node:assert/strict";
import { test } from "node:test";

import { initials } from "./initials.ts";

test("two spaces between the names", () => {
  assert.equal(initials("ana  marić"), "AM");
});

test("an empty name has no initials", () => {
  assert.equal(initials("   "), "");
});
