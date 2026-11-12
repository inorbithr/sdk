// The passes-on-init test: boots the service on a free port and checks what a service owes.

import assert from "node:assert/strict";
import type { AddressInfo } from "node:net";
import { after, before, describe, test } from "node:test";

import { loadConfig } from "../src/main.ts";
import { Logger } from "../src/obs.ts";
import { Service, makeServer } from "../src/server.ts";

const lines: string[] = [];
const svc = new Service(new Logger({ write: (l: string) => lines.push(l) }));
const server = makeServer(svc);
let base = "";

before(async () => {
  await new Promise<void>((ok) => server.listen(0, "127.0.0.1", ok));
  base = `http://127.0.0.1:${(server.address() as AddressInfo).port}`;
  svc.ready = true;
});
after(() => {
  server.close();
  server.closeAllConnections();
});

const records = () => lines.map((l) => JSON.parse(l) as Record<string, unknown>);

describe("service", () => {
  test("probes answer", async () => {
    assert.equal((await fetch(`${base}/healthz`)).status, 200);
    assert.equal((await fetch(`${base}/readyz`)).status, 200);
    const hello = await fetch(`${base}/v1/hello?name=Ada`);
    assert.deepEqual(await hello.json(), { message: "Hello, Ada" });
    assert.equal((await fetch(`${base}/nope`)).status, 404);
  });

  test("one span with the route and the trace id in the log, and no secret", async () => {
    const canary = "canary-value-must-not-be-logged";
    process.env["SERVICE_CANARY"] = canary;
    lines.length = 0;
    const trace = "4bf92f3577b34da6a3ce929d0e0e4736";
    const res = await fetch(`${base}/v1/hello?name=secret-query-value`, {
      headers: { traceparent: `00-${trace}-00f067aa0ba902b7-01`, authorization: "Bearer secret-header-value" },
    });
    assert.ok(res.headers.get("traceparent")?.startsWith(`00-${trace}-`));
    const spans = records().filter((r) => r["msg"] === "span");
    const requests = records().filter((r) => r["msg"] === "request");
    assert.equal(spans.length, 1);
    assert.equal(requests.length, 1);
    assert.equal(spans[0]!["route"], "/v1/hello");
    assert.equal(spans[0]!["trace_id"], trace);
    assert.equal(spans[0]!["parent_span_id"], "00f067aa0ba902b7");
    assert.equal(requests[0]!["trace_id"], trace);
    const all = lines.join("");
    for (const secret of [canary, "secret-query-value", "secret-header-value"]) {
      assert.ok(!all.includes(secret), `the log holds ${secret}`);
    }
    for (const value of Object.values(process.env)) {
      if (value && value.length >= 12) assert.ok(!all.includes(value), "the log holds an environment value");
    }
    delete process.env["SERVICE_CANARY"];
  });

  test("not ready while shutting down", async () => {
    svc.ready = false;
    assert.equal((await fetch(`${base}/readyz`)).status, 503);
    assert.equal((await fetch(`${base}/healthz`)).status, 200);
    svc.ready = true;
  });

  test("an unknown path is never logged and metrics count by route", async () => {
    lines.length = 0;
    await fetch(`${base}/private/user-1234`);
    await fetch(`${base}/healthz`);
    assert.ok(!lines.join("").includes("user-1234"));
    const metrics = await (await fetch(`${base}/metrics`)).text();
    assert.match(metrics, /route="unmatched",status="404"/);
    assert.match(metrics, /route="\/healthz",status="200"/);
  });

  test("configuration has defaults and refuses a bad port", () => {
    assert.deepEqual(loadConfig({}), { host: "127.0.0.1", port: 8080 });
    assert.throws(() => loadConfig({ PORT: "99999" }));
  });
});
