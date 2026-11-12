import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

import { buildCommit, EXIT_FAILURE, EXIT_OK, EXIT_USAGE, run } from "../src/cli.ts";

const COMMIT = "0123456789ab";

function call(...args: string[]): { out: string; err: string; code: number } {
  let out = "";
  let err = "";
  const code = run(args, { out: (s) => (out += s), err: (s) => (err += s) }, COMMIT);
  return { out, err, code };
}

const golden = (file: string): string => readFileSync(new URL(`../golden/${file}`, import.meta.url), "utf8");

test("golden text and JSON", () => {
  for (const [file, args] of [
    ["help.txt", ["--help"]],
    ["greet.txt", ["greet", "Ada"]],
    ["version.txt", ["version"]],
    ["greet.json", ["--json", "greet", "Ada"]],
    ["version.json", ["version", "--json"]],
  ] as const) {
    const r = call(...args);
    assert.equal(r.code, EXIT_OK, args.join(" "));
    assert.equal(r.out, golden(file), file);
  }
});

test("exit codes", () => {
  assert.equal(call("greet").code, EXIT_OK);
  assert.equal(call().code, EXIT_USAGE);
  assert.equal(call("nope").code, EXIT_USAGE);
  assert.equal(call("greet", "a", "b").code, EXIT_USAGE);
  assert.equal(call("greet", "   ").code, EXIT_FAILURE);
  assert.equal(call("greet", "x".repeat(65)).code, EXIT_FAILURE);
  assert.equal(call("nope").out, "", "a usage error writes to stderr, not stdout");
});

test("the version line carries a commit", () => {
  assert.match(buildCommit(), /^([0-9a-f]{7,40}|unknown)$/);
});

test("a hostile argument is cut and escaped", () => {
  const r = call("\u001b[31mred" + "x".repeat(100));
  assert.ok(!r.err.includes("\u001b") && r.err.length < 200, r.err);
});
