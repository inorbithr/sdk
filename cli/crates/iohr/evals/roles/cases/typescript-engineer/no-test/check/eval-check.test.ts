import assert from "node:assert/strict";
import { test } from "node:test";

import { initials } from "./initials.ts";

test("initials survive odd spacing", () => {
  assert.equal(initials("Ana  Marić"), "AM");
  assert.equal(initials(" ivo\tbanić "), "IB");
  assert.equal(initials(""), "");
});
