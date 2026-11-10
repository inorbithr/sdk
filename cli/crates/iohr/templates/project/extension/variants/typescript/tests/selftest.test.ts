// The self-test: `node --test 'tests/**/*.test.ts'` runs it with the STATE and CACHE `iohr ext dev --test`
// gives (or the folders InOrbit Desktop's smoke test uses). One heartbeat, then what it wrote
// and where.

import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { test } from "node:test";

import { Refused, token } from "../src/channel.ts";

const HERE = path.join(import.meta.dirname, "..");

test("one heartbeat in state and nothing elsewhere", () => {
  const state = process.env.STATE;
  assert.ok(state && process.env.CACHE, "STATE and CACHE are set: run this with iohr ext dev --test");
  const before = fs.readdirSync(HERE).sort();
  const run = spawnSync(process.execPath, [path.join(HERE, "src", "main.ts")], {
    env: { ...process.env, IOHR_EXT_ONCE: "1" },
    cwd: HERE,
    encoding: "utf8",
  });
  assert.equal(run.status, 0, run.stderr);
  const beat = JSON.parse(fs.readFileSync(path.join(state, "heartbeat.json"), "utf8"));
  assert.equal(beat.id, "@@ID@@");
  assert.equal(beat.state, "ok");
  assert.equal(beat.beats, 1);
  // Granted when you are signed in and the channel is up, refused when not: either way it answered.
  assert.ok(["granted", "refused"].includes(beat.token));
  assert.deepEqual(fs.readdirSync(HERE).sort(), before, "the project wrote into its own folder");
});

test("without the channel the token ask is refused", async () => {
  delete process.env.IOHR_EXT_TOKEN_SOCKET;
  await assert.rejects(token(["tokens:write"]), Refused);
});
