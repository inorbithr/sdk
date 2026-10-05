/**
 * The driver for `conformance/cases`: starts the replay server, loads every case, runs
 * its action through the generated public surface, and compares the result and the
 * server's verdict with `expect` (`conformance/README.md`). Without the server binary
 * (`mise run conformance:server:build`) it skips, unless IOHR_TEST_REQUIRE_REPLAY=1.
 */

import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { createInterface } from "node:readline";
import { test } from "node:test";
import * as otel from "@opentelemetry/api";

import {
  type CallOptions,
  Client,
  type ClientOptions,
  type InOrbitError,
  type LogRecord,
  type Middleware,
  Public,
  type RawResponse,
  type SpanLike,
  type TracerProviderLike,
} from "../../src/index.js";

interface Case {
  name: string;
  area: string;
  pending?: string[];
  action: {
    op: string;
    args?: Record<string, unknown>;
    repeat?: number;
    concurrent?: number;
    take?: number;
    options?: { idempotency_key?: string; traceparent?: string; timeout_ms?: number };
    rewrite?: { after: number; files: Record<string, string> };
  };
  client?: {
    max_retries?: number;
    timeout_ms?: number;
    key_id?: string;
    key_secret?: string;
    scopes?: string[];
    streams?: "sse" | "socket";
    stream_idle_timeout_ms?: number;
    load?: boolean;
    env?: Record<string, string>;
    config_file?: string;
    files?: Record<string, string>;
    profile?: string;
    credential_sources?: ("env" | "workload" | "file" | "cli")[];
    cli?: boolean;
    pipeline?: {
      add?: { name: string; stage: "per_call" | "per_retry"; before?: string; after?: string }[];
      remove?: string[];
    };
    log?: "off" | "error" | "warn" | "info" | "debug";
    log_headers?: boolean;
    log_allow_headers?: string[];
    rate_limit?: "observe" | "wait" | "off";
    total_timeout_ms?: number;
    retry_budget_capacity?: number;
    tracing?: boolean;
    transport?: "http" | "https" | "mtls" | "proxy";
    no_proxy?: string;
  };
  expect: {
    ok?: unknown;
    items?: unknown[];
    error?: {
      kind?: string;
      code?: string;
      status?: number;
      message_contains?: string;
      message_excludes?: string;
    };
    attempts?: number;
    token_exchanges?: number;
    probes?: Record<string, { count?: number; seen?: Record<string, string>[] }>;
    logs?: { contains?: Record<string, unknown>[]; excludes?: string[] };
    spans?: { name: string; kind?: "internal" | "client"; attributes?: Record<string, unknown> }[];
    rate_limit?: Record<string, unknown> | null;
    idempotency_key?: string;
    config?: Record<string, unknown>;
  };
}

interface Loaded {
  case: Case;
  base_url: string;
  http_url: string;
  proxy_url: string;
  ca_file: string;
  client_cert_file: string;
  client_key_file: string;
}

interface Verdict {
  status: string;
  token_exchanges?: number;
  attempts?: number;
  mismatch?: unknown;
  next?: unknown;
}

const root = resolve(process.cwd(), "..");
const bin = resolve(
  root,
  `conformance/server/bin/replay${process.platform === "win32" ? ".exe" : ""}`,
);

async function startReplay(): Promise<{ url: string; stop(): void } | undefined> {
  if (!existsSync(bin)) {
    const note =
      "conformance: no replay server at conformance/server/bin/replay; run `mise run conformance:server:build`";
    assert.ok(process.env.IOHR_TEST_REQUIRE_REPLAY === undefined, note);
    console.warn(`${note}; skipping`);
    return undefined;
  }
  const child = spawn(
    bin,
    ["--addr", "127.0.0.1:0", "--cases", resolve(root, "conformance/cases")],
    {
      stdio: ["ignore", "pipe", "inherit"],
    },
  );
  const lines = createInterface({ input: child.stdout });
  const first = await new Promise<string>((ok, fail) => {
    const timer = setTimeout(
      () => fail(new Error("the replay server did not announce itself within 10 s")),
      10_000,
    );
    lines.once("line", (line) => {
      clearTimeout(timer);
      ok(line);
    });
  });
  const prefix = "replay: listening on ";
  assert.ok(first.startsWith(prefix), `unexpected first line: ${first}`);
  return { url: first.slice(prefix.length).trim(), stop: () => child.kill() };
}

function subset(want: unknown, got: unknown): boolean {
  if (Array.isArray(want)) {
    return (
      Array.isArray(got) && want.length === got.length && want.every((w, i) => subset(w, got[i]))
    );
  }
  if (typeof want === "object" && want !== null) {
    return (
      typeof got === "object" &&
      got !== null &&
      Object.entries(want).every(([k, v]) => subset(v, (got as Record<string, unknown>)[k]))
    );
  }
  return want === got;
}

/** Header matchers: a literal, `*`, `$name` (captured, then equal) or `~regex` (whole value). */
function matches(want: string, got: string | undefined, captures: Map<string, string>): boolean {
  if (got === undefined) {
    return false;
  }
  if (want === "*") {
    return true;
  }
  if (want.startsWith("$")) {
    const seen = captures.get(want);
    if (seen === undefined) {
      captures.set(want, got);
      return true;
    }
    return seen === got;
  }
  if (want.startsWith("~")) {
    return new RegExp(`^(?:${want.slice(1)})$`).test(got);
  }
  return want === got;
}

function call(api: Public, action: Case["action"]): Promise<RawResponse> {
  const args = action.args ?? {};
  const arg = (k: string): string | undefined =>
    args[k] === undefined ? undefined : String(args[k]);
  const o = action.options ?? {};
  const options: CallOptions = {
    ...(o.idempotency_key === undefined ? {} : { idempotencyKey: o.idempotency_key }),
    ...(o.traceparent === undefined ? {} : { traceparent: o.traceparent }),
    ...(o.timeout_ms === undefined ? {} : { timeout: o.timeout_ms }),
  };
  switch (action.op) {
    case "me":
      return api.me(options).then((r) => r.raw);
    case "accounts.get_me":
      return api.accounts.getMe(options).then((r) => r.raw);
    case "accounts.get_usage": {
      const params = Object.fromEntries(
        Object.entries({ from: arg("from"), to: arg("to") }).filter(([, v]) => v !== undefined),
      );
      return api.accounts.getUsage(arg("org_id") ?? "", params, options).then((r) => r.raw);
    }
    case "radar.get_digest":
      return api.radar.getDigest(arg("id") ?? "", options).then((r) => r.raw);
    case "events.create_endpoint":
      return api.events
        .createEndpoint(
          args as {
            account_id?: string;
            url?: string;
            description?: string;
            event_types?: string[];
          },
          options,
        )
        .then((r) => r.raw);
    case "events.update_endpoint": {
      const { endpoint_id: _id, ...body } = args as {
        endpoint_id?: string;
        url?: string;
        description?: string;
        enabled?: boolean;
        event_types?: string[];
      };
      return api.events.updateEndpoint(arg("endpoint_id") ?? "", body, options).then((r) => r.raw);
    }
    case "events.delete_endpoint":
      return api.events.deleteEndpoint(arg("endpoint_id") ?? "", options).then((r) => r.raw);
    default:
      throw new Error(`the conformance schema names an op this driver does not know: ${action.op}`);
  }
}

/** A stream action: every item until the stream ends, `take` items, or the error. */
async function stream(
  api: Public,
  action: Case["action"],
): Promise<{ items: unknown[]; error?: InOrbitError }> {
  const args = action.args ?? {};
  if (action.op !== "events.stream_events") {
    throw new Error(
      `the conformance schema names a stream op this driver does not know: ${action.op}`,
    );
  }
  const params = Object.fromEntries(
    Object.entries({ types: args.types, account_id: args.account_id }).filter(
      ([, v]) => v !== undefined,
    ),
  ) as { types?: string; account_id?: string };
  const items: unknown[] = [];
  try {
    for await (const item of api.events.streamEvents(params)) {
      items.push(item);
      if (action.take !== undefined && items.length >= action.take) {
        break;
      }
    }
  } catch (e) {
    return { items, error: e as InOrbitError };
  }
  // The cancel frame leaves on the break; give the server a moment to read it.
  await new Promise((r) => setTimeout(r, 100));
  return { items };
}

function checkError(
  e: InOrbitError & { code?: string; status?: number },
  want: NonNullable<Case["expect"]["error"]>,
): string[] {
  const problems: string[] = [];
  if (want.kind !== undefined && e.kind !== want.kind) {
    problems.push(`error kind: want ${want.kind}, got ${e.kind} (${e.message})`);
  }
  if (e.kind === "api") {
    if (want.code !== undefined && e.code !== want.code) {
      problems.push(`error code: want ${want.code}, got ${e.code}`);
    }
    if (want.status !== undefined && e.status !== want.status) {
      problems.push(`error status: want ${want.status}, got ${e.status}`);
    }
  } else if (want.code !== undefined || want.status !== undefined) {
    problems.push(`error: want an API error, got ${e.message}`);
  }
  if (want.message_contains !== undefined && !e.message.includes(want.message_contains)) {
    problems.push(
      `message: want it to contain ${JSON.stringify(want.message_contains)}, got ${JSON.stringify(e.message)}`,
    );
  }
  if (want.message_excludes !== undefined && e.message.includes(want.message_excludes)) {
    problems.push(
      `message: must not contain ${JSON.stringify(want.message_excludes)}, got ${JSON.stringify(e.message)}`,
    );
  }
  return problems;
}

interface SpanRecord {
  name: string;
  kind: number;
  attributes: Record<string, unknown>;
}

/** An in-memory tracer: spans in start order, ids made up, parents from the context. */
function memoryTracer(): { provider: TracerProviderLike; spans: SpanRecord[] } {
  const spans: SpanRecord[] = [];
  const hex = (n: number): string =>
    [...crypto.getRandomValues(new Uint8Array(n))]
      .map((b) => b.toString(16).padStart(2, "0"))
      .join("");
  const tracer = {
    startSpan(
      name: string,
      options: { kind: number; attributes?: Record<string, string | number | boolean> },
      context?: unknown,
    ): SpanLike {
      const parent = otel.trace.getSpan((context ?? otel.context.active()) as otel.Context);
      const traceId = parent?.spanContext().traceId ?? hex(16);
      const record: SpanRecord = {
        name,
        kind: options.kind,
        attributes: { ...options.attributes },
      };
      spans.push(record);
      const sc = { traceId, spanId: hex(8), traceFlags: 1 };
      return {
        setAttribute(k, v) {
          record.attributes[k] = v;
          return this;
        },
        setStatus() {
          return this;
        },
        spanContext: () => sc,
        end() {},
      };
    },
  };
  return { provider: { getTracer: () => tracer }, spans };
}

function substitute(text: string, replay: string, dir: string): string {
  return text.replaceAll("{replay}", replay).replaceAll("{dir}", dir);
}

interface Built {
  client: Client;
  dir: string;
  records: LogRecord[];
  probes: Map<string, Headers[]>;
  spans: SpanRecord[];
}

function build(loaded: Loaded): Built {
  const c = loaded.case;
  const o = c.client ?? {};
  const dir = mkdtempSync(join(tmpdir(), "inorbit-case-"));
  for (const [name, content] of Object.entries(o.files ?? {})) {
    const p = join(dir, name);
    mkdirSync(dirname(p), { recursive: true });
    writeFileSync(p, substitute(content, loaded.base_url, dir));
  }
  const records: LogRecord[] = [];
  const keep = (_: string, r: LogRecord): void => {
    records.push(r);
  };
  const probes = new Map<string, Headers[]>();
  const tracer = memoryTracer();
  const common: ClientOptions = {
    ...(o.max_retries === undefined && o.load === true ? {} : { maxRetries: o.max_retries ?? 2 }),
    ...(o.timeout_ms === undefined ? {} : { timeout: o.timeout_ms }),
    ...(o.streams === undefined ? {} : { streams: o.streams }),
    ...(o.stream_idle_timeout_ms === undefined
      ? {}
      : { streamIdleTimeout: o.stream_idle_timeout_ms }),
    ...(o.log === undefined
      ? {}
      : { log: o.log, logger: { debug: keep, info: keep, warn: keep, error: keep } }),
    ...(o.log_headers === undefined ? {} : { logHeaders: o.log_headers }),
    ...(o.log_allow_headers === undefined ? {} : { logAllowHeaders: o.log_allow_headers }),
    ...(o.rate_limit === undefined ? {} : { rateLimit: o.rate_limit }),
    ...(o.total_timeout_ms === undefined ? {} : { totalTimeout: o.total_timeout_ms }),
    ...(o.retry_budget_capacity === undefined
      ? {}
      : { retryBudgetCapacity: o.retry_budget_capacity }),
    ...(o.tracing === undefined
      ? {}
      : o.tracing
        ? { tracing: true, tracerProvider: tracer.provider, opentelemetry: otel }
        : { tracing: false }),
    ...(o.transport === "https" || o.transport === "proxy" || o.transport === "mtls"
      ? { caBundle: loaded.ca_file }
      : {}),
    ...(o.transport === "mtls"
      ? { clientCert: loaded.client_cert_file, clientKey: loaded.client_key_file }
      : {}),
    ...(o.transport === "proxy" ? { proxy: loaded.proxy_url } : {}),
    ...(o.no_proxy === undefined ? {} : { noProxy: o.no_proxy }),
    ...(o.credential_sources === undefined ? {} : { credentialSources: o.credential_sources }),
    ...(o.cli === true ? { cliPath: bin } : {}),
    ...(o.pipeline === undefined
      ? {}
      : {
          pipeline: (p) => {
            for (const a of o.pipeline?.add ?? []) {
              const seen: Headers[] = [];
              probes.set(a.name, seen);
              const probe: Middleware = {
                name: a.name,
                handle(req, next) {
                  seen.push(new Headers(req.headers));
                  return next(req);
                },
              };
              if (a.before !== undefined) {
                p.insertBefore(a.before, probe);
              } else if (a.after !== undefined) {
                p.insertAfter(a.after, probe);
              } else if (a.stage === "per_call") {
                p.addPerCall(probe);
              } else {
                p.addPerRetry(probe);
              }
            }
            for (const name of o.pipeline?.remove ?? []) {
              p.remove(name);
            }
            return p;
          },
        }),
  };
  let client: Client;
  if (o.load === true) {
    const env = Object.fromEntries(
      Object.entries(o.env ?? {}).map(([k, v]) => [k, substitute(v, loaded.base_url, dir)]),
    );
    let configFile: string | undefined;
    if (o.config_file !== undefined) {
      configFile = join(dir, "config.toml");
      writeFileSync(configFile, substitute(o.config_file, loaded.base_url, dir));
    }
    client = Client.load({
      ...common,
      ...(configFile === undefined ? {} : { configFile }),
      ...(o.profile === undefined ? {} : { profile: o.profile }),
      loadOptions: { env, home: null, cwd: dir },
    });
  } else {
    client = new Client({
      ...common,
      baseUrl: loaded.base_url,
      tokenUrl: `${loaded.base_url}/oauth2/token`,
      keyId: o.key_id ?? "ak_test",
      keySecret: o.key_secret ?? "s3cr3t",
      scopes: o.scopes ?? ["identity:read"],
    });
  }
  return { client, dir, records, probes, spans: tracer.spans };
}

/** What the M6 expectations add: probes, logs, spans, the rate limit, the key, the config. */
function checkM6(c: Case, built: Built, last: RawResponse | InOrbitError | undefined): string[] {
  const problems: string[] = [];
  const want = c.expect;
  for (const [name, p] of Object.entries(want.probes ?? {})) {
    const seen = built.probes.get(name) ?? [];
    if (p.count !== undefined && seen.length !== p.count) {
      problems.push(`probe ${name}: ran ${seen.length} times, want ${p.count}`);
    }
    const captures = new Map<string, string>();
    for (const [i, headers] of (p.seen ?? []).entries()) {
      for (const [h, v] of Object.entries(headers)) {
        const got = seen[i]?.get(h) ?? undefined;
        if (!matches(v, got, captures)) {
          problems.push(
            `probe ${name} request ${i + 1}: ${h} is ${JSON.stringify(got)}, want ${v}`,
          );
        }
      }
    }
  }
  if (want.logs !== undefined) {
    for (const w of want.logs.contains ?? []) {
      if (!built.records.some((r) => subset(w, r))) {
        problems.push(
          `logs: no record holds ${JSON.stringify(w)}: ${JSON.stringify(built.records)}`,
        );
      }
    }
    const text = JSON.stringify(built.records);
    for (const x of want.logs.excludes ?? []) {
      if (text.includes(x)) {
        problems.push(`logs: ${JSON.stringify(x)} appears in ${text}`);
      }
    }
  }
  if (want.spans !== undefined) {
    const kinds: Record<string, number> = { internal: 0, client: 2 };
    if (built.spans.length !== want.spans.length) {
      problems.push(`spans: want ${want.spans.length}, got ${JSON.stringify(built.spans)}`);
    } else {
      for (const [i, w] of want.spans.entries()) {
        const s = built.spans[i];
        if (
          s === undefined ||
          s.name !== w.name ||
          (w.kind !== undefined && s.kind !== kinds[w.kind]) ||
          !subset(w.attributes ?? {}, s.attributes)
        ) {
          problems.push(`span ${i}: want ${JSON.stringify(w)}, got ${JSON.stringify(s)}`);
        }
      }
    }
  }
  const raw = last instanceof Error ? undefined : last;
  if (want.rate_limit !== undefined) {
    const r = raw?.rateLimit;
    const got =
      r === undefined
        ? null
        : {
            ...(r.limit === undefined ? {} : { limit: r.limit }),
            ...(r.remaining === undefined ? {} : { remaining: r.remaining }),
            ...(r.reset === undefined ? {} : { reset_ms: r.reset }),
          };
    if (want.rate_limit === null ? got !== null : !subset(want.rate_limit, got)) {
      problems.push(
        `rate_limit: want ${JSON.stringify(want.rate_limit)}, got ${JSON.stringify(got)}`,
      );
    }
  }
  if (want.idempotency_key !== undefined) {
    const k = raw?.idempotencyKey;
    const ok =
      want.idempotency_key === "*" ? k !== undefined && k !== "" : k === want.idempotency_key;
    if (!ok) {
      problems.push(`idempotency_key: want ${want.idempotency_key}, got ${k}`);
    }
  }
  if (want.config !== undefined) {
    const d = built.client.config().describe();
    if (!subset(want.config, d)) {
      problems.push(`config: want ${JSON.stringify(want.config)} in ${JSON.stringify(d)}`);
    }
  }
  return problems;
}

test("every case passes", async () => {
  const replay = await startReplay();
  if (replay === undefined) {
    return;
  }
  try {
    const { cases } = (await (await fetch(`${replay.url}/_cases`)).json()) as { cases: string[] };
    assert.ok(cases.length > 0, "no cases listed");
    const failed: string[] = [];
    let passed = 0;
    for (const name of cases) {
      const answer = await fetch(`${replay.url}/_case`, {
        method: "POST",
        body: JSON.stringify({ name }),
      });
      if (answer.status === 501) {
        console.log(`skip ${name}: not implemented by the replay server yet`);
        continue;
      }
      assert.ok(answer.ok, `${name}: loading answered ${answer.status}`);
      const loaded = (await answer.json()) as Loaded;
      const c = loaded.case;
      if (c.pending?.includes("ts")) {
        console.log(`skip ${name}: pending for ts`);
        continue;
      }
      let built: Built;
      try {
        built = build(loaded);
      } catch (e) {
        failed.push(`${name}:\n  building the client: ${(e as Error).message}`);
        continue;
      }
      const api = new Public(built.client);
      const problems: string[] = [];
      try {
        if (c.action.op === "events.stream_events") {
          const got = await stream(api, c.action);
          if (c.expect.items !== undefined && !subset(c.expect.items, got.items)) {
            problems.push(
              `items: want ${JSON.stringify(c.expect.items)}, got ${JSON.stringify(got.items)}`,
            );
          }
          if (got.error !== undefined && c.expect.error === undefined) {
            problems.push(`want the stream to end cleanly, got ${got.error.message}`);
          } else if (got.error === undefined && c.expect.error !== undefined) {
            problems.push("want an error, the stream ended cleanly");
          } else if (got.error !== undefined && c.expect.error !== undefined) {
            problems.push(...checkError(got.error, c.expect.error));
          }
        } else {
          const run = (): Promise<RawResponse | InOrbitError> =>
            call(api, c.action).catch((e: InOrbitError) => e);
          const results: (RawResponse | InOrbitError)[] = [];
          if (c.action.concurrent !== undefined) {
            results.push(...(await Promise.all(Array.from({ length: c.action.concurrent }, run))));
          } else {
            for (let i = 0; i < (c.action.repeat ?? 1); i++) {
              results.push(await run());
              if (c.action.rewrite !== undefined && i + 1 === c.action.rewrite.after) {
                for (const [file, content] of Object.entries(c.action.rewrite.files)) {
                  writeFileSync(join(built.dir, file), content);
                }
              }
            }
          }
          for (const r of results) {
            const isError = r instanceof Error;
            if (!isError && c.expect.ok !== undefined) {
              const got = r.json();
              if (!subset(c.expect.ok, got)) {
                problems.push(
                  `ok: want a superset of ${JSON.stringify(c.expect.ok)}, got ${JSON.stringify(got)}`,
                );
              }
            } else if (!isError && c.expect.error !== undefined) {
              problems.push(`want an error, got HTTP ${r.status}`);
            } else if (isError && c.expect.error !== undefined) {
              problems.push(...checkError(r, c.expect.error));
            } else if (isError && c.expect.ok !== undefined) {
              problems.push(`want ok, got ${r.message}`);
            }
          }
          problems.push(...checkM6(c, built, results.at(-1)));
        }
        const verdict = (await (await fetch(`${replay.url}/_result`)).json()) as Verdict;
        if (verdict.status !== "pass") {
          problems.unshift(
            `server: ${verdict.status} mismatch=${JSON.stringify(verdict.mismatch)} next=${JSON.stringify(verdict.next)}`,
          );
        }
        if (c.expect.attempts !== undefined && verdict.attempts !== c.expect.attempts) {
          problems.push(`attempts: want ${c.expect.attempts}, got ${verdict.attempts}`);
        }
        if (
          c.expect.token_exchanges !== undefined &&
          verdict.token_exchanges !== c.expect.token_exchanges
        ) {
          problems.push(
            `token_exchanges: want ${c.expect.token_exchanges}, got ${verdict.token_exchanges}`,
          );
        }
      } finally {
        rmSync(built.dir, { recursive: true, force: true });
      }
      if (problems.length === 0) {
        passed++;
        console.log(`pass ${name}`);
      } else {
        failed.push(`${name}:\n  ${problems.join("\n  ")}`);
      }
    }
    console.log(`conformance: ${passed} passed, ${failed.length} failed`);
    assert.deepEqual(failed, [], failed.join("\n"));
  } finally {
    replay.stop();
  }
});
