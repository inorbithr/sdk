import assert from "node:assert/strict";
import { test } from "node:test";
import { parseExtensions } from "../src/lib/extensions.ts";
import { panelRoute, panelSrc, routedPanel, startPanel, startToken, stopPanel } from "../src/lib/panels.ts";

const token = "ab".repeat(32);
const doc = (content: string | null) => ({
  querySelector: () => (content === null ? null : { getAttribute: () => content }),
});

test("the start token is the page's 64-hex meta, else none", () => {
  assert.equal(startToken(doc(token) as unknown as Document), token);
  for (const bad of [null, "", "zz".repeat(32), token.slice(1), `${token}0`]) {
    assert.equal(startToken(doc(bad) as unknown as Document), null);
  }
});

test("a start POSTs only the name with the token in a header, and reads the port", async () => {
  const sent: { url: string; init: RequestInit | undefined }[] = [];
  const fetcher = (async (url: string | URL | Request, init?: RequestInit) => {
    sent.push({ url: String(url), init });
    return Response.json({ port: 4100 });
  }) as typeof fetch;
  assert.deepEqual(await startPanel("todo", token, fetcher), { state: "ok", port: 4100 });
  assert.equal(sent[0]?.url, "/api/extensions/todo/start");
  assert.equal(sent[0]?.init?.method, "POST");
  assert.deepEqual(sent[0]?.init?.headers, { "x-iohr-token": token });
  assert.equal(sent[0]?.init?.body, undefined);
  assert.equal(await stopPanel("todo", token, fetcher), true);
  assert.equal(sent[1]?.url, "/api/extensions/todo/stop");
  // A name is one path segment, never a path.
  await startPanel("../x", token, fetcher);
  assert.equal(sent[2]?.url, "/api/extensions/..%2Fx/start");
});

test("a start never throws: no token, a refusal, a bad body or a bad port is a reason", async () => {
  const answer = (status: number, body: string) => (async () => new Response(body, { status })) as typeof fetch;
  assert.equal((await startPanel("todo", null, answer(200, "{}"))).state, "error");
  for (const [status, body] of [
    [502, ""],
    [403, ""],
    [200, "nope"],
    [200, '{"port":80}'],
    [200, '{"port":"4100"}'],
    [200, '{"port":70000}'],
  ] as const) {
    assert.equal((await startPanel("todo", token, answer(status, body))).state, "error", `${status} ${body}`);
  }
  const down = (async () => {
    throw new TypeError("network");
  }) as typeof fetch;
  assert.equal((await startPanel("todo", token, down)).state, "error");
  assert.equal(await stopPanel("todo", token, down), false);
});

test("a panel is framed only on plain HTTP at 127.0.0.1 and its own port", () => {
  assert.equal(panelSrc(4100, "/"), "http://127.0.0.1:4100/");
  assert.equal(panelSrc(4100, "/app/index.html?x=1"), "http://127.0.0.1:4100/app/index.html?x=1");
  for (const entry of ["//evil.example/", "http://evil.example/", "index.html", ""]) {
    assert.equal(panelSrc(4100, entry), null, entry);
  }
  for (const port of [0, 80, 70000, 4100.5]) assert.equal(panelSrc(port, "/"), null, String(port));
});

test("a panel's page route round-trips its name and nothing else", () => {
  assert.equal(panelRoute("todo"), "/ext/todo");
  assert.equal(routedPanel(panelRoute("todo")), "todo");
  assert.equal(routedPanel("/ext/todo/"), "todo");
  for (const path of ["/", "/ext", "/ext/", "/ext/a/b", "/ext/%E0%A4%A"]) assert.equal(routedPanel(path), null, path);
});

test("the listing keeps a running panel's port and drops one out of range", () => {
  const row = { name: "todo", version: "1.0.0", title: "Todo", entry: "/" };
  assert.deepEqual(parseExtensions({ extensions: [{ ...row, port: 4100 }] }), [{ ...row, port: 4100 }]);
  assert.deepEqual(parseExtensions({ extensions: [{ ...row, port: 80 }] }), [row]);
  assert.deepEqual(parseExtensions({ extensions: [{ ...row, port: "4100" }] }), [row]);
});
