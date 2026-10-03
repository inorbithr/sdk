import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { inspect } from "node:util";

import {
  ApiError,
  Client,
  ClientCredentials,
  ConfigError,
  codegen,
  RawResponse,
  StaticToken,
} from "../src/index.js";
import { CODES, codeForStatus } from "../src/errors.js";
import { backoffMs, retryAfterMs } from "../src/retry.js";

function raw(status: number, body: string, headers: Record<string, string> = {}): RawResponse {
  return new RawResponse({
    status,
    headers: new Headers(headers),
    body: new TextEncoder().encode(body),
    requestId: "iohr-0",
    attempts: 1,
  });
}

describe("codegen", () => {
  it("encodes a path parameter as one segment", () => {
    assert.equal(codegen.pathSegment("a/b c!"), "a%2Fb%20c%21");
    assert.equal(codegen.pathSegment("org_1-x.y~"), "org_1-x.y~");
  });

  it("converts 64-bit integers both ways through models, lists and maps", () => {
    const shapes: codegen.Shapes = {
      Usage: { amount: "i64", rows: { array: { ref: "Row" } } },
      Row: { n: "i64", by: { map: "i64" } },
    };
    const wire = { amount: "9007199254740993", rows: [{ n: "2", by: { a: "3" }, label: "x" }] };
    const value = codegen.decode(wire, { ref: "Usage" }, shapes) as {
      amount: bigint;
      rows: { n: bigint; by: Record<string, bigint>; label: string }[];
    };
    assert.equal(value.amount, 9007199254740993n);
    assert.equal(value.rows[0]?.by.a, 3n);
    assert.equal(value.rows[0]?.label, "x");
    assert.deepEqual(codegen.encode(value, { ref: "Usage" }, shapes), wire);
  });
});

describe("errors", () => {
  it("reads the problem envelope and keeps an unknown code", () => {
    const e = new ApiError(
      raw(
        409,
        JSON.stringify({
          code: "brand_new",
          error: "taken",
          details: [{ type: "retry", after_seconds: "4" }],
        }),
        {
          "x-request-id": "srv-1",
        },
      ),
    );
    assert.equal(e.code, "brand_new");
    assert.equal(e.kind, "api");
    assert.equal(e.retryAfterSeconds(), 4);
    assert.match(e.message, /^taken \(brand_new, HTTP 409, request id srv-1\)$/);
  });

  it("maps a plain-text gateway answer by status", () => {
    const e = new ApiError(raw(403, "RBAC: access denied"));
    assert.equal(e.code, "forbidden");
    assert.equal(e.problem, "RBAC: access denied");
    assert.equal(new ApiError(raw(503, "")).problem, "the API failed to answer");
  });
});

describe("client", () => {
  it("refuses to start without credentials, naming the variables", () => {
    assert.throws(
      () => new Client({}),
      (e) => e instanceof ConfigError && /INORBIT_TOKEN/.test(e.message),
    );
    assert.throws(
      () => new Client({ keyId: "ak_1", keySecret: "s" }),
      (e) => e instanceof ConfigError && /INORBIT_SCOPES/.test(e.message),
    );
  });

  it("calls only https, or plain http to this machine", () => {
    assert.throws(() => new Client({ token: "t", baseUrl: "http://example.com" }), ConfigError);
    assert.throws(
      () => new Client({ token: "t", baseUrl: "https://api.inorbit.hr/v1" }),
      ConfigError,
    );
    assert.equal(
      new Client({ token: "t", baseUrl: "http://127.0.0.1:8080" }).baseUrl,
      "http://127.0.0.1:8080",
    );
  });

  it("reads a named profile's variables and nothing else", () => {
    const env = process.env;
    process.env = { INORBIT_TOKEN: "bare", INORBIT_BASE_URL: "http://localhost:1" };
    try {
      assert.throws(
        () => Client.fromEnv("ACME_CI"),
        (e) => e instanceof ConfigError && /INORBIT_ACME_CI_TOKEN/.test(e.message),
      );
      process.env = { INORBIT_ACME_CI_TOKEN: "t", INORBIT_BASE_URL: "http://localhost:1" };
      assert.equal(Client.fromEnv("ACME_CI").baseUrl, "http://localhost:1");
    } finally {
      process.env = env;
    }
  });
});

describe("credentials", () => {
  it("never print a secret", () => {
    const cc = new ClientCredentials({
      keyId: "ak_1",
      keySecret: "s3cr3t",
      scopes: ["identity:read"],
    });
    for (const text of [
      JSON.stringify(cc),
      String(cc),
      inspect(cc),
      inspect(new StaticToken("tok3n")),
    ]) {
      assert.doesNotMatch(text, /s3cr3t|tok3n/);
    }
  });

  it("make one exchange however many calls wait for it", async () => {
    let exchanges = 0;
    const fetch: typeof globalThis.fetch = async () => {
      exchanges++;
      await new Promise((r) => setTimeout(r, 20));
      return new Response(JSON.stringify({ access_token: "t1", expires_in: 900 }), { status: 200 });
    };
    const cc = new ClientCredentials({ keyId: "ak_1", keySecret: "s", scopes: ["a:b"], fetch });
    const tokens = await Promise.all(Array.from({ length: 10 }, () => cc.token()));
    assert.equal(exchanges, 1);
    assert.ok(tokens.every((t) => t.access === "t1"));
    cc.invalidate();
    await cc.token();
    assert.equal(exchanges, 2);
  });
});

describe("retry", () => {
  it("caps Retry-After and keeps backoff under its ceiling", () => {
    assert.equal(retryAfterMs(new Headers({ "retry-after": "3" })), 3000);
    assert.equal(retryAfterMs(new Headers({ "retry-after": "900" })), 60_000);
    assert.equal(retryAfterMs(new Headers({ "retry-after": "Wed, 21 Oct" })), undefined);
    for (let i = 0; i < 10; i++) {
      assert.ok(backoffMs(i) <= 8000);
    }
  });
});

describe("codes", () => {
  it("knows unprocessable and keeps an unknown code", () => {
    assert.equal(CODES.unprocessable, 422);
    assert.equal(codeForStatus(422), "unprocessable");
    assert.equal(codeForStatus(418), "http_418");
  });
});
