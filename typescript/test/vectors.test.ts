/**
 * The pure-function vectors of `conformance/vectors/` (docs/config.md section 9.2):
 * configuration resolution, config file paths, `no_proxy`, rate-limit headers and
 * durations, each run against the runtime without a server.
 */

import assert from "node:assert/strict";
import {
  chmodSync,
  mkdirSync,
  mkdtempSync,
  readdirSync,
  readFileSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { describe, it } from "node:test";
import { parse } from "yaml";

import { ConfigError, type LoadClientOptions, loadConfig } from "../src/index.js";
import { parseNoProxy, proxyFor } from "../src/proxy.js";
import { readRateLimit } from "../src/ratelimit.js";
import { configPath, parseDuration, resolve as resolveSettings } from "../src/settings.js";

const root = resolve(process.cwd(), "..", "conformance", "vectors");

interface Vector {
  name: string;
  kind: string;
  pending?: string[];
  input?: {
    code?: Record<string, unknown>;
    env?: Record<string, string>;
    config_file?: string;
    home?: boolean;
    files?: Record<string, string>;
    os?: "linux" | "macos" | "windows";
    profile_type?: string;
    cli?: "present" | "absent";
  };
  expect?: Record<string, unknown>;
  checks?: Record<string, unknown>[];
}

function vectors(kind: string): Vector[] {
  const dir = join(root, kind);
  return readdirSync(dir)
    .filter((f) => f.endsWith(".yaml"))
    .sort()
    .map((f) => parse(readFileSync(join(dir, f), "utf8")) as Vector);
}

const slash = (s: string): string => s.replaceAll("\\", "/");

/** `want` is a subset of `got`: objects by key, arrays by position with the same length. */
function subset(want: unknown, got: unknown, at: string): string | undefined {
  if (Array.isArray(want)) {
    if (!Array.isArray(got) || got.length !== want.length) {
      return `${at}: want ${JSON.stringify(want)}, got ${JSON.stringify(got)}`;
    }
    for (let i = 0; i < want.length; i++) {
      const p = subset(want[i], got[i], `${at}[${i}]`);
      if (p !== undefined) {
        return p;
      }
    }
    return undefined;
  }
  if (typeof want === "object" && want !== null) {
    if (typeof got !== "object" || got === null) {
      return `${at}: want ${JSON.stringify(want)}, got ${JSON.stringify(got)}`;
    }
    for (const [k, v] of Object.entries(want)) {
      const p = subset(v, (got as Record<string, unknown>)[k], `${at}.${k}`);
      if (p !== undefined) {
        return p;
      }
    }
    return undefined;
  }
  if (typeof want === "string" && typeof got === "string") {
    return slash(want) === slash(got) ? undefined : `${at}: want ${want}, got ${got}`;
  }
  return want === got
    ? undefined
    : `${at}: want ${JSON.stringify(want)}, got ${JSON.stringify(got)}`;
}

function substitute(v: unknown, dir: string, file: string): unknown {
  if (typeof v === "string") {
    return v.replaceAll("{file}", file).replaceAll("{dir}", dir);
  }
  if (Array.isArray(v)) {
    return v.map((x) => substitute(x, dir, file));
  }
  if (typeof v === "object" && v !== null) {
    return Object.fromEntries(Object.entries(v).map(([k, x]) => [k, substitute(x, dir, file)]));
  }
  return v;
}

/** Code options by catalogue name, as `load` takes them in TypeScript. */
function codeOptions(code: Record<string, unknown>): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  for (const [k, v] of Object.entries(code)) {
    if (k === "http_client") {
      out.fetch = (() => Promise.reject(new Error("not called"))) as typeof fetch;
      continue;
    }
    const name = k.replace(/_([a-z])/g, (_, c: string) => c.toUpperCase());
    const ms = typeof v === "string" ? parseDuration(v) : undefined;
    const duration = /timeout|delay|max$/.test(k) && ms !== undefined;
    out[name] = duration ? ms : v;
  }
  return out;
}

function runConfig(v: Vector): string | undefined {
  const dir = mkdtempSync(join(tmpdir(), "inorbit-vector-"));
  const input = v.input ?? {};
  const env: Record<string, string> = Object.fromEntries(
    Object.entries(input.env ?? {}).map(([k, x]) => [k, x.replaceAll("{dir}", dir)]),
  );
  const os = input.os ?? "linux";
  const home = input.home === true ? join(dir, "home") : null;
  if (home !== null) {
    mkdirSync(home, { recursive: true });
  }
  for (const [name, content] of Object.entries(input.files ?? {})) {
    const p = join(dir, name);
    mkdirSync(dirname(p), { recursive: true });
    writeFileSync(p, content);
  }
  const code = codeOptions(input.code ?? {});
  let file = join(dir, "config.toml");
  if (input.config_file !== undefined) {
    if (input.home === true) {
      const { INORBIT_CONFIG_FILE: _, ...located } = env;
      const at = configPath(os, located, home, undefined);
      assert.ok(at !== undefined, `${v.name}: no default location`);
      mkdirSync(dirname(at.path), { recursive: true });
      writeFileSync(at.path, input.config_file);
      file = at.path;
    } else {
      writeFileSync(file, input.config_file);
      code.configFile = file;
    }
  }
  if (input.cli === "present") {
    const bin = join(dir, "bin");
    mkdirSync(bin, { recursive: true });
    for (const name of process.platform === "win32" ? ["iohr.exe"] : ["iohr"]) {
      writeFileSync(join(bin, name), "#!/bin/sh\nexit 1\n");
      chmodSync(join(bin, name), 0o755);
    }
    env.PATH = bin;
  }
  const options: LoadClientOptions = {
    ...(code as LoadClientOptions),
    ...(input.profile_type === undefined ? {} : { profileType: input.profile_type }),
    loadOptions: { env, os, home, cwd: dir },
  };
  const want = substitute(v.expect ?? {}, dir, file) as Record<string, unknown>;
  let doc: Record<string, unknown> | undefined;
  let error: unknown;
  try {
    doc = loadConfig(options).describe() as unknown as Record<string, unknown>;
  } catch (e) {
    error = e;
  }
  const shown =
    doc !== undefined
      ? JSON.stringify(doc)
      : error instanceof ConfigError
        ? `${error.message} ${JSON.stringify(error.problems)}`
        : String(error);
  for (const x of (want.excludes as string[] | undefined) ?? []) {
    if (shown.includes(x)) {
      return `${JSON.stringify(x)} appears in ${shown}`;
    }
  }
  if (want.error !== undefined) {
    if (!(error instanceof ConfigError)) {
      return `want a ConfigError, got ${shown}`;
    }
    const w = want.error as {
      problems?: { setting?: string; source?: string; message_contains?: string }[];
      message_contains?: string[];
    };
    if (w.problems !== undefined) {
      if (w.problems.length !== error.problems.length) {
        return `want ${w.problems.length} problems, got ${JSON.stringify(error.problems)}`;
      }
      for (const [i, p] of w.problems.entries()) {
        const have = error.problems[i];
        if (have === undefined) {
          return "missing problem";
        }
        if (p.setting !== undefined && p.setting !== have.setting) {
          return `problem ${i}: setting ${p.setting} != ${JSON.stringify(have)}`;
        }
        if (p.source !== undefined && slash(p.source) !== slash(have.source)) {
          return `problem ${i}: source ${p.source} != ${JSON.stringify(have)}`;
        }
        if (p.message_contains !== undefined && !have.message.includes(p.message_contains)) {
          return `problem ${i}: message lacks ${p.message_contains}: ${JSON.stringify(have)}`;
        }
      }
    }
    for (const part of w.message_contains ?? []) {
      if (!error.message.includes(part)) {
        return `the message lacks ${JSON.stringify(part)}:\n${error.message}`;
      }
    }
    return undefined;
  }
  if (doc === undefined) {
    return `unexpected error: ${shown}`;
  }
  for (const key of ["profile", "settings", "credential", "pipeline"]) {
    if (want[key] !== undefined) {
      const p = subset(want[key], doc[key], key);
      if (p !== undefined) {
        return p;
      }
    }
  }
  if ("config_file" in want) {
    const w = want.config_file === null ? null : slash(String(want.config_file));
    const h = doc.config_file === null ? null : slash(String(doc.config_file));
    if (w !== h) {
      return `config_file: want ${w}, got ${h}`;
    }
  }
  const settings = doc.settings as Record<string, unknown>;
  for (const a of (want.settings_absent as string[] | undefined) ?? []) {
    if (a in settings) {
      return `${a} should not be in settings`;
    }
  }
  const ignored = doc.ignored as unknown[];
  for (const w of (want.ignored as unknown[] | undefined) ?? []) {
    if (!ignored.some((h) => subset(w, h, "") === undefined)) {
      return `ignored lacks ${JSON.stringify(w)}: ${JSON.stringify(ignored)}`;
    }
  }
  return undefined;
}

function snapshotWire(s: ReturnType<typeof readRateLimit>): unknown {
  if (s === undefined) {
    return null;
  }
  const out: Record<string, unknown> = {};
  if (s.limit !== undefined) out.limit = s.limit;
  if (s.remaining !== undefined) out.remaining = s.remaining;
  if (s.reset !== undefined) out.reset_ms = s.reset;
  if (s.policy !== undefined) {
    const p: Record<string, unknown> = { name: s.policy.name };
    if (s.policy.quota !== undefined) p.quota = s.policy.quota;
    if (s.policy.window !== undefined) p.window_ms = s.policy.window;
    out.policy = p;
  }
  return out;
}

const pending = (v: Vector): boolean => v.pending?.includes("ts") === true;

describe("conformance vectors", () => {
  it("config", () => {
    const failures: string[] = [];
    for (const v of vectors("config")) {
      if (pending(v)) {
        continue;
      }
      const p = runConfig(v);
      if (p !== undefined) {
        failures.push(`${v.name}: ${p}`);
      }
    }
    assert.deepEqual(failures, []);
  });

  it("config-path", () => {
    for (const v of vectors("config-path").filter((x) => !pending(x))) {
      for (const c of v.checks ?? []) {
        const code = (c.code as { config_file?: string } | undefined)?.config_file;
        const got = configPath(
          (c.os as "linux") ?? "linux",
          (c.env as Record<string, string>) ?? {},
          (c.home as string | null) ?? null,
          code,
        );
        assert.equal(got?.path ?? null, c.expect, `${v.name}: ${c.summary}`);
      }
    }
  });

  it("durations", () => {
    for (const v of vectors("durations").filter((x) => !pending(x))) {
      for (const c of v.checks ?? []) {
        const got = parseDuration(String(c.value));
        assert.equal(got ?? "error", c.expect, `${v.name}: ${JSON.stringify(c.value)}`);
      }
    }
  });

  it("no-proxy", () => {
    for (const v of vectors("no-proxy").filter((x) => !pending(x))) {
      for (const c of v.checks ?? []) {
        const env = {
          ...(c.env as Record<string, string>),
          INORBIT_TOKEN: "t",
          INORBIT_CONFIG_FILE: "off",
        };
        const code = new Map(Object.entries((c.code as Record<string, unknown>) ?? {}));
        const what = `${v.name}: ${c.summary}`;
        let res: ReturnType<typeof resolveSettings>;
        try {
          res = resolveSettings({
            code,
            httpClient: false,
            tokenProvider: false,
            load: { env, os: "linux", home: null, cwd: "/" },
          });
        } catch (e) {
          assert.ok(e instanceof ConfigError, `${what}: ${String(e)}`);
          assert.deepEqual(c.expect, { error: "config" }, `${what}: ${e.message}`);
          continue;
        }
        assert.notDeepEqual(c.expect, { error: "config" }, `${what}: want a ConfigError`);
        const noProxy = parseNoProxy((res.values.get("no_proxy") as string[] | undefined) ?? []);
        const got = proxyFor(new URL(String(c.url)), res.proxy, noProxy) ?? null;
        assert.equal(got, c.expect, what);
      }
    }
  });

  it("rate-limit", () => {
    for (const v of vectors("rate-limit").filter((x) => !pending(x))) {
      for (const c of v.checks ?? []) {
        const got = snapshotWire(readRateLimit(new Headers(c.headers as Record<string, string>)));
        assert.deepEqual(got, c.expect ?? null, `${v.name}: ${c.summary}`);
      }
    }
  });
});
