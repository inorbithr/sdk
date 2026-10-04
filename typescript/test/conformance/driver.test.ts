/**
 * The driver for `conformance/cases`: starts the replay server, loads every case, runs
 * its action through the generated public surface, and compares the result and the
 * server's verdict with `expect` (`conformance/README.md`). Without the server binary
 * (`mise run conformance:server:build`) it skips, unless IOHR_TEST_REQUIRE_REPLAY=1.
 */

import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { existsSync } from "node:fs";
import { resolve } from "node:path";
import { createInterface } from "node:readline";
import { test } from "node:test";

import { Client, type InOrbitError, Public, type RawResponse } from "../../src/index.js";

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
  };
  client?: {
    max_retries?: number;
    timeout_ms?: number;
    key_id?: string;
    key_secret?: string;
    scopes?: string[];
    streams?: "sse" | "socket";
    stream_idle_timeout_ms?: number;
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
  };
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

function call(api: Public, action: Case["action"]): Promise<RawResponse> {
  const args = action.args ?? {};
  const arg = (k: string): string | undefined =>
    args[k] === undefined ? undefined : String(args[k]);
  switch (action.op) {
    case "me":
      return api.me().then((r) => r.raw);
    case "accounts.get_me":
      return api.accounts.getMe().then((r) => r.raw);
    case "accounts.get_usage": {
      const params = Object.fromEntries(
        Object.entries({ from: arg("from"), to: arg("to") }).filter(([, v]) => v !== undefined),
      );
      return api.accounts.getUsage(arg("org_id") ?? "", params).then((r) => r.raw);
    }
    case "radar.get_digest":
      return api.radar.getDigest(arg("id") ?? "").then((r) => r.raw);
    case "events.create_endpoint":
      return api.events
        .createEndpoint(
          args as {
            account_id?: string;
            url?: string;
            description?: string;
            event_types?: string[];
          },
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
      return api.events.updateEndpoint(arg("endpoint_id") ?? "", body).then((r) => r.raw);
    }
    case "events.delete_endpoint":
      return api.events.deleteEndpoint(arg("endpoint_id") ?? "").then((r) => r.raw);
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

test("every case passes", async () => {
  const replay = await startReplay();
  if (replay === undefined) {
    return;
  }
  try {
    const { cases } = (await (await fetch(`${replay.url}/_cases`)).json()) as { cases: string[] };
    assert.ok(cases.length > 0, "no cases listed");
    const failed: string[] = [];
    for (const name of cases) {
      const loaded = await fetch(`${replay.url}/_case`, {
        method: "POST",
        body: JSON.stringify({ name }),
      });
      if (loaded.status === 501) {
        console.log(`skip ${name}: not implemented by the replay server yet`);
        continue;
      }
      assert.ok(loaded.ok, `${name}: loading answered ${loaded.status}`);
      const c = ((await loaded.json()) as { case: Case }).case;
      if (c.pending?.includes("ts")) {
        console.log(`skip ${name}: pending for ts`);
        continue;
      }
      const o = c.client ?? {};
      const client = new Client({
        baseUrl: replay.url,
        tokenUrl: `${replay.url}/oauth2/token`,
        keyId: o.key_id ?? "ak_test",
        keySecret: o.key_secret ?? "s3cr3t",
        scopes: o.scopes ?? ["identity:read"],
        maxRetries: o.max_retries ?? 2,
        ...(o.timeout_ms === undefined ? {} : { timeout: o.timeout_ms }),
        ...(o.streams === undefined ? {} : { streams: o.streams }),
        ...(o.stream_idle_timeout_ms === undefined
          ? {}
          : { streamIdleTimeout: o.stream_idle_timeout_ms }),
      });
      const api = new Public(client);
      if (c.action.op === "events.stream_events") {
        const got = await stream(api, c.action);
        const verdict = (await (await fetch(`${replay.url}/_result`)).json()) as Verdict;
        const problems: string[] = [];
        if (verdict.status !== "pass") {
          problems.push(
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
        if (problems.length === 0) {
          console.log(`pass ${name}`);
        } else {
          failed.push(`${name}:\n  ${problems.join("\n  ")}`);
        }
        continue;
      }
      const run = (): Promise<RawResponse | InOrbitError> =>
        call(api, c.action).catch((e: InOrbitError) => e);
      const results: (RawResponse | InOrbitError)[] = [];
      if (c.action.concurrent !== undefined) {
        results.push(...(await Promise.all(Array.from({ length: c.action.concurrent }, run))));
      } else {
        for (let i = 0; i < (c.action.repeat ?? 1); i++) {
          results.push(await run());
        }
      }
      const verdict = (await (await fetch(`${replay.url}/_result`)).json()) as Verdict;
      const problems: string[] = [];
      if (verdict.status !== "pass") {
        problems.push(
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
      if (problems.length === 0) {
        console.log(`pass ${name}`);
      } else {
        failed.push(`${name}:\n  ${problems.join("\n  ")}`);
      }
    }
    assert.deepEqual(failed, [], failed.join("\n"));
  } finally {
    replay.stop();
  }
});
