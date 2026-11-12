// Settings.tsx is React, which node cannot load here, so this test reads its source: the table `BODY` names the
// panes that have a body, and the line that picks one falls back to `NotHere`. The pane list says which panes are
// built; the two must agree, or a built pane shows "not on this page yet" or a not-built one shows a control.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { PANES } from "../src/lib/settings-core.ts";

const source = readFileSync(new URL("../src/settings.tsx", import.meta.url), "utf8");
const panes = readFileSync(new URL("../src/settings-panes.tsx", import.meta.url), "utf8");

const table = source.match(/const BODY: .*> = \{\n([\s\S]*?)\n\};/);
const bodies = new Set([...(table?.[1] ?? "").matchAll(/^ {2}(\w+): /gm)].map((m) => m[1]));

test("the BODY table is found and names only real panes", () => {
  assert.ok(table, "settings.tsx has a BODY table");
  assert.ok(bodies.size > 0);
  const ids = new Set<string>(PANES.map((p) => p.id));
  for (const id of bodies) assert.ok(ids.has(id), `BODY names ${id}, which is no pane`);
});

test("every pane marked built has a BODY entry", () => {
  for (const p of PANES.filter((x) => x.built)) assert.ok(bodies.has(p.id), `${p.id} is built and has no BODY entry`);
});

test("every pane not built has no BODY entry, so it renders NotHere, which says where it works", () => {
  for (const p of PANES.filter((x) => !x.built)) {
    assert.ok(!bodies.has(p.id), `${p.id} is not built and has a BODY entry`);
    assert.ok(p.where.length > 0, `${p.id} names no place`);
  }
  assert.match(source, /BODY\[pane\.id\]\?\.\(props\) \?\? <NotHere pane=\{pane\} \/>/);
  assert.match(panes, /export function NotHere\(\{ pane \}: \{ pane: Pane \}\)[\s\S]*pane\.where/);
});
