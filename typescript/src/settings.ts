/**
 * Configuration resolution (config.md sections 2 to 5): each setting from code, the
 * environment, the config file or its default, the credential chain, and the
 * `describe()` document. Resolution reads files and the environment and contacts no host.
 *
 * @module
 */

import { parse as parseToml, TomlDate } from "smol-toml";
import { ConfigError, type ConfigProblem } from "./errors.js";
import {
  currentOs,
  fs,
  homeDir,
  type Os,
  proc,
  processEnv,
  type StatLike,
  workingDir,
} from "./platform.js";
import { type ProxyChoice, parseNoProxyEntry } from "./proxy.js";

/** The built-in pipeline, outermost first (config.md section 7.2). */
export const BUILT_INS: readonly string[] = [
  "request_id",
  "user_agent",
  "idempotency_key",
  "call_tracing",
  "deadline",
  "retry",
  "auth",
  "rate_limit",
  "attempt_tracing",
  "logging",
  "hooks",
  "timeout",
];

const MAX_FILE = 1024 * 1024;
const REDACTED = "<redacted>";
const CLI_KEYS = ["kind", "account", "storage", "issuer", "client_id"];
const SOURCES = ["env", "workload", "file", "cli"] as const;

/** A credential source the chain may use (config.md section 5.1). */
export type CredentialSource = (typeof SOURCES)[number];

/** What resolution reads instead of the process: for tests, and for your own (config.md section 9.2). */
export interface LoadOptions {
  /** The environment (default: the process's). An empty value is unset. */
  readonly env?: Readonly<Record<string, string | undefined>>;
  /** Whose file locations apply (default: this machine's). */
  readonly os?: Os;
  /** The home directory; `null` for none (default: this machine's). */
  readonly home?: string | null;
  /** The working directory relative paths in the environment resolve against. */
  readonly cwd?: string;
}

type Ty =
  | "duration"
  | "int"
  | "bool"
  | "scopes"
  | "list"
  | "url"
  | "path"
  | "secret"
  | "str"
  | "proxy"
  | "pins"
  | "reserved"
  | { readonly oneOf: readonly string[] };

interface Setting {
  readonly name: string;
  readonly ty: Ty;
  /** Whether the key may appear in the config file. */
  readonly file: boolean;
  /** A credential piece: read only with a typed profile's prefix, chosen whole. */
  readonly credential: boolean;
  /** Belongs to a caller-supplied HTTP client (section 6.6). */
  readonly transport: boolean;
  /** The default, as `describe()` shows it. */
  readonly fallback?: unknown;
}

const s = (name: string, ty: Ty, fallback?: unknown): Setting => ({
  name,
  ty,
  file: true,
  credential: false,
  transport: false,
  ...(fallback === undefined ? {} : { fallback }),
});
const cred = (name: string, ty: Ty, file: boolean): Setting => ({
  name,
  ty,
  file,
  credential: true,
  transport: false,
});
const net = (name: string, ty: Ty, fallback?: unknown): Setting => ({
  ...s(name, ty, fallback),
  transport: true,
});

/** The catalogue (section 3), in its order; problems are reported in this order. */
const CATALOGUE: readonly Setting[] = [
  s("base_url", "url", "https://api.inorbit.hr"),
  s("token_url", "url", "https://auth.inorbit.hr/oauth2/token"),
  s("region", "reserved"),
  cred("key_id", "str", true),
  cred("key_secret", "secret", false),
  cred("key_secret_file", "path", true),
  s("scopes", "scopes"),
  cred("token", "secret", false),
  cred("token_file", "path", true),
  s("credential_sources", "list", [...SOURCES]),
  s("cli_path", "path"),
  net("connect_timeout", "duration", "10s"),
  s("timeout", "duration", "30s"),
  s("total_timeout", "duration", "120s"),
  s("stream_idle_timeout", "duration", "45s"),
  s("max_retries", "int", 2),
  s("retry_base_delay", "duration", "500ms"),
  s("retry_max_delay", "duration", "8s"),
  s("retry_after_max", "duration", "60s"),
  s("retry_budget", "bool", true),
  s("streams", { oneOf: ["sse", "socket"] }, "sse"),
  net("proxy", "proxy"),
  net("no_proxy", "list"),
  net("ca_bundle", "path"),
  net("system_trust", "bool", true),
  net("client_cert", "path"),
  net("client_key", "path"),
  { ...net("client_key_password", "secret"), file: false },
  net("pinned_keys", "pins"),
  s("log", { oneOf: ["off", "error", "warn", "info", "debug"] }, "off"),
  s("log_headers", "bool", false),
  s("log_allow_headers", "list"),
  s("tracing", "bool"),
  s("metrics", "bool"),
  s("rate_limit", { oneOf: ["observe", "wait", "off"] }, "observe"),
  s("user_agent_suffix", "str"),
];

/** Every setting's catalogue name, in the catalogue's order. */
export const SETTING_NAMES: readonly string[] = CATALOGUE.map((x) => x.name);

function order(setting: string): number {
  if (setting === "profile") {
    return 0;
  }
  if (setting === "config_file") {
    return 1;
  }
  if (setting === "credential") {
    return Number.MAX_SAFE_INTEGER;
  }
  const i = CATALOGUE.findIndex((x) => x.name === setting);
  return i < 0 ? Number.MAX_SAFE_INTEGER - 1 : i + 2;
}

/** A catalogue name in TypeScript's case (`total_timeout` → `totalTimeout`). */
export function camel(name: string): string {
  return name.replace(/_([a-z])/g, (_, c: string) => c.toUpperCase());
}

/** Parses a duration: digits, then `ms`, `s`, `m` or `h`, greater than zero; milliseconds. */
export function parseDuration(v: string): number | undefined {
  const m = /^(\d+)(ms|s|m|h)$/.exec(v);
  if (m === null) {
    return undefined;
  }
  const n = Number(m[1]);
  const ms = n * ({ ms: 1, s: 1000, m: 60_000, h: 3_600_000 } as const)[m[2] as "ms"];
  return Number.isSafeInteger(ms) && ms > 0 ? ms : undefined;
}

/** A duration as `describe()` shows it: `30s`, or `1500ms`. */
export function showDuration(ms: number): string {
  return ms % 1000 === 0 ? `${ms / 1000}s` : `${ms}ms`;
}

function redactUserinfo(raw: string): string {
  const i = raw.indexOf("://");
  if (i < 0) {
    return raw;
  }
  const rest = raw.slice(i + 3);
  const end = rest.search(/[/?#]/);
  const authority = end < 0 ? rest : rest.slice(0, end);
  const at = authority.lastIndexOf("@");
  return at < 0 ? raw : `${raw.slice(0, i)}://${REDACTED}@${rest.slice(at + 1)}`;
}

function hasUserinfo(raw: string): boolean {
  try {
    const u = new URL(raw);
    return u.username !== "" || u.password !== "";
  } catch {
    return false;
  }
}

function isLoopbackHost(host: string): boolean {
  const h = host.replace(/^\[|\]$/g, "").toLowerCase();
  return h === "localhost" || h === "::1" || /^127\.\d+\.\d+\.\d+$/.test(h);
}

/** Path rules for one OS: absolute, join, parent. */
class Paths {
  constructor(readonly os: Os) {}

  get sep(): string {
    return this.os === "windows" ? "\\" : "/";
  }

  isAbsolute(p: string): boolean {
    if (p.startsWith("/")) {
      return true;
    }
    if (this.os === "windows" || /^[A-Za-z]:[\\/]/.test(p)) {
      return p.startsWith("\\") || /^[A-Za-z]:[\\/]/.test(p);
    }
    return false;
  }

  join(dir: string, rest: string): string {
    return `${dir.replace(/[/\\]+$/, "")}${this.sep}${rest}`;
  }

  parent(path: string): string {
    const i = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
    const p = i === 0 ? path.slice(0, 1) : i < 0 ? "." : path.slice(0, i);
    return this.os === "windows" ? p.replaceAll("/", "\\") : p.replaceAll("\\", "/");
  }
}

/**
 * Where the config file is (config.md section 4.1): the path, whether it was named (code
 * or `INORBIT_CONFIG_FILE`) and where that came from; `undefined` reads no file.
 */
export function configPath(
  os: Os,
  env: Readonly<Record<string, string | undefined>>,
  home: string | null,
  code: string | undefined,
): { readonly path: string; readonly named: boolean; readonly label: string } | undefined {
  const paths = new Paths(os);
  const v = (k: string): string | undefined => {
    const x = env[k];
    return x === undefined || x === "" ? undefined : x;
  };
  if (code !== undefined) {
    return code === "off" ? undefined : { path: code, named: true, label: "code" };
  }
  const named = v("INORBIT_CONFIG_FILE");
  if (named !== undefined) {
    return named === "off"
      ? undefined
      : { path: named, named: true, label: "env INORBIT_CONFIG_FILE" };
  }
  const dir = v("IOHR_CONFIG_DIR");
  if (dir !== undefined) {
    return { path: paths.join(dir, "config.toml"), named: false, label: "env IOHR_CONFIG_DIR" };
  }
  const found = (path: string) => ({ path, named: false, label: "default" });
  switch (os) {
    case "macos":
      return home === null
        ? undefined
        : found(paths.join(home, "Library/Application Support/hr.InOrbit.iohr/config.toml"));
    case "windows": {
      const appdata = v("APPDATA");
      return appdata === undefined
        ? undefined
        : found(paths.join(appdata, "InOrbit\\iohr\\config\\config.toml"));
    }
    default: {
      const xdg = v("XDG_CONFIG_HOME");
      if (xdg !== undefined && paths.isAbsolute(xdg)) {
        return found(paths.join(xdg, "iohr/config.toml"));
      }
      return home === null ? undefined : found(paths.join(home, ".config/iohr/config.toml"));
    }
  }
}

/** A profile name the command line accepts. */
function validProfile(name: string): boolean {
  return /^[a-z0-9][a-z0-9_-]{0,63}$/.test(name);
}

/** A profile name in the environment's form: `acme-ci` → `ACME_CI`. */
export function envName(profile: string): string {
  return profile.toUpperCase().replaceAll("-", "_");
}

type TomlTable = Record<string, unknown>;

function tomlType(v: unknown): string {
  if (typeof v === "string") {
    return "string";
  }
  if (typeof v === "number" || typeof v === "bigint") {
    return Number.isInteger(v) || typeof v === "bigint" ? "integer" : "float";
  }
  if (typeof v === "boolean") {
    return "boolean";
  }
  if (v instanceof TomlDate) {
    return "datetime";
  }
  return Array.isArray(v) ? "array" : "table";
}

function isTable(v: unknown): v is TomlTable {
  return typeof v === "object" && v !== null && !Array.isArray(v) && !(v instanceof TomlDate);
}

type Raw =
  | { readonly from: "code"; readonly value: unknown }
  | { readonly from: "env"; readonly value: string }
  | { readonly from: "file"; readonly value: unknown };

interface Layer {
  readonly table: TomlTable;
  readonly label: string;
  /** The profile's own table, where the command line's keys are not SDK keys. */
  readonly profile: boolean;
}

/** One setting as `describe()` shows it. */
export interface DescribedSetting {
  readonly value: unknown;
  readonly source: string;
}

/** One source of the credential chain, as `describe()` lists it. */
export interface TriedSource {
  readonly source: string;
  readonly result: "used" | "skipped";
  readonly reason?: string;
}

/** The `describe()` document (config.md section 2.6), stable within a major version. */
export interface Description {
  readonly profile: { readonly name: string; readonly source: string } | null;
  readonly config_file: string | null;
  readonly settings: Readonly<Record<string, DescribedSetting>>;
  readonly credential: {
    readonly source: string;
    readonly kind: string;
    readonly tried: readonly TriedSource[];
  } | null;
  readonly pipeline: readonly string[];
  readonly ignored: readonly {
    readonly key: string;
    readonly source: string;
    readonly reason: string;
  }[];
}

/** What the chain decided: the credential's shape and where its pieces are. */
export type CredentialPlan =
  | { readonly source: string; readonly kind: "custom" }
  | { readonly source: string; readonly kind: "static_token"; readonly token: string }
  | { readonly source: string; readonly kind: "token_file"; readonly path: string }
  | {
      readonly source: string;
      readonly kind: "client_credentials";
      readonly keyId: string;
      readonly keySecret?: string;
      readonly keySecretFile?: string;
    }
  | {
      readonly source: string;
      readonly kind: "cli";
      readonly profile: string;
      readonly program: string;
    };

/** The effective values, typed (durations in milliseconds), by catalogue name. */
export type Values = ReadonlyMap<string, unknown>;

/** A resolved configuration: the values a client is built from, and its description. */
export interface Resolution {
  readonly values: Values;
  readonly description: Description;
  readonly credential: CredentialPlan;
  readonly profile: string | undefined;
  /** The proxy, with whether it was set on purpose (section 6.2). */
  readonly proxy: ProxyChoice | undefined;
}

/** What `resolve` is handed. */
export interface ResolveInput {
  /** Options set in code, by catalogue name; durations in milliseconds. */
  readonly code: ReadonlyMap<string, unknown>;
  /** Whether code supplied the HTTP client (`fetch` or a dispatcher). */
  readonly httpClient: boolean;
  /** Whether code supplied a token provider. */
  readonly tokenProvider: boolean;
  /** Resolve for a typed (generated) profile of this name. */
  readonly profileType?: string | undefined;
  /** Explicit construction: code and defaults only, no environment, file or chain. */
  readonly explicit?: boolean;
  readonly load?: LoadOptions | undefined;
}

function fileText(path: string): { bytes?: Uint8Array; stat?: StatLike } | undefined {
  const f = fs();
  if (f === undefined) {
    return undefined;
  }
  try {
    const stat = f.statSync(path, { throwIfNoEntry: false });
    if (stat === undefined || !stat.isFile()) {
      return undefined;
    }
    if (stat.size > MAX_FILE) {
      return { stat };
    }
    return { bytes: f.readFileSync(path), stat };
  } catch {
    return undefined;
  }
}

function readable(path: string): boolean {
  const f = fs();
  if (f === undefined) {
    return false;
  }
  try {
    f.readFileSync(path);
    return true;
  } catch {
    return false;
  }
}

/**
 * Whether `program` can be run: a path to a file, or a name found on `PATH`. `PATH` is
 * read with this machine's rules (separator, Windows extensions), whatever OS the
 * resolution follows.
 */
function programFound(program: string, env: Readonly<Record<string, string | undefined>>): boolean {
  const f = fs();
  if (f === undefined) {
    return false;
  }
  const isFile = (p: string): boolean => {
    try {
      return f.statSync(p, { throwIfNoEntry: false })?.isFile() ?? false;
    } catch {
      return false;
    }
  };
  if (/[/\\]/.test(program)) {
    return isFile(program);
  }
  const windows = proc()?.platform === "win32";
  const path = env.PATH ?? env.Path ?? "";
  if (path === "") {
    return false;
  }
  const exts = windows ? ["", ".exe", ".cmd", ".bat"] : [""];
  const sep = windows ? "\\" : "/";
  return path
    .split(windows ? ";" : ":")
    .filter((d) => d !== "")
    .some((dir) =>
      exts.some((ext) => isFile(`${dir.replace(/[/\\]+$/, "")}${sep}${program}${ext}`)),
    );
}

/** Resolves a configuration, or throws a {@link ConfigError} listing every problem. */
export function resolve(input: ResolveInput): Resolution {
  return new Resolver(input).run();
}

class Resolver {
  readonly #in: ResolveInput;
  readonly #env: Readonly<Record<string, string | undefined>>;
  readonly #os: Os;
  readonly #paths: Paths;
  readonly #home: string | null;
  readonly #cwd: string;
  readonly #prefix: string | undefined;
  readonly #problems: ConfigProblem[] = [];
  readonly #settings: Record<string, DescribedSetting> = {};
  readonly #values = new Map<string, unknown>();
  readonly #ignored: { key: string; source: string; reason: string }[] = [];
  #layers: Layer[] = [];
  #filePath: string | null = null;
  #fileDir = ".";
  #noFs = false;
  #proxy: ProxyChoice | undefined;

  constructor(input: ResolveInput) {
    this.#in = input;
    const load = input.load ?? {};
    this.#env = input.explicit === true ? {} : (load.env ?? processEnv());
    this.#os = load.os ?? currentOs();
    this.#paths = new Paths(this.#os);
    this.#home = load.home === undefined ? homeDir() : load.home;
    this.#cwd = load.cwd ?? workingDir();
    this.#prefix =
      input.profileType === undefined ? undefined : `INORBIT_${envName(input.profileType)}_`;
  }

  #var(k: string): string | undefined {
    const v = this.#env[k];
    return v === undefined || v === "" ? undefined : v;
  }

  #problem(setting: string, source: string, message: string): void {
    this.#problems.push({ setting, source, message });
  }

  #label(suffix: string): string {
    if (this.#filePath === null) {
      return "";
    }
    return suffix === "" ? `file ${this.#filePath}` : `file ${this.#filePath} [${suffix}]`;
  }

  run(): Resolution {
    const explicit = this.#in.explicit === true;
    let doc: TomlTable = {};
    if (!explicit) {
      doc = this.#readFile();
    }
    let profile: { name: string; source: string } | undefined;
    if (this.#in.profileType !== undefined) {
      profile = { name: this.#in.profileType, source: "code" };
    } else if (!explicit) {
      profile = this.#chooseProfile(doc);
    }
    const profiles = isTable(doc.profiles) ? doc.profiles : {};
    const table =
      profile !== undefined && isTable(profiles[profile.name])
        ? (profiles[profile.name] as TomlTable)
        : undefined;
    if (table !== undefined && profile !== undefined) {
      this.#layers.push({ table, label: this.#label(`profiles.${profile.name}`), profile: true });
    }
    if (isTable(doc.sdk)) {
      this.#layers.push({ table: doc.sdk, label: this.#label("sdk"), profile: false });
    }
    this.#fileDir = this.#filePath === null ? this.#cwd : this.#paths.parent(this.#filePath);
    this.#checkFileKeys();
    this.#resolveSettings();
    const credential = explicit ? this.#explicitCredential() : this.#chain(profile?.name, table);
    this.#crossChecks();
    if (this.#problems.length > 0) {
      const problems = [...this.#problems].sort((a, b) => order(a.setting) - order(b.setting));
      throw ConfigError.of(problems);
    }
    if (credential === undefined) {
      throw new ConfigError("no credentials");
    }
    return {
      values: this.#values,
      description: {
        profile: profile === undefined ? null : { name: profile.name, source: profile.source },
        config_file: this.#filePath,
        settings: this.#settings,
        credential: credential.described,
        pipeline: BUILT_INS,
        ignored: this.#ignored,
      },
      credential: credential.plan,
      profile: profile?.name,
      proxy: this.#proxy,
    };
  }

  #readFile(): TomlTable {
    const code = this.#in.code.get("config_file");
    if (fs() === undefined) {
      this.#noFs = true;
      return {};
    }
    const located = configPath(
      this.#os,
      this.#env,
      this.#home,
      typeof code === "string" ? code : undefined,
    );
    if (located === undefined) {
      return {};
    }
    const abs = this.#paths.isAbsolute(located.path)
      ? located.path
      : this.#paths.join(this.#cwd, located.path);
    const read = fileText(abs);
    if (read === undefined) {
      if (located.named) {
        this.#problem("config_file", located.label, `there is no readable file at ${abs}`);
      }
      return {};
    }
    if (read.bytes === undefined) {
      this.#problem("config_file", located.label, `${abs} is larger than 1 MiB`);
      return {};
    }
    let text: string;
    try {
      text = new TextDecoder("utf-8", { fatal: true }).decode(read.bytes);
    } catch {
      this.#problem("config_file", located.label, `${abs} is not valid TOML: it is not UTF-8`);
      return {};
    }
    try {
      const parsed = parseToml(text) as TomlTable;
      this.#filePath = abs;
      return parsed;
    } catch (e) {
      // The first line only: the parser's excerpt could quote a secret.
      const first = (e instanceof Error ? e.message : String(e)).split("\n")[0] ?? "";
      this.#problem("config_file", located.label, `${abs} is not valid TOML: ${first}`);
      return {};
    }
  }

  #chooseProfile(doc: TomlTable): { name: string; source: string } | undefined {
    const code = this.#in.code.get("profile");
    let chosen: { name: string; source: string } | undefined;
    if (typeof code === "string" && code !== "") {
      chosen = { name: code, source: "code" };
    } else if (this.#var("INORBIT_PROFILE") !== undefined) {
      chosen = { name: this.#var("INORBIT_PROFILE") ?? "", source: "env INORBIT_PROFILE" };
    } else if (typeof doc.default === "string") {
      chosen = { name: doc.default, source: this.#label("") };
    }
    if (chosen === undefined) {
      return undefined;
    }
    const profiles = isTable(doc.profiles) ? doc.profiles : {};
    if (!validProfile(chosen.name)) {
      this.#problem(
        "profile",
        chosen.source,
        `${JSON.stringify(chosen.name)} is not a profile name: 1 to 64 lower-case letters, digits, '-' or '_', starting with a letter or digit`,
      );
      return undefined;
    }
    if (!Object.hasOwn(profiles, chosen.name)) {
      const where =
        this.#filePath === null
          ? "no config file was read"
          : `${this.#filePath} has no [profiles.${chosen.name}]`;
      this.#problem(
        "profile",
        chosen.source,
        `there is no profile ${JSON.stringify(chosen.name)}: ${where}; \`iohr profile list\` shows the profiles`,
      );
      return undefined;
    }
    return chosen;
  }

  #checkFileKeys(): void {
    for (const layer of this.#layers) {
      for (const [k, v] of Object.entries(layer.table)) {
        if (layer.profile && CLI_KEYS.includes(k)) {
          continue;
        }
        const setting = CATALOGUE.find((x) => x.name === k);
        if (setting === undefined) {
          const reason =
            k === "profile" || k === "config_file"
              ? "not read from the config file"
              : "unknown key";
          this.#ignored.push({ key: k, source: layer.label, reason });
        } else if (!setting.file) {
          const wayOut =
            k === "key_secret"
              ? "key_secret_file, the environment, or iohr login"
              : k === "token"
                ? "token_file, the environment, or iohr login"
                : "the environment or code";
          this.#problem(
            k,
            layer.label,
            `secrets are not allowed in the config file; use ${wayOut}`,
          );
        } else if (setting.ty === "proxy" && typeof v === "string" && hasUserinfo(v)) {
          this.#problem(
            k,
            layer.label,
            "a proxy URL with a user name or password holds a secret, which is not allowed in the config file; set it in INORBIT_PROXY or in code",
          );
        }
      }
    }
  }

  #envNames(setting: Setting): string[] {
    const upper = setting.name.toUpperCase();
    const names: string[] = [];
    if (this.#prefix !== undefined) {
      names.push(`${this.#prefix}${upper}`);
    }
    if (!(setting.credential && this.#prefix !== undefined)) {
      names.push(`INORBIT_${upper}`);
    }
    return names;
  }

  #raw(setting: Setting): { raw: Raw; source: string } | undefined {
    if (this.#in.code.has(setting.name)) {
      return { raw: { from: "code", value: this.#in.code.get(setting.name) }, source: "code" };
    }
    for (const n of this.#envNames(setting)) {
      const v = this.#var(n);
      if (v !== undefined) {
        return { raw: { from: "env", value: v }, source: `env ${n}` };
      }
    }
    if (setting.file) {
      for (const l of this.#layers) {
        if (Object.hasOwn(l.table, setting.name)) {
          return { raw: { from: "file", value: l.table[setting.name] }, source: l.label };
        }
      }
    }
    const after =
      setting.name === "proxy"
        ? ["https_proxy", "HTTPS_PROXY"]
        : setting.name === "no_proxy"
          ? ["no_proxy", "NO_PROXY"]
          : [];
    for (const n of after) {
      const v = this.#var(n);
      if (v !== undefined) {
        return { raw: { from: "env", value: v }, source: `env ${n}` };
      }
    }
    return undefined;
  }

  #set(name: string, shown: unknown, value: unknown, source: string): void {
    this.#settings[name] = { value: shown, source };
    this.#values.set(name, value);
  }

  #resolveSettings(): void {
    for (const setting of CATALOGUE) {
      if (setting.credential) {
        continue;
      }
      const found = this.#raw(setting);
      if (found === undefined) {
        if (setting.fallback !== undefined && !(setting.transport && this.#in.httpClient)) {
          this.#set(setting.name, setting.fallback, defaultValue(setting), "default");
        }
        continue;
      }
      if (setting.transport && this.#in.httpClient) {
        if (found.source === "code") {
          this.#problem(
            setting.name,
            "code",
            "configure this on your HTTP client (fetch or dispatcher), or leave http_client out",
          );
        } else {
          this.#ignored.push({
            key: setting.name,
            source: found.source,
            reason: "the caller's HTTP client decides this",
          });
        }
        continue;
      }
      try {
        const [shown, value] = this.#parse(setting, found.raw);
        this.#set(setting.name, shown, value, found.source);
        if (setting.name === "proxy") {
          this.#proxy = {
            url: value as string,
            explicit: !found.source.startsWith("env ") || found.source === "env INORBIT_PROXY",
          };
        }
      } catch (e) {
        this.#problem(setting.name, found.source, (e as Error).message);
      }
    }
  }

  #path(p: string, fromFile: boolean): string {
    if (p.startsWith("~/")) {
      if (this.#home === null) {
        throw new Error(`${p} starts with ~/ but there is no home directory`);
      }
      return this.#paths.join(this.#home, p.slice(2));
    }
    if (this.#paths.isAbsolute(p)) {
      return p;
    }
    return this.#paths.join(fromFile ? this.#fileDir : this.#cwd, p);
  }

  /** The value as `describe()` shows it, and as the client uses it. */
  #parse(setting: Setting, raw: Raw): [unknown, unknown] {
    const fromFile = raw.from === "file";
    const text = (what: string): string => {
      if (typeof raw.value === "string") {
        return raw.value;
      }
      throw new Error(
        raw.from === "file" ? `must be ${what}, not ${tomlType(raw.value)}` : `must be ${what}`,
      );
    };
    const list = (comma: boolean): string[] => {
      if (raw.from === "env") {
        return comma
          ? raw.value
              .split(",")
              .map((x) => x.trim())
              .filter((x) => x !== "")
          : raw.value.split(/\s+/).filter((x) => x !== "");
      }
      if (raw.from === "code" && typeof raw.value === "string") {
        return comma
          ? raw.value
              .split(",")
              .map((x) => x.trim())
              .filter((x) => x !== "")
          : raw.value.split(/\s+/).filter((x) => x !== "");
      }
      if (Array.isArray(raw.value)) {
        if (raw.value.every((x) => typeof x === "string")) {
          return [...(raw.value as string[])];
        }
        throw new Error(
          raw.from === "file" ? "must be an array of strings" : "must be a list of strings",
        );
      }
      throw new Error(
        raw.from === "file"
          ? `must be an array of strings, not ${tomlType(raw.value)}`
          : "must be a list of strings",
      );
    };
    const ty = setting.ty;
    if (typeof ty === "object") {
      const v = text("a string");
      if (!ty.oneOf.includes(v)) {
        throw new Error(`${JSON.stringify(v)} is not one of ${ty.oneOf.join(", ")}`);
      }
      return [v, v];
    }
    switch (ty) {
      case "duration": {
        if (raw.from === "code") {
          const n = raw.value;
          if (typeof n !== "number" || !Number.isFinite(n) || n <= 0) {
            throw new Error("must be a number of milliseconds greater than zero");
          }
          return [showDuration(Math.ceil(n)), Math.ceil(n)];
        }
        const v = text('a duration string such as "30s"');
        const ms = parseDuration(v);
        if (ms === undefined) {
          throw new Error(
            parseDuration(`${v}s`) !== undefined
              ? `${JSON.stringify(v)} is not a duration; write it with a unit, such as 30s`
              : `${JSON.stringify(v)} is not a duration greater than zero: digits, then ms, s, m or h, such as 30s`,
          );
        }
        return [showDuration(ms), ms];
      }
      case "int": {
        const v = raw.value;
        if (raw.from === "env") {
          if (!/^\d+$/.test(raw.value) || Number(raw.value) > 0xffffffff) {
            throw new Error(`${JSON.stringify(raw.value)} is not a whole number of 0 or more`);
          }
          return [Number(raw.value), Number(raw.value)];
        }
        if (typeof v === "number" && Number.isInteger(v) && v >= 0 && v <= 0xffffffff) {
          return [v, v];
        }
        if (raw.from === "file" && typeof v !== "number") {
          throw new Error(`must be an integer, not ${tomlType(v)}`);
        }
        throw new Error("must be a whole number of 0 or more");
      }
      case "bool": {
        if (raw.from === "env") {
          const v = raw.value.toLowerCase();
          if (v === "true" || v === "1") {
            return [true, true];
          }
          if (v === "false" || v === "0") {
            return [false, false];
          }
          throw new Error(`${JSON.stringify(raw.value)} is not true, false, 1 or 0`);
        }
        if (typeof raw.value === "boolean") {
          return [raw.value, raw.value];
        }
        throw new Error(
          raw.from === "file"
            ? `must be a boolean, not ${tomlType(raw.value)}`
            : "must be a boolean",
        );
      }
      case "scopes": {
        const l = list(false);
        return [l, l];
      }
      case "list": {
        const l = list(true);
        if (setting.name === "credential_sources") {
          const bad = l.find((x) => !(SOURCES as readonly string[]).includes(x));
          if (bad !== undefined) {
            throw new Error(
              `${JSON.stringify(bad)} is not a credential source; use env, workload, file or cli`,
            );
          }
        }
        if (setting.name === "no_proxy") {
          const bad = l.find((x) => parseNoProxyEntry(x) === undefined);
          if (bad !== undefined) {
            throw new Error(
              `${JSON.stringify(bad)} is not a no_proxy entry: a host, .domain, host:port, an IP address or a CIDR range`,
            );
          }
        }
        if (setting.name === "log_allow_headers") {
          const lower = l.map((x) => x.toLowerCase());
          return [lower, lower];
        }
        return [l, l];
      }
      case "url": {
        const v = text("a URL string");
        let u: URL;
        try {
          u = new URL(v);
        } catch {
          throw new Error(`${JSON.stringify(v)} is not an absolute URL`);
        }
        if (!(u.protocol === "https:" || (u.protocol === "http:" && isLoopbackHost(u.hostname)))) {
          throw new Error(
            `${JSON.stringify(v)} must use https (plain http is allowed only for localhost and loopback addresses)`,
          );
        }
        if (u.username !== "" || u.password !== "" || u.hash !== "") {
          throw new Error(`${JSON.stringify(v)} must not carry credentials or a fragment`);
        }
        if (setting.name === "base_url" && (u.pathname !== "/" || u.search !== "")) {
          throw new Error(`${JSON.stringify(v)} is an origin only, such as https://api.inorbit.hr`);
        }
        return [v, v];
      }
      case "path": {
        const p = this.#path(text("a path string"), fromFile);
        return [p, p];
      }
      case "secret": {
        if (typeof raw.value === "string") {
          return [REDACTED, raw.value];
        }
        throw new Error("must be a string");
      }
      case "str": {
        const v = text("a string");
        if (
          setting.name === "user_agent_suffix" &&
          ([...v].length > 128 || /[^\x20-\x7e]/.test(v))
        ) {
          throw new Error(
            "must be product tokens (such as myapp/1.2), at most 128 printable ASCII characters",
          );
        }
        return [v, v];
      }
      case "proxy": {
        const v = text("a URL string");
        if (v === "off") {
          return ["off", "off"];
        }
        const shown = redactUserinfo(v);
        let u: URL;
        try {
          u = new URL(v);
        } catch {
          throw new Error(`${JSON.stringify(shown)} is not an absolute URL`);
        }
        if (u.protocol !== "http:" && u.protocol !== "https:") {
          throw new Error(
            `${JSON.stringify(shown)} must be an http:// or https:// proxy URL, or off`,
          );
        }
        return [shown, v];
      }
      case "pins": {
        const l = list(true);
        if (l.length < 2) {
          throw new Error("pin at least two keys (the current one and a backup)");
        }
        for (const p of l) {
          let ok = false;
          try {
            ok = /^[A-Za-z0-9+/]+={0,2}$/.test(p) && atob(p).length === 32;
          } catch {
            ok = false;
          }
          if (!ok) {
            throw new Error(`${JSON.stringify(p)} is not a base64 SHA-256 of a public key`);
          }
        }
        return [l, l];
      }
      default:
        throw new Error("region is reserved until the API offers regions; remove it");
    }
  }

  #allowed(source: CredentialSource): boolean {
    const l = this.#values.get("credential_sources");
    return !Array.isArray(l) || l.includes(source);
  }

  #show(name: string, value: unknown, source: string): void {
    this.#settings[name] = { value, source };
  }

  #explicitCredential(): { plan: CredentialPlan; described: Description["credential"] } {
    const c = this.#in.code;
    const str = (k: string): string | undefined => {
      const v = c.get(k);
      return typeof v === "string" && v !== "" ? v : undefined;
    };
    let plan: CredentialPlan;
    if (this.#in.tokenProvider) {
      plan = { source: "code", kind: "custom" };
    } else if (str("token") !== undefined) {
      plan = { source: "code", kind: "static_token", token: str("token") ?? "" };
      this.#show("token", REDACTED, "code");
    } else if (str("token_file") !== undefined) {
      plan = { source: "code", kind: "token_file", path: str("token_file") ?? "" };
      this.#show("token_file", str("token_file"), "code");
    } else {
      const keyId = String(c.get("key_id") ?? "");
      this.#show("key_id", keyId, "code");
      if (str("key_secret_file") !== undefined && c.get("key_secret") === undefined) {
        plan = {
          source: "code",
          kind: "client_credentials",
          keyId,
          keySecretFile: str("key_secret_file") ?? "",
        };
        this.#show("key_secret_file", str("key_secret_file"), "code");
      } else {
        plan = {
          source: "code",
          kind: "client_credentials",
          keyId,
          keySecret: String(c.get("key_secret") ?? ""),
        };
        this.#show("key_secret", REDACTED, "code");
      }
    }
    this.#scopesFor(plan.kind);
    return {
      plan,
      described: { source: "code", kind: plan.kind, tried: [{ source: "code", result: "used" }] },
    };
  }

  /** The chain (section 5.1); `undefined` when a problem stopped it. */
  #chain(
    profile: string | undefined,
    table: TomlTable | undefined,
  ): { plan: CredentialPlan; described: Description["credential"] } | undefined {
    const tried: TriedSource[] = [];
    const skip = (source: string, reason: string): void => {
      tried.push({ source, result: "skipped", reason });
    };
    const p = this.#prefix ?? "INORBIT_";
    let plan: CredentialPlan | undefined;

    // 1. Code.
    const c = this.#in.code;
    const codeString = (k: string): string | undefined => {
      const v = c.get(k);
      return typeof v === "string" && v !== "" ? v : undefined;
    };
    if (this.#in.tokenProvider) {
      plan = { source: "code", kind: "custom" };
    } else if (codeString("token") !== undefined) {
      plan = { source: "code", kind: "static_token", token: codeString("token") ?? "" };
      this.#show("token", REDACTED, "code");
    } else if (codeString("token_file") !== undefined) {
      const path = this.#path(codeString("token_file") ?? "", false);
      if (!readable(path)) {
        this.#problem("token_file", "code", `cannot read ${path}`);
        return undefined;
      }
      plan = { source: "code", kind: "token_file", path };
      this.#show("token_file", path, "code");
    } else if (codeString("key_id") !== undefined) {
      const keyId = codeString("key_id") ?? "";
      const secret = codeString("key_secret");
      const secretFile = codeString("key_secret_file");
      if (secret !== undefined && secretFile !== undefined) {
        this.#problem("key_secret", "code", "keySecret and keySecretFile are both set; set one");
        return undefined;
      }
      if (secret === undefined && secretFile === undefined) {
        this.#problem("key_secret", "code", "keyId is set without keySecret or keySecretFile");
        return undefined;
      }
      if (!this.#values.has("scopes")) {
        this.#problem("scopes", "code", "a key needs scopes: set scopes");
        return undefined;
      }
      this.#show("key_id", keyId, "code");
      if (secret !== undefined) {
        plan = { source: "code", kind: "client_credentials", keyId, keySecret: secret };
        this.#show("key_secret", REDACTED, "code");
      } else {
        const path = this.#path(secretFile ?? "", false);
        if (!readable(path)) {
          this.#problem("key_secret_file", "code", `cannot read ${path}`);
          return undefined;
        }
        plan = { source: "code", kind: "client_credentials", keyId, keySecretFile: path };
        this.#show("key_secret_file", path, "code");
      }
    }
    if (plan !== undefined) {
      tried.push({ source: "code", result: "used" });
    } else {
      skip("code", "none set");
    }

    // 2. The environment.
    if (plan === undefined) {
      const n = (x: string): string => `${p}${x}`;
      const src = (x: string): string => `env ${p}${x}`;
      const token = this.#var(n("TOKEN"));
      const tokenFile = this.#var(n("TOKEN_FILE"));
      const keyId = this.#var(n("KEY_ID"));
      const secret = this.#var(n("KEY_SECRET"));
      const secretFile = this.#var(n("KEY_SECRET_FILE"));
      if (!this.#allowed("env")) {
        skip("env", "not in credential_sources");
      } else if (token === undefined && tokenFile === undefined && keyId === undefined) {
        skip("env", `${n("TOKEN")}, ${n("TOKEN_FILE")} and ${n("KEY_ID")} are not set`);
      } else {
        const set = (
          [
            ["TOKEN", token],
            ["TOKEN_FILE", tokenFile],
            ["KEY_ID", keyId],
          ] as const
        )
          .filter(([, v]) => v !== undefined)
          .map(([k]) => k);
        if (set.length > 1) {
          this.#problem(
            (set[0] ?? "").toLowerCase(),
            src(set[0] ?? ""),
            `${set.map(n).join(" and ")} are both set; set one credential`,
          );
          return undefined;
        }
        if (token !== undefined) {
          this.#show("token", REDACTED, src("TOKEN"));
          plan = { source: "env", kind: "static_token", token };
        } else if (tokenFile !== undefined) {
          const path = this.#path(tokenFile, false);
          if (!readable(path)) {
            this.#problem("token_file", src("TOKEN_FILE"), `cannot read ${path}`);
            return undefined;
          }
          this.#show("token_file", path, src("TOKEN_FILE"));
          plan = { source: "env", kind: "token_file", path };
        } else if (keyId !== undefined) {
          if (secret !== undefined && secretFile !== undefined) {
            this.#problem(
              "key_secret",
              src("KEY_SECRET"),
              `${n("KEY_SECRET")} and ${n("KEY_SECRET_FILE")} are both set; set one`,
            );
            return undefined;
          }
          if (secret === undefined && secretFile === undefined) {
            this.#problem(
              "key_secret",
              src("KEY_ID"),
              `${n("KEY_ID")} is set without ${n("KEY_SECRET")} or ${n("KEY_SECRET_FILE")}`,
            );
            return undefined;
          }
          if (!this.#values.has("scopes")) {
            this.#problem("scopes", src("KEY_ID"), `a key needs scopes: set ${p}SCOPES`);
            return undefined;
          }
          this.#show("key_id", keyId, src("KEY_ID"));
          if (secret !== undefined) {
            this.#show("key_secret", REDACTED, src("KEY_SECRET"));
            plan = { source: "env", kind: "client_credentials", keyId, keySecret: secret };
          } else {
            const path = this.#path(secretFile ?? "", false);
            if (!readable(path)) {
              this.#problem("key_secret_file", src("KEY_SECRET_FILE"), `cannot read ${path}`);
              return undefined;
            }
            this.#show("key_secret_file", path, src("KEY_SECRET_FILE"));
            plan = { source: "env", kind: "client_credentials", keyId, keySecretFile: path };
          }
        }
        if (plan !== undefined) {
          tried.push({ source: "env", result: "used" });
        }
      }
    }

    // 3. Workload identity: reserved until the platform exchanges outside tokens.
    if (plan === undefined) {
      skip(
        "workload",
        this.#allowed("workload") ? "not offered by the platform yet" : "not in credential_sources",
      );
    }

    // 4. The config file's profile table.
    if (plan === undefined) {
      const label = this.#layers[0]?.label ?? "";
      if (!this.#allowed("file")) {
        skip("file", "not in credential_sources");
      } else if (this.#noFs) {
        skip("file", "this runtime has no file system");
      } else if (this.#filePath === null) {
        skip("file", "no config file was read");
      } else if (profile === undefined) {
        skip("file", "no profile chosen");
      } else if (
        table !== undefined &&
        (Object.hasOwn(table, "token_file") || Object.hasOwn(table, "key_id"))
      ) {
        const str = (k: string): string | undefined =>
          typeof table[k] === "string" ? (table[k] as string) : undefined;
        if (Object.hasOwn(table, "token_file") && Object.hasOwn(table, "key_id")) {
          this.#problem(
            "token_file",
            label,
            "token_file and key_id are both set; set one credential",
          );
          return undefined;
        }
        if (str("token_file") !== undefined) {
          const path = this.#safePath(str("token_file") ?? "");
          if (!readable(path)) {
            this.#problem("token_file", label, `cannot read ${path}`);
            return undefined;
          }
          this.#show("token_file", path, label);
          plan = { source: "file", kind: "token_file", path };
        } else if (str("key_id") !== undefined) {
          if (Object.hasOwn(table, "key_secret")) {
            return undefined; // Already reported: secrets are not allowed in the file.
          }
          const f = str("key_secret_file");
          if (f === undefined) {
            this.#problem("key_secret", label, "key_id is set without key_secret_file");
            return undefined;
          }
          if (!this.#values.has("scopes")) {
            this.#problem("scopes", label, "a key needs scopes: set scopes in the profile's table");
            return undefined;
          }
          const path = this.#safePath(f);
          if (!readable(path)) {
            this.#problem("key_secret_file", label, `cannot read ${path}`);
            return undefined;
          }
          this.#show("key_id", str("key_id"), label);
          this.#show("key_secret_file", path, label);
          plan = {
            source: "file",
            kind: "client_credentials",
            keyId: str("key_id") ?? "",
            keySecretFile: path,
          };
        } else {
          this.#problem("token_file", label, "must be a path string");
          return undefined;
        }
        tried.push({ source: "file", result: "used" });
      } else {
        skip("file", `profile ${profile} sets no token_file or key_id`);
      }
    }

    // 5. The iohr login.
    if (plan === undefined) {
      const program = (this.#values.get("cli_path") as string | undefined) ?? "iohr";
      if (!this.#allowed("cli")) {
        skip("cli", "not in credential_sources");
      } else if (fs() === undefined) {
        skip("cli", "this runtime cannot run iohr");
      } else if (profile === undefined) {
        skip("cli", "skipped, no profile chosen");
      } else if (table === undefined || !Object.hasOwn(table, "kind")) {
        skip("cli", `profile ${profile} was not made by iohr login`);
      } else if (!programFound(program, this.#env)) {
        skip("cli", program === "iohr" ? "iohr not found on PATH" : `iohr not found at ${program}`);
      } else {
        tried.push({ source: "cli", result: "used" });
        plan = { source: "cli", kind: "cli", profile, program };
      }
    }

    if (plan === undefined) {
      const lines = tried.map((t) => `\n  ${t.source}: ${t.reason ?? ""}`).join("");
      this.#problem(
        "credential",
        "",
        `no credentials found for profile ${JSON.stringify(profile ?? "default")}; tried:${lines}\nSet ${p}KEY_ID, ${p}KEY_SECRET and ${p}SCOPES, or ${p}TOKEN, or run \`iohr login\`.`,
      );
      return undefined;
    }
    this.#scopesFor(plan.kind);
    return { plan, described: { source: plan.source, kind: plan.kind, tried } };
  }

  #safePath(p: string): string {
    try {
      return this.#path(p, true);
    } catch {
      return p;
    }
  }

  /** `scopes` next to a credential that carries its own is listed as ignored. */
  #scopesFor(kind: CredentialPlan["kind"]): void {
    if (kind === "client_credentials" || kind === "custom") {
      return;
    }
    const scopes = this.#settings.scopes;
    if (scopes !== undefined) {
      delete this.#settings.scopes;
      this.#values.delete("scopes");
      this.#ignored.push({
        key: "scopes",
        source: scopes.source,
        reason: "not used by this credential",
      });
    }
  }

  #crossChecks(): void {
    const get = (k: string): DescribedSetting | undefined => this.#settings[k];
    if (get("system_trust")?.value === false && get("ca_bundle") === undefined) {
      this.#problem(
        "system_trust",
        get("system_trust")?.source ?? "",
        "system_trust = false needs a ca_bundle to trust instead",
      );
    }
    const cert = get("client_cert");
    const key = get("client_key");
    if (cert !== undefined && key === undefined) {
      this.#problem("client_key", cert.source, "client_cert needs client_key");
    } else if (cert === undefined && key !== undefined) {
      this.#problem("client_cert", key.source, "client_key needs client_cert");
    }
    for (const name of ["ca_bundle", "client_cert", "client_key"]) {
      const v = get(name);
      if (v !== undefined && typeof v.value === "string" && !readable(v.value)) {
        this.#problem(name, v.source, `cannot read ${v.value}`);
      }
    }
  }
}

function defaultValue(setting: Setting): unknown {
  if (setting.ty === "duration" && typeof setting.fallback === "string") {
    return parseDuration(setting.fallback);
  }
  return setting.fallback;
}
