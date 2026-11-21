import assert from "node:assert/strict";
import { test } from "node:test";

import { signup } from "./signup.ts";

test("a bad email is refused", async () => {
  const store = { save: async () => {} };
  assert.equal((await signup(store, { email: "nope" })).status, 400);
});
