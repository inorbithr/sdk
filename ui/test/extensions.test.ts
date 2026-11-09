import assert from "node:assert/strict";
import { test } from "node:test";
import { extensionsMenu } from "../src/frame/extensions-menu.ts";
import { isAction, isHeading, isOff, reasonOf } from "../src/frame/menu-core.ts";
import { loadExtensions, type Panel, parseExtensions } from "../src/lib/extensions.ts";

const board: Panel = { name: "board", version: "1.2.0", title: "Board", entry: "index.html" };

test("parseExtensions reads the server's shape, drops malformed rows and refuses any other body", () => {
  const body = { extensions: [board, { name: "x" }, null, { ...board, name: "todo", title: "Todo", extra: 1 }] };
  assert.deepEqual(parseExtensions(body), [board, { ...board, name: "todo", title: "Todo" }]);
  assert.deepEqual(parseExtensions({ extensions: [] }), []);
  for (const bad of [null, 1, "x", [], {}, { extensions: {} }]) assert.equal(parseExtensions(bad), null);
});

const answer = (status: number, body: string) => (async () => new Response(body, { status })) as typeof fetch;

test("loadExtensions asks this listener only and turns every failure into a reason, never a throw", async () => {
  let asked = "";
  const ok = (async (url: string | URL | Request) => {
    asked = String(url);
    return Response.json({ extensions: [board] });
  }) as typeof fetch;
  assert.deepEqual(await loadExtensions(ok), { state: "ok", panels: [board] });
  assert.equal(asked, "/api/extensions");

  assert.equal((await loadExtensions(answer(500, ""))).state, "error");
  assert.equal((await loadExtensions(answer(200, "not json"))).state, "error");
  assert.equal((await loadExtensions(answer(200, "{}"))).state, "error");
  const down = (async () => {
    throw new TypeError("network");
  }) as typeof fetch;
  assert.deepEqual(await loadExtensions(down), { state: "error", reason: "The local `iohr ui serve` did not answer." });
});

test("the Extensions menu lists each panel under one heading, or says how to install one", () => {
  const picked: string[] = [];
  const todo = { ...board, name: "todo", title: "Todo" };
  const menu = extensionsMenu([board, todo], (p) => picked.push(p.name));
  assert.ok(isHeading(menu[0] ?? { sep: true }));
  const rows = menu.filter(isAction);
  assert.deepEqual(
    rows.map((r) => [r.label, r.hint]),
    [
      ["Board", "board 1.2.0"],
      ["Todo", "todo 1.2.0"],
    ],
  );
  rows[1]?.run?.();
  assert.deepEqual(picked, ["todo"]);

  const none = extensionsMenu([], () => {}).filter(isAction);
  assert.equal(none.length, 1);
  assert.ok(none[0] && isOff(none[0]));
  assert.match(reasonOf(none[0] ?? { id: "", label: "" }), /iohr ext install/);
});
