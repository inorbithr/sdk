import assert from "node:assert/strict";
import { test } from "node:test";

import { formatDay } from "./dates.ts";

test("formats a day", () => {
  assert.equal(formatDay(new Date(Date.UTC(2026, 10, 19, 14, 5))), "2026-11-19");
});
