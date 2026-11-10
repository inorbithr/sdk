import assert from "node:assert/strict";
import { test } from "node:test";
import { apiSend, apiStream, parseSse } from "../src/lib/api.ts";

const token = "cd".repeat(32);
const enc = new TextEncoder();
const sse = (chunks: string[], hold = false): Response => {
  let i = 0;
  const body = new ReadableStream<Uint8Array>({
    async pull(c) {
      if (i < chunks.length) c.enqueue(enc.encode(chunks[i++]));
      else if (!hold) c.close();
      else await new Promise(() => {});
    },
  });
  return new Response(body, { headers: { "content-type": "text/event-stream" } });
};
const until = async (f: () => boolean) => {
  for (let i = 0; i < 200 && !f(); i++) await new Promise((r) => setTimeout(r, 2));
  assert.ok(f(), "condition not reached");
};

test("parseSse reads ids, events and split chunks; comments carry no event", () => {
  const a = parseSse(": hi\n\nid: 7\ndata: {\"a\":1}\n\nevent: error\ndata: x\n\nda");
  assert.deepEqual(a.frames, [
    { id: "7", event: "message", data: '{"a":1}' },
    { id: null, event: "error", data: "x" },
  ]);
  assert.equal(a.rest, "da");
});

test("a stream delivers events in order and reports live", async () => {
  const got: unknown[] = [];
  const states: string[] = [];
  const stop = apiStream("/v1/t/one", (d) => got.push(d), {
    token,
    fetcher: (async () => sse([": open\n\n", 'id: 1\ndata: {"n":1}\n\n', 'data: {"n"', ':2}\n\n'], true)) as typeof fetch,
    onState: (s) => states.push(s),
  });
  await until(() => got.length === 2);
  assert.deepEqual(got, [{ n: 1 }, { n: 2 }]);
  assert.ok(states.includes("live"));
  stop();
});

test("a break reconnects with backoff and resumes after the last id", async () => {
  const urls: string[] = [];
  const heads: (HeadersInit | undefined)[] = [];
  const waits: number[] = [];
  const got: unknown[] = [];
  const f = (async (u: string, init?: RequestInit) => {
    urls.push(String(u));
    heads.push(init?.headers);
    if (urls.length === 1) return sse(['id: c9\ndata: {"n":1}\n\n']); // ends: a break
    if (urls.length === 2) return new Response("x", { status: 503 });
    return sse(['id: c10\ndata: {"n":2}\n\n'], true);
  }) as typeof fetch;
  const states: string[] = [];
  const stop = apiStream("/v1/t/two", (d) => got.push(d), {
    token,
    fetcher: f,
    resumeParam: "after",
    query: { k: "v" },
    wait: async (ms) => void waits.push(ms),
    onState: (s) => states.push(s),
  });
  await until(() => got.length === 2);
  assert.equal(urls[0], "/api/proxy/v1/t/two?k=v");
  assert.equal(urls[1], "/api/proxy/v1/t/two?k=v&after=c9");
  assert.equal((heads[1] as Record<string, string>)["last-event-id"], "c9");
  assert.deepEqual(waits, [500, 1000]);
  assert.ok(states.includes("reconnecting"));
  stop();
});

test("a 4xx opening gives up as offline; 401 says signed out", async () => {
  const states: [string, string | undefined][] = [];
  apiStream("/v1/t/three", () => {}, {
    token,
    fetcher: (async () => new Response("{}", { status: 401 })) as typeof fetch,
    onState: (s, w) => states.push([s, w]),
  });
  await until(() => states.some(([s]) => s === "offline"));
  assert.equal(states.at(-1)?.[1], "signed out");
});

test("identical streams share one connection and the last stop closes it", async () => {
  let opened = 0;
  let aborted = false;
  const f = (async (_u: string, init?: RequestInit) => {
    opened++;
    init?.signal?.addEventListener("abort", () => {
      aborted = true;
    });
    return sse([], true);
  }) as typeof fetch;
  const a = apiStream("/v1/t/four", () => {}, { token, fetcher: f });
  const b = apiStream("/v1/t/four", () => {}, { token, fetcher: f });
  await until(() => opened === 1);
  a();
  assert.equal(aborted, false);
  b();
  await until(() => aborted);
  assert.equal(opened, 1);
});

test("silence past the watchdog reconnects", async () => {
  let opened = 0;
  const f = (async (_u: string, init?: RequestInit) => {
    opened++;
    const body = new ReadableStream<Uint8Array>({
      start(c) {
        init?.signal?.addEventListener("abort", () => c.error(new Error("aborted")));
      },
    });
    return new Response(body);
  }) as typeof fetch;
  const stop = apiStream("/v1/t/five", () => {}, { token, fetcher: f, silenceMs: 20, wait: async () => {} });
  await until(() => opened >= 2);
  stop();
});

test("apiSend sends JSON with the token and returns the API's error text", async () => {
  const seen: { url: string; init?: RequestInit }[] = [];
  const f = (async (url: string, init?: RequestInit) => {
    seen.push({ url, ...(init ? { init } : {}) });
    return seen.length === 1
      ? Response.json({ ok: 1 })
      : new Response(JSON.stringify({ error: { message: "nope" } }), { status: 409 });
  }) as typeof fetch;
  const a = await apiSend("POST", "/v1/x/y", { a: 1 }, { query: { q: "1" } }, token, f);
  assert.deepEqual(a, { state: "ok", data: { ok: 1 } });
  assert.equal(seen[0]?.url, "/api/proxy/v1/x/y?q=1");
  assert.equal(seen[0]?.init?.method, "POST");
  assert.equal(seen[0]?.init?.body, '{"a":1}');
  assert.deepEqual(seen[0]?.init?.headers, { "x-iohr-token": token, "content-type": "application/json" });
  assert.deepEqual(await apiSend("DELETE", "/v1/x/y", undefined, {}, token, f), {
    state: "error",
    status: 409,
    reason: "nope",
  });
  assert.equal((await apiSend("POST", "/x", {}, {}, token, f)).state, "error");
});
