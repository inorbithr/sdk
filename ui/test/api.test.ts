import assert from "node:assert/strict";
import { test } from "node:test";
import { apiGet, proxyPath, readSession, signIn, signOut } from "../src/lib/api.ts";

const token = "ab".repeat(32);
const seen: { url: string; init: RequestInit | undefined }[] = [];
const answering = (res: () => Response) =>
  (async (url: string | URL | Request, init?: RequestInit) => {
    seen.push({ url: String(url), init });
    return res();
  }) as typeof fetch;

test("the session read sends the token header and never a credential", async () => {
  const f = answering(() =>
    Response.json({
      signed_in: true,
      login: "idle",
      profile: "default",
      kind: "person",
      account: "acc",
      subject: "u1",
      scopes: ["iohr.api", 7],
    }),
  );
  const s = await readSession(token, f);
  assert.deepEqual(s, {
    state: "signed-in",
    profile: "default",
    kind: "person",
    account: "acc",
    subject: "u1",
    scopes: ["iohr.api"],
    login: "idle",
  });
  assert.equal(seen.at(-1)?.url, "/api/session");
  assert.deepEqual(seen.at(-1)?.init?.headers, { "x-iohr-token": token });
});

test("signed out carries the phase and the reason; a bad body or status is unreachable", async () => {
  assert.deepEqual(
    await readSession(
      token,
      answering(() => Response.json({ signed_in: false, login: "failed", reason: "no issuer" })),
    ),
    {
      state: "signed-out",
      login: "failed",
      reason: "no issuer",
    },
  );
  assert.deepEqual(
    await readSession(
      token,
      answering(() => Response.json({ signed_in: false, login: "weird" })),
    ),
    { state: "signed-out", login: "idle", reason: null },
  );
  for (const res of [
    () => new Response("x", { status: 401 }),
    () => new Response("x", { status: 500 }),
    () => new Response("nope"),
  ]) {
    assert.equal((await readSession(token, answering(res))).state, "unreachable");
  }
  assert.equal((await readSession(null)).state, "unreachable");
  const down = (async () => {
    throw new Error("down");
  }) as typeof fetch;
  assert.equal((await readSession(token, down)).state, "unreachable");
});

test("sign in and out are POSTs with the token header and no body", async () => {
  const ok = answering(() => Response.json({ login: "waiting" }));
  assert.equal(await signIn(token, ok), true);
  assert.equal(seen.at(-1)?.url, "/api/session/login");
  assert.equal(seen.at(-1)?.init?.method, "POST");
  assert.equal(seen.at(-1)?.init?.body, undefined);
  assert.equal(await signOut(token, ok), true);
  assert.equal(seen.at(-1)?.url, "/api/session/logout");
  assert.equal(await signIn(null, ok), false);
  assert.equal(
    await signOut(
      token,
      answering(() => new Response("x", { status: 403 })),
    ),
    false,
  );
});

test("a read is a GET under /v1/ through the proxy, query encoded", () => {
  assert.equal(
    proxyPath("/v1/orgs/o/monitors", { limit: "5", q: "a b&c" }),
    "/api/proxy/v1/orgs/o/monitors?limit=5&q=a%20b%26c",
  );
  assert.equal(proxyPath("/v1/accounts/me"), "/api/proxy/v1/accounts/me");
  for (const bad of ["/v2/x", "v1/x", "/v1/../x", "/v1//x", "/v1/x?y=1", "/v1/x#y", "/v1/a\\b"])
    assert.equal(proxyPath(bad), null, bad);
});

test("apiGet gives the API's data, signed-out on 401, and the API's reason on a failure", async () => {
  const ok = await apiGet<{ a: number }>(
    "/v1/x",
    {},
    token,
    answering(() => Response.json({ a: 1 })),
  );
  assert.deepEqual(ok, { state: "ok", data: { a: 1 } });
  assert.deepEqual(seen.at(-1)?.init?.headers, { "x-iohr-token": token });
  assert.deepEqual(
    await apiGet(
      "/v1/x",
      {},
      token,
      answering(() => new Response("x", { status: 401 })),
    ),
    { state: "signed-out" },
  );
  assert.deepEqual(
    await apiGet(
      "/v1/x",
      {},
      token,
      answering(() => Response.json({ error: { status: 404, message: "no such thing" } }, { status: 404 })),
    ),
    {
      state: "error",
      status: 404,
      reason: "no such thing",
    },
  );
  assert.deepEqual(
    await apiGet(
      "/v1/x",
      {},
      token,
      answering(() => new Response("boom", { status: 502 })),
    ),
    {
      state: "error",
      status: 502,
      reason: "The API answered HTTP 502.",
    },
  );
  assert.equal((await apiGet("/v2/x", {}, token)).state, "error");
  assert.equal((await apiGet("/v1/x", {}, null)).state, "error");
});
