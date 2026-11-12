import assert from "node:assert/strict";
import { afterEach, test } from "node:test";
import { apiStream, resetLive, type SocketConfig, type SocketLike } from "../src/lib/api.ts";

const token = "ef".repeat(32);
const enc = new TextEncoder();

class FakeSocket implements SocketLike {
  static made: FakeSocket[] = [];
  static refuse = false;
  sent: Record<string, unknown>[] = [];
  onopen: (() => void) | null = null;
  onmessage: ((e: { data: unknown }) => void) | null = null;
  onclose: (() => void) | null = null;
  onerror: (() => void) | null = null;
  closed = false;
  url: string;
  constructor(url: string) {
    this.url = url;
    FakeSocket.made.push(this);
    queueMicrotask(() => {
      if (FakeSocket.refuse) this.onerror?.();
      else this.onopen?.();
    });
  }
  send(data: string) {
    const f = JSON.parse(data) as Record<string, unknown>;
    this.sent.push(f);
    if (f.type === "auth") queueMicrotask(() => this.say({ type: "ready" }));
  }
  close() {
    if (this.closed) return;
    this.closed = true;
    queueMicrotask(() => this.onclose?.());
  }
  say(f: Record<string, unknown>) {
    this.onmessage?.({ data: JSON.stringify(f) });
  }
  calls() {
    return this.sent.filter((f) => f.type === "call");
  }
}
const cfg: SocketConfig = { url: "ws://127.0.0.1:1/api/live", ctor: FakeSocket };
const until = async (f: () => boolean) => {
  for (let i = 0; i < 300 && !f(); i++) await new Promise((r) => setTimeout(r, 2));
  assert.ok(f(), "condition not reached");
};
const sse = (text: string): Response =>
  new Response(
    new ReadableStream<Uint8Array>({
      start(c) {
        c.enqueue(enc.encode(text));
      },
      async pull() {
        await new Promise(() => {});
      },
    }),
  );

afterEach(() => {
  resetLive();
  FakeSocket.made = [];
  FakeSocket.refuse = false;
});

test("the token is the first frame, then calls carry the API path and events arrive in order", async () => {
  const got: unknown[] = [];
  const states: string[] = [];
  const stop = apiStream("/v1/coordinator/picture/events", (d) => got.push(d), {
    token,
    query: { tier: "live" },
    socket: cfg,
    onState: (s) => states.push(s),
  });
  await until(() => FakeSocket.made[0]?.calls().length === 1);
  const ws = FakeSocket.made[0] as FakeSocket;
  assert.deepEqual(ws.sent[0], { type: "auth", token });
  assert.deepEqual(ws.calls()[0], { type: "call", id: "1", path: "/v1/coordinator/picture/events?tier=live" });
  ws.say({ type: "data", id: "1", data: { n: 1 } });
  ws.say({ type: "data", id: "1", data: { n: 2 } });
  assert.deepEqual(got, [{ n: 1 }, { n: 2 }]);
  assert.ok(states.includes("live"));
  stop();
  await until(() => ws.sent.some((f) => f.type === "cancel"));
  assert.deepEqual(ws.sent.at(-1), { type: "cancel", id: "1" });
});

test("streams share one socket", async () => {
  const a = apiStream("/v1/events/events", () => {}, { token, socket: cfg });
  const b = apiStream("/v1/coordinator/asks/events", () => {}, { token, socket: cfg });
  await until(() => FakeSocket.made[0]?.calls().length === 2);
  assert.equal(FakeSocket.made.length, 1);
  a();
  b();
});

test("a path the server does not map is read as server-sent events, with no backoff", async () => {
  const got: unknown[] = [];
  const urls: string[] = [];
  const stop = apiStream("/v1/coordinator/plan/events", (d) => got.push(d), {
    token,
    socket: cfg,
    fetcher: (async (u: string) => {
      urls.push(String(u));
      return sse('data: {"via":"sse"}\n\n');
    }) as typeof fetch,
  });
  await until(() => FakeSocket.made[0]?.calls().length === 1);
  (FakeSocket.made[0] as FakeSocket).say({ type: "error", id: "1", status: "404 Not Found", message: "unmapped" });
  await until(() => got.length === 1);
  assert.deepEqual(got, [{ via: "sse" }]);
  assert.deepEqual(urls, ["/api/proxy/v1/coordinator/plan/events"]);
  stop();
});

test("a socket that cannot open falls back to server-sent events and is not retried at once", async () => {
  FakeSocket.refuse = true;
  const got: unknown[] = [];
  const stop = apiStream("/v1/events/events", (d) => got.push(d), {
    token,
    socket: cfg,
    fetcher: (async () => sse('data: {"via":"sse"}\n\n')) as typeof fetch,
  });
  await until(() => got.length === 1);
  const stop2 = apiStream("/v1/coordinator/asks/events", (d) => got.push(d), {
    token,
    socket: cfg,
    fetcher: (async () => sse('data: {"via":"sse2"}\n\n')) as typeof fetch,
  });
  await until(() => got.length === 2);
  assert.equal(FakeSocket.made.length, 1, "no second attempt during the cooldown");
  stop();
  stop2();
});

test("a refusal that will not change ends the stream; a break resumes after the last id", async () => {
  const states: [string, string | undefined][] = [];
  const stop = apiStream("/v1/events/events", () => {}, {
    token,
    socket: cfg,
    wait: async () => {},
    onState: (s, why) => states.push([s, why]),
  });
  await until(() => FakeSocket.made[0]?.calls().length === 1);
  (FakeSocket.made[0] as FakeSocket).say({ type: "error", id: "1", status: "403 Forbidden", message: "not allowed" });
  await until(() => states.some(([s]) => s === "offline"));
  assert.deepEqual(states.at(-1), ["offline", "not allowed"]);
  stop();

  const got: unknown[] = [];
  const stop2 = apiStream("/v1/atlas/signals/events", (d) => got.push(d), {
    token,
    socket: cfg,
    resumeParam: "after",
    wait: async () => {},
  });
  await until(() => FakeSocket.made.at(-1)?.calls().length === 1);
  const first = FakeSocket.made.at(-1) as FakeSocket;
  first.say({ type: "data", id: "2", data: { sse_id: "s9", n: 1 } });
  first.close();
  await until(() => FakeSocket.made.length === 2);
  const again = FakeSocket.made.at(-1) as FakeSocket;
  await until(() => again.calls().length === 1);
  assert.equal(again.calls()[0]?.path, "/v1/atlas/signals/events?after=s9");
  stop2();
});
