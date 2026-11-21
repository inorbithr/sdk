import assert from "node:assert/strict";
import { test } from "node:test";

import { formatMinute } from "./dates.ts";

test("formats a minute", () => {
  assert.equal(formatMinute(new Date(Date.UTC(2026, 0, 2, 3, 4, 59))), "2026-01-02 03:04");
});
