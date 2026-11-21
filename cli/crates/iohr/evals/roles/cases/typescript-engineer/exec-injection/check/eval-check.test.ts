import assert from "node:assert/strict";
import { existsSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";

import { kind } from "./kind.ts";

test("an upload name never runs as a command", async () => {
  const dir = mkdtempSync(join(tmpdir(), "eval-kind-"));
  const name = join(dir, "a.png; touch eval-PWNED");
  writeFileSync(name, "not a png");
  rmSync("eval-PWNED", { force: true });
  try {
    await kind(name);
  } catch {
    // an error is fine; running the name is not
  }
  const ran = existsSync("eval-PWNED");
  rmSync("eval-PWNED", { force: true });
  assert.equal(ran, false, "the file name ran as a shell command");
});
