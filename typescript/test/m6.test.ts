import assert from "node:assert/strict";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, it } from "node:test";
import { inspect } from "node:util";

import {
  AuthError,
  CachedToken,
  ChainedCredential,
  Client,
  CliToken,
  ConfigError,
  DefaultCredential,
  type LogRecord,
  loadConfig,
  type Middleware,
  StaticToken,
  type Token,
  TokenFile,
} from "../src/index.js";

const ok = (body: unknown = { subject: "ak_1" }, headers: Record<string, string> = {}) =>
  new Response(JSON.stringify(body), {
    status: 200,
    headers: { "content-type": "application/json", ...headers },
  });

/** A client whose `fetch` answers with `answer` and records what it was sent. */
function stubbed(
  answer: (url: string, init: RequestInit) => Response,
  extra: Parameters<typeof Client.load>[0] = {},
): { client: Client; sent: Headers[] } {
  const sent: Headers[] = [];
  const client = Client.load({
    fetch: (async (url: string | URL, init?: RequestInit) => {
      sent.push(new Headers(init?.headers));
      return answer(String(url), init ?? {});
    }) as typeof fetch,
    loadOptions: { env: { INORBIT_TOKEN: "tok-secret", INORBIT_CONFIG_FILE: "off" }, home: null },
    ...extra,
  });
  return { client, sent };
}

const probe = (name: string, seen: string[]): Middleware => ({
  name,
  handle(req, next) {
    seen.push(`${name}:${req.info.stage}:${req.info.attempt}`);
    return next(req);
  },
});

describe("the pipeline", () => {
  it("lists the built-ins and puts additions at their slots", () => {
    const { client } = stubbed(() => ok(), {
      pipeline: (p) =>
        p
          .addPerCall(probe("call-a", []))
          .addPerCall(probe("call-b", []))
          .addPerRetry(probe("try", []))
          .insertAfter("auth", probe("after-auth", []))
          .replace("rate_limit", probe("mine", []))
          .remove("hooks"),
    });
    assert.deepEqual(client.config().describe().pipeline, [
      "request_id",
      "user_agent",
      "idempotency_key",
      "call_tracing",
      "deadline",
      "call-a",
      "call-b",
      "retry",
      "auth",
      "after-auth",
      "rate_limit",
      "attempt_tracing",
      "logging",
      "try",
      "timeout",
    ]);
  });

  it("refuses removing retry, auth or timeout, duplicates and unknown names", () => {
    for (const edit of [
      (p: import("../src/index.js").Pipeline) => p.remove("retry"),
      (p: import("../src/index.js").Pipeline) => p.remove("auth"),
      (p: import("../src/index.js").Pipeline) => p.remove("timeout"),
      (p: import("../src/index.js").Pipeline) => p.addPerCall(probe("auth", [])),
      (p: import("../src/index.js").Pipeline) => p.insertBefore("nope", probe("x", [])),
    ]) {
      assert.throws(() => stubbed(() => ok(), { pipeline: edit }), ConfigError);
    }
  });

  it("runs per-call middleware once and per-retry middleware on every attempt", async () => {
    const seen: string[] = [];
    let n = 0;
    const { client } = stubbed(
      () =>
        n++ === 0 ? new Response("{}", { status: 503, headers: { "retry-after": "0" } }) : ok(),
      {
        pipeline: (p) => p.addPerCall(probe("call", seen)).addPerRetry(probe("try", seen)),
      },
    );
    await client.request({ name: "me", method: "GET", path: "/v1/me" });
    assert.deepEqual(seen, ["call:per_call:0", "try:per_retry:1", "try:per_retry:2"]);
  });

  it("lets a middleware answer without the network", async () => {
    let called = false;
    const { client } = stubbed(
      () => {
        called = true;
        return ok();
      },
      {
        pipeline: (p) =>
          p.addPerRetry({
            name: "canned",
            handle: async (req) => ({
              status: 200,
              headers: new Headers({ "content-type": "application/json" }),
              body: new TextEncoder().encode('{"subject":"canned"}'),
              stream: undefined,
              request: req,
            }),
          }),
      },
    );
    const { value } = await client.request<{ subject: string }>({
      method: "GET",
      path: "/v1/me",
    });
    assert.equal(value.subject, "canned");
    assert.equal(called, false);
  });
});

describe("idempotency keys", () => {
  it("are generated once per call, sent on every attempt, and on the result", async () => {
    let n = 0;
    const { client, sent } = stubbed(() =>
      n++ === 0 ? new Response("{}", { status: 503, headers: { "retry-after": "0" } }) : ok(),
    );
    const raw = await client.send({
      method: "POST",
      path: "/v1/webhooks/endpoints",
      body: {},
      idempotencyKey: true,
    });
    const keys = sent.map((h) => h.get("idempotency-key"));
    assert.equal(keys.length, 2);
    assert.match(keys[0] ?? "", /^[0-9a-f-]{36}$/);
    assert.equal(keys[0], keys[1]);
    assert.equal(raw.idempotencyKey, keys[0]);
  });

  it("are refused on an operation that does not take one", async () => {
    const { client } = stubbed(() => ok());
    await assert.rejects(
      client.send({ method: "PATCH", path: "/v1/x", body: {} }, { idempotencyKey: "k" }),
      ConfigError,
    );
  });

  it("leave a write without the mark unretried", async () => {
    const { client, sent } = stubbed(() => new Response("{}", { status: 503 }));
    await assert.rejects(client.send({ method: "POST", path: "/v1/x", body: {} }));
    assert.equal(sent.length, 1);
  });
});

describe("credentials", () => {
  it("fails a refused static token from load with an AuthError", async () => {
    const { client } = stubbed(() => new Response("expired", { status: 401 }));
    await assert.rejects(client.send({ method: "GET", path: "/v1/me" }), AuthError);
  });

  it("keeps explicit construction's 401 an ApiError", async () => {
    const client = new Client({
      token: "t",
      fetch: (async () => new Response("expired", { status: 401 })) as typeof fetch,
    });
    await assert.rejects(client.send({ method: "GET", path: "/v1/me" }), { name: "ApiError" });
  });

  it("caches one refresh for concurrent callers and keeps a valid token when a refresh fails", async () => {
    let calls = 0;
    let fail = false;
    const source = {
      token: async (): Promise<Token> => {
        calls++;
        await new Promise((r) => setTimeout(r, 5));
        if (fail) {
          throw new AuthError("down");
        }
        return { access: `t${calls}`, expiresAt: Date.now() + 400 };
      },
    };
    const failed: unknown[] = [];
    const cached = new CachedToken(source, { onRefreshFailed: (e) => failed.push(e) });
    const [a, b] = await Promise.all([cached.token(), cached.token()]);
    assert.equal(calls, 1);
    assert.equal(a.access, b.access);
    // Past four fifths of its life the token is due; the refresh fails; the valid one is used.
    await new Promise((r) => setTimeout(r, 340));
    fail = true;
    assert.equal((await cached.token()).access, "t1");
    assert.equal(failed.length, 1);
    // Within 5 s of a failed refresh the cached token is served without another try.
    assert.equal((await cached.token()).access, "t1");
    assert.equal(calls, 2);
  });

  it("reads a token file again after a refusal", async () => {
    const dir = mkdtempSync(join(tmpdir(), "inorbit-tf-"));
    const path = join(dir, "token");
    writeFileSync(path, "first\n");
    const tf = new TokenFile(path);
    assert.equal((await tf.token()).access, "first");
    writeFileSync(path, "second");
    assert.equal((await tf.token()).access, "first");
    tf.invalidate();
    assert.equal((await tf.token()).access, "second");
  });

  it("chains providers and keeps the first that works", async () => {
    const chain = new ChainedCredential([
      { token: () => Promise.reject(new AuthError("no")) },
      new StaticToken("second"),
    ]);
    assert.equal((await chain.token()).access, "second");
  });

  it("never prints a secret", () => {
    const dir = mkdtempSync(join(tmpdir(), "inorbit-dc-"));
    writeFileSync(join(dir, "s"), "file-secret");
    const values = [
      new CachedToken(new StaticToken("tok-secret")),
      new TokenFile(join(dir, "s")),
      new CliToken({ profile: "dev" }),
      new ChainedCredential([new StaticToken("tok-secret")]),
      new DefaultCredential({
        loadOptions: {
          env: { INORBIT_TOKEN: "tok-secret", INORBIT_CONFIG_FILE: "off" },
          home: null,
        },
      }),
    ];
    for (const v of values) {
      for (const text of [JSON.stringify(v), String(v), inspect(v)]) {
        assert.ok(!text.includes("tok-secret") && !text.includes("file-secret"), text);
      }
    }
    const described = JSON.stringify(
      loadConfig({
        loadOptions: {
          env: {
            INORBIT_KEY_ID: "ak_1",
            INORBIT_KEY_SECRET: "key-secret",
            INORBIT_SCOPES: "a:b",
            INORBIT_PROXY: "http://u:proxy-secret@p.example:1",
            INORBIT_CONFIG_FILE: "off",
          },
          home: null,
        },
      }),
    );
    assert.ok(!described.includes("key-secret") && !described.includes("proxy-secret"));
  });
});

describe("logging", () => {
  it("logs metadata only, headers from the allowlist", async () => {
    const records: LogRecord[] = [];
    const keep = (_: string, r: LogRecord) => records.push(r);
    const { client } = stubbed(() => ok({ secret: "body-secret" }, { "x-other": "v" }), {
      log: "debug",
      logHeaders: true,
      logger: { debug: keep, info: keep, warn: keep, error: keep },
    });
    await client.send({
      name: "me",
      method: "GET",
      path: "/v1/me",
      query: [["q", "query-secret"]],
    });
    const text = JSON.stringify(records);
    for (const secret of ["tok-secret", "body-secret", "query-secret", "Bearer"]) {
      assert.ok(!text.includes(secret), `${secret} in ${text}`);
    }
    const response = records.find((r) => r.event === "response") as unknown as { headers: object };
    assert.deepEqual(response.headers, {
      "content-type": "application/json",
      "x-other": "REDACTED",
    });
    assert.ok(records.some((r) => r.event === "call" && r.status === 200));
  });

  it("is silent by default", async () => {
    const records: LogRecord[] = [];
    const keep = (_: string, r: LogRecord) => records.push(r);
    const { client } = stubbed(() => ok(), {
      logger: { debug: keep, info: keep, warn: keep, error: keep },
    });
    await client.send({ method: "GET", path: "/v1/me" });
    assert.equal(records.length, 0);
  });
});

describe("rate limits", () => {
  it("keeps the latest snapshot on the result and the client", async () => {
    const { client } = stubbed(() =>
      ok(undefined, {
        "x-ratelimit-limit": "10",
        "x-ratelimit-remaining": "4",
        "x-ratelimit-reset": "2",
      }),
    );
    const raw = await client.send({ method: "GET", path: "/v1/me" });
    assert.deepEqual(raw.rateLimit, { limit: 10, remaining: 4, reset: 2000 });
    assert.deepEqual(client.rateLimit(), raw.rateLimit);
  });
});

describe("load", () => {
  it("reports every problem with its source, never a secret", () => {
    assert.throws(
      () =>
        Client.load({
          loadOptions: {
            env: { INORBIT_TOKEN: "t", INORBIT_TIMEOUT: "30", INORBIT_MAX_RETRIES: "-1" },
            home: null,
          },
        }),
      (e: unknown) => {
        assert.ok(e instanceof ConfigError);
        assert.deepEqual(
          e.problems.map((p) => [p.setting, p.source]),
          [
            ["timeout", "env INORBIT_TIMEOUT"],
            ["max_retries", "env INORBIT_MAX_RETRIES"],
          ],
        );
        assert.match(e.message, /^configuration is invalid \(2 problems\):/);
        return true;
      },
    );
  });

  it("refuses transport settings in code next to a caller's fetch", () => {
    assert.throws(
      () => stubbed(() => ok(), { proxy: "http://proxy.example:3128" }),
      (e: unknown) => e instanceof ConfigError && e.problems[0]?.setting === "proxy",
    );
  });

  it("describes explicit construction from code and defaults", () => {
    const d = new Client({ token: "t" }).config().describe();
    assert.deepEqual(d.credential, {
      source: "code",
      kind: "static_token",
      tried: [{ source: "code", result: "used" }],
    });
    assert.deepEqual(d.settings.timeout, { value: "30s", source: "default" });
    assert.equal(d.settings.token?.value, "<redacted>");
  });

  it("sends the normalised user agent", async () => {
    const { client, sent } = stubbed(() => ok(), { userAgentSuffix: "app/1.0" });
    await client.send({ method: "GET", path: "/v1/me" });
    assert.match(
      sent[0]?.get("user-agent") ?? "",
      /^inorbithr-sdk-typescript\/\S+ (node|deno|bun)\/\S+ (linux|macos|windows|freebsd|other)\/(x86_64|aarch64|x86|arm|riscv64|other) app\/1\.0$/,
    );
  });
});
