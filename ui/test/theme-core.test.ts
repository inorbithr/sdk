import assert from "node:assert/strict";
import { test } from "node:test";
import { CHOICES, isDark, parseChoice } from "../src/lib/theme-core.ts";

test("a stored choice is one of three; anything else is System", () => {
  for (const c of CHOICES) assert.equal(parseChoice(c.id), c.id);
  for (const bad of [null, undefined, 3, "", "Dark", "auto", {}, ["dark"]]) assert.equal(parseChoice(bad), "system");
});

test("System follows the computer, Light and Dark do not", () => {
  assert.equal(isDark("system", true), true);
  assert.equal(isDark("system", false), false);
  assert.equal(isDark("light", true), false);
  assert.equal(isDark("dark", false), true);
});
