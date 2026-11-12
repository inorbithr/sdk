import assert from "node:assert/strict";
import { readdirSync, readFileSync } from "node:fs";
import { test } from "node:test";
import layoutTable from "../src/frame/layout-shortcuts.json" with { type: "json" };
import frameTable from "../src/frame/shortcuts.json" with { type: "json" };
import {
  type Binding,
  clash,
  collisions,
  createHost,
  flatten,
  type Group,
  type Press,
  problems,
  type Reserved,
  type Row,
  SEQ_START,
  stepSeq,
  typingIn,
} from "../src/lib/shortcut-registry.ts";
import reserved from "../src/shortcuts/browser-reserved.json" with { type: "json" };
import terminal from "../src/shortcuts/terminal-keys.json" with { type: "json" };
import web from "../src/shortcuts/web-shortcuts.json" with { type: "json" };

const dir = new URL("../src/shortcuts/pages/", import.meta.url);
const pages: Group[] = readdirSync(dir)
  .filter((f) => f.endsWith(".keys.json"))
  .map((f) => ({ ...JSON.parse(readFileSync(new URL(f, dir), "utf8")), source: "page" as const }));
const groups: Group[] = [
  { scope: "global", title: "Frame", source: "frame", rows: frameTable as Row[] },
  { scope: "global", title: "Window layout", source: "layout", rows: layoutTable as Row[] },
  { scope: "global", title: "Go to", source: "web", rows: web as Row[] },
  ...pages,
];
const all = flatten(groups);
const R = reserved.keys as Reserved[];

const ev = (key: string, o: Partial<Press> = {}): Press => ({
  key,
  metaKey: false,
  ctrlKey: false,
  altKey: false,
  shiftKey: false,
  ...o,
});
const b = (id: string, scope: string, r: Partial<Row> & { source?: Binding["source"] }): Binding => ({
  id,
  label: id,
  scope,
  title: scope,
  source: "page",
  where: "",
  ...r,
});

test("every shipped binding is sound: no collision, no browser key, no malformed row", () => {
  assert.deepEqual(problems(all, R), []);
  assert.ok(pages.length >= 5);
});

test("two bindings on one key are found", () => {
  assert.equal(collisions([b("a", "global", { accel: "CmdOrCtrl+K" }), b("b", "x", { accel: "Cmd+K" })]).length, 1);
  assert.equal(
    collisions([b("a", "x", { accel: "N" }), b("b", "y", { accel: "N" })]).length,
    0,
    "two pages may share a key",
  );
  assert.equal(collisions([b("a", "x", { accel: "N" }), b("b", "x", { accel: "N" })]).length, 1);
  assert.equal(collisions([b("a", "global", { seq: ["g", "d"] }), b("b", "global", { seq: ["g", "d"] })]).length, 1);
  assert.ok(
    clash(b("a", "global", { seq: ["g", "d"] }), b("b", "x", { accel: "G" })),
    "a bare key under a sequence's first key",
  );
  assert.ok(clash(b("a", "global", { seq: ["g"] }), b("b", "global", { seq: ["g", "d"] })), "a prefix");
  const g = b("pal", "global", { accel: "CmdOrCtrl+K" });
  assert.equal(clash(g, b("p", "decisions", { accel: "CmdOrCtrl+K", overrides: "pal" })), false, "a named override");
});

test("the browser's own keys are refused, a text-editing key only where the surface edits", () => {
  assert.match(problems([b("t", "global", { accel: "CmdOrCtrl+T" })], R).join(), /new tab/);
  assert.match(problems([b("t", "global", { accel: "Ctrl+T" })], R).join(), /new tab/);
  assert.match(problems([b("t", "global", { accel: "CmdOrCtrl+1" })], R).join(), /tab/);
  assert.match(problems([b("z", "planner", { accel: "CmdOrCtrl+Z" })], R).join(), /undo/);
  assert.deepEqual(problems([b("z", "canvas", { accel: "CmdOrCtrl+Z" })], R), []);
});

test("malformed rows and bad overrides are reported", () => {
  const p = problems(
    [b("a", "x", {}), b("c", "x", { accel: "Q", seq: ["g", "x"] }), b("o", "x", { accel: "J", overrides: "nope" })],
    R,
  );
  assert.equal(p.length, 3, p.join("; "));
});

test("a sequence fires on its second key, times out, and starts over on a stray key", () => {
  const seqs = [{ id: "d", seq: ["g", "d"] }];
  const a = stepSeq(SEQ_START, ev("g"), seqs, 1000);
  assert.equal(a.holds, true);
  assert.equal(stepSeq(a.state, ev("d"), seqs, 1500).fire, "d");
  assert.equal(stepSeq(a.state, ev("d"), seqs, 3000).fire, undefined, "too slow");
  assert.equal(stepSeq(a.state, ev("x"), seqs, 1100).fire, undefined);
  assert.equal(stepSeq(a.state, ev("d", { metaKey: true }), seqs, 1100).fire, undefined);
});

test("the host: chords, sequences, page before global, text fields, no handler passes through", () => {
  const bs = [
    b("pal", "global", { accel: "CmdOrCtrl+K", source: "frame" }),
    b("dpal", "decisions", { accel: "CmdOrCtrl+K", overrides: "pal" }),
    b("go-d", "global", { seq: ["g", "d"] }),
    b("help", "global", { accel: "?" }),
    b("next", "decisions", { accel: "J" }),
  ];
  const host = createHost(bs);
  const hit: string[] = [];
  const off = host.mount("global", () => ({
    pal: () => hit.push("pal"),
    "go-d": () => hit.push("go-d"),
    help: () => hit.push("help"),
  }));
  let prevented = 0;
  const press = (e: Press, t = 0) => host.press({ ...e, preventDefault: () => void prevented++ }, t);
  assert.equal(press(ev("k", { metaKey: true })), true);
  assert.equal(press(ev("k", { ctrlKey: true })), true, "Ctrl stands in for Cmd");
  assert.equal(press(ev("?", { shiftKey: true })), true);
  assert.equal(press(ev("g"), 10), false);
  assert.equal(press(ev("d"), 20), true);
  assert.equal(press(ev("j")), false, "no handler, no scope mounted");
  assert.equal(press(ev("?"), 0), true);
  assert.deepEqual(hit, ["pal", "pal", "help", "go-d", "help"]);
  assert.equal(prevented, 5);
  const offPage = host.mount("decisions", () => ({ dpal: () => hit.push("dpal"), next: () => hit.push("next") }));
  press(ev("k", { metaKey: true }));
  assert.equal(hit.at(-1), "dpal", "the page stands in front of the global key");
  press(ev("j"));
  assert.equal(hit.at(-1), "next");
  const input = { tagName: "INPUT", type: "text" };
  assert.equal(press({ ...ev("j"), target: input }), false, "a bare key is typing in a field");
  assert.equal(press({ ...ev("?", { shiftKey: true }), target: input }), false);
  press({ ...ev("g"), target: input }, 100);
  assert.equal(press({ ...ev("d"), target: input }, 110), false, "no sequence inside a field");
  assert.equal(press({ ...ev("k", { metaKey: true }), target: input }), true, "a Cmd chord works in a field");
  offPage();
  press(ev("k", { metaKey: true }));
  assert.equal(hit.at(-1), "pal");
  off();
  assert.equal(press(ev("k", { metaKey: true })), false);
});

test("typingIn", () => {
  assert.equal(typingIn({ tagName: "TEXTAREA" }), true);
  assert.equal(typingIn({ tagName: "DIV", isContentEditable: true }), true);
  assert.equal(typingIn({ tagName: "INPUT", type: "checkbox" }), false);
  assert.equal(typingIn({ tagName: "BUTTON" }), false);
  assert.equal(typingIn(null), false);
});

test("every go row names a key the page can bind, and every Mac key shown is a real accelerator", () => {
  for (const r of web as Row[]) if (r.mac) assert.match(r.mac, /^CmdOrCtrl\+.$/);
});

test("the terminal panel's keys are still bound where the table says, and each web id exists", () => {
  const root = new URL("../../", import.meta.url);
  const ids = new Set(all.map((x) => x.id));
  const agree = new Set(["same", "differs", "terminal"]);
  for (const t of terminal.rows as { id: string; source: string; probe: string; web?: string; agree: string }[]) {
    assert.ok(
      readFileSync(new URL(t.source, root), "utf8").includes(t.probe),
      `${t.id}: ${t.probe} is not in ${t.source}`,
    );
    assert.ok(agree.has(t.agree), t.id);
    if (t.web) assert.ok(ids.has(t.web), `${t.id}: no web binding ${t.web}`);
    assert.equal(t.agree === "terminal", t.web === undefined && t.agree !== "same", t.id);
  }
});
