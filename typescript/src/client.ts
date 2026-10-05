/**
 * The client: configuration (`load`, explicit options, `fromEnv`), the middleware
 * pipeline every call goes through, and streams (design.md sections 2 to 7, config.md).
 *
 * @module
 */

import {
  type CachedTokenOptions,
  ClientCredentials,
  CliToken,
  DEFAULT_TOKEN_URL,
  describe,
  StaticToken,
  type Token,
  TokenFile,
  type TokenProvider,
} from "./auth.js";
import { decode, type Shape, type Shapes } from "./codegen.js";
import {
  ApiError,
  ConfigError,
  ConnectionError,
  DecodeError,
  InOrbitError,
  RawResponse,
  TimeoutError,
} from "./errors.js";
import type { Hook } from "./hooks.js";
import {
  attemptOf,
  builtIns,
  CALL,
  type Context,
  MAX_BODY,
  newCallState,
  OPERATION,
  type OperationMarks,
  RetryBudget,
  rawOf,
  transport,
} from "./middleware.js";
import { Pipeline, type SdkRequest, type SdkResponse } from "./pipeline.js";
import { osArch, runtimeName } from "./platform.js";
import { parseNoProxy } from "./proxy.js";
import type { RateLimit } from "./ratelimit.js";
import {
  type CredentialSource,
  camel,
  type Description,
  type LoadOptions,
  type Resolution,
  resolve,
  SETTING_NAMES,
} from "./settings.js";
import { envelopeError, SocketHub, SseParser, type StreamTransport } from "./stream.js";
import {
  Log,
  type Logger,
  type LogLevel,
  type MeterProviderLike,
  type OpenTelemetryApi,
  type Redact,
  type Telemetry,
  type TracerProviderLike,
  telemetry,
} from "./telemetry.js";
import { DEFAULT_CONNECT_TIMEOUT, needsNodeTransport, nodeFetch } from "./transport.js";
import { SDK_VERSION } from "./version.js";

export { MAX_BODY };

/** Where the API is. */
export const DEFAULT_BASE_URL = "https://api.inorbit.hr";

/** An HTTP method. */
export type Method = "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD";

const IDEMPOTENT: ReadonlySet<string> = new Set(["GET", "PUT", "DELETE", "HEAD"]);
const RETRY_BUDGET_CAPACITY = 500;

/** One call, as a generated surface builds it. */
export interface Operation {
  /** What hooks see (`radar.list_digests`). */
  readonly name?: string;
  /** The method. */
  readonly method: Method;
  /** The path, parameters bound and encoded. */
  readonly path: string;
  /** The path template (`/v1/radar/digests/{digest_id}`), for span names. */
  readonly template?: string;
  /** Query parameters; `undefined` ones are left out, arrays repeat the name. */
  readonly query?: ReadonlyArray<readonly [string, unknown]>;
  /** The JSON body. */
  readonly body?: unknown;
  /** The scopes the operation needs, for the record. */
  readonly scopes?: readonly string[];
  /** Retry it like an idempotent method although its method is not. */
  readonly idempotent?: boolean;
  /**
   * The operation takes an `Idempotency-Key` (config.md section 7.5): one is sent with
   * every attempt, so the write is retried like a read.
   */
  readonly idempotencyKey?: boolean;
}

/** A streaming operation: the call, and how the socket names it. */
export interface StreamOperation extends Operation {
  /** The RPC a `/v1/ws` call frame names (`iohr.events.v1.EventsService/StreamEvents`). */
  readonly rpc?: string;
  /** The path and query parameters as one JSON object, the socket call's body. */
  readonly fields?: Readonly<Record<string, unknown>>;
}

/** Per-call options. */
export interface CallOptions {
  /** Aborts the call, retries included. */
  readonly signal?: AbortSignal;
  /**
   * Milliseconds the call may take (default: the client's `timeout` per attempt). It
   * replaces the attempt timeout and shortens `totalTimeout`; it never extends it.
   */
  readonly timeout?: number;
  /**
   * The `Idempotency-Key` to send, on an operation that takes one (default: a fresh UUID
   * per call). Reuse it to repeat a call safely. On any other operation it is a
   * {@link ConfigError}.
   */
  readonly idempotencyKey?: string;
  /** A W3C `traceparent` to continue: the call's parent, or sent as is when tracing is off. */
  readonly traceparent?: string;
}

/** A typed answer and the raw one beside it. */
export interface Response<T> {
  /** The answer, typed. */
  readonly value: T;
  /** The answer as it came. */
  readonly raw: RawResponse;
}

/**
 * How to build a client. Give `token`, or `keyId` with `keySecret` (or `keySecretFile`)
 * and `scopes`, a `tokenFile`, or a `tokenProvider`. Durations are milliseconds. Every
 * setting of config.md section 3 is here in camel case; `Client.load` also reads them
 * from the environment and the config file.
 */
export interface ClientOptions {
  /** An API token (from the console or `iohr token create`). */
  readonly token?: string;
  /** A file holding an API token, read again when it changes. */
  readonly tokenFile?: string;
  /** An API key's id. */
  readonly keyId?: string;
  /** An API key's secret. */
  readonly keySecret?: string;
  /** A file holding the key's secret, read before every token exchange. */
  readonly keySecretFile?: string;
  /** The scopes to ask for with a key; no default. */
  readonly scopes?: readonly string[];
  /** Your own token source. */
  readonly tokenProvider?: TokenProvider;
  /** The API's origin (default `https://api.inorbit.hr`; plain HTTP only to this machine). */
  readonly baseUrl?: string;
  /** The token endpoint for a key (default `https://auth.inorbit.hr/oauth2/token`). */
  readonly tokenUrl?: string;
  /** Reserved until the API offers regions; setting it is a {@link ConfigError}. */
  readonly region?: string;
  /** Which credential sources `load` may use (default all: env, workload, file, cli). */
  readonly credentialSources?: readonly CredentialSource[];
  /** The `iohr` command line the `cli` source runs (default `iohr` on `PATH`). */
  readonly cliPath?: string;
  /** Milliseconds a new connection may take, TLS included (default 10 000; Node). */
  readonly connectTimeout?: number;
  /** Milliseconds each attempt may take, the whole answer included (default 30 000). */
  readonly timeout?: number;
  /** Milliseconds one call may take, every attempt and wait included (default 120 000). */
  readonly totalTimeout?: number;
  /** Retries after the first attempt (default 2; 0 disables). */
  readonly maxRetries?: number;
  /** The backoff's base, in milliseconds (default 500). */
  readonly retryBaseDelay?: number;
  /** The backoff's cap, in milliseconds (default 8 000). */
  readonly retryMaxDelay?: number;
  /** The longest `Retry-After` waited for, in milliseconds (default 60 000); a longer one ends the call. */
  readonly retryAfterMax?: number;
  /** The per-client retry quota (default `true`). */
  readonly retryBudget?: boolean;
  /** The retry quota's capacity (default 500); for tests. */
  readonly retryBudgetCapacity?: number;
  /** A proxy URL (`http://` or `https://`), or `"off"` for none, the standard variables included (Node). */
  readonly proxy?: string;
  /** Hosts reached without the proxy: a comma-separated string or a list (config.md section 6.2). */
  readonly noProxy?: string | readonly string[];
  /** PEM certificates added to the trust store (Node). */
  readonly caBundle?: string;
  /** `false` trusts `caBundle` only (default `true`; Node). */
  readonly systemTrust?: boolean;
  /** A PEM client certificate chain for mTLS (Node). */
  readonly clientCert?: string;
  /** The PEM private key for `clientCert` (Node). */
  readonly clientKey?: string;
  /** The password of an encrypted `clientKey`. */
  readonly clientKeyPassword?: string;
  /** Base64 SHA-256 hashes of pinned public keys, at least two (Node). */
  readonly pinnedKeys?: readonly string[];
  /** The log level (default `"off"`). */
  readonly log?: LogLevel;
  /** Log allowlisted header values at `debug` (default `false`). */
  readonly logHeaders?: boolean;
  /** Header names added to the logging allowlist. */
  readonly logAllowHeaders?: readonly string[];
  /** Where records go (default `console` when `log` is set). */
  readonly logger?: Logger;
  /** Sees every log record last, and may change or drop it. */
  readonly redact?: Redact;
  /** Spans for calls and attempts (default: on when `@opentelemetry/api` is there). */
  readonly tracing?: boolean;
  /** Metrics (default: as `tracing`). */
  readonly metrics?: boolean;
  /** The tracer provider (default the global one). */
  readonly tracerProvider?: TracerProviderLike;
  /** The meter provider (default the global one). */
  readonly meterProvider?: MeterProviderLike;
  /** The `@opentelemetry/api` module, where it cannot be found by name (bundles, Deno). */
  readonly opentelemetry?: OpenTelemetryApi;
  /** Rate-limit headers: read them (`"observe"`, the default), also wait out an empty window (`"wait"`), or ignore them (`"off"`). */
  readonly rateLimit?: "observe" | "wait" | "off";
  /** Appended to the user agent: product tokens, at most 128 characters. */
  readonly userAgentSuffix?: string;
  /** Observers of every attempt. */
  readonly hooks?: readonly Hook[];
  /** Edits the pipeline: add, insert, replace or remove middlewares by name. */
  readonly pipeline?: (pipeline: Pipeline) => Pipeline | undefined;
  /** The `fetch` to use (default the global one). Proxy and TLS settings then belong to it. */
  readonly fetch?: typeof fetch;
  /** An `undici` dispatcher passed to every `fetch` (Node); proxy and TLS settings then belong to it. */
  readonly dispatcher?: unknown;
  /**
   * How streams open (default `"sse"`): server-sent events, or `"socket"`, every stream
   * over one `/v1/ws` connection. The socket sends the token as a header, which the
   * browsers' WebSocket cannot: use it on Node, Deno and Bun; browsers use `"sse"`.
   */
  readonly streams?: StreamTransport;
  /** Milliseconds a stream may be silent, not even a keep-alive, before it fails (default 45 000). */
  readonly streamIdleTimeout?: number;
}

/** What `Client.load` takes: every client option, and where to look. */
export interface LoadClientOptions extends ClientOptions {
  /** The config file's profile (default `INORBIT_PROFILE`, then the file's `default`). */
  readonly profile?: string;
  /** The config file, or `"off"` for none (default `INORBIT_CONFIG_FILE`, then the `iohr` location). */
  readonly configFile?: string;
  /** The environment, OS and home directory to resolve with, instead of the process's. */
  readonly loadOptions?: LoadOptions;
  /** For generated profile classes: resolve as this typed profile (`INORBIT_<P>_*`, `[profiles.<p>]`). */
  readonly profileType?: string;
}

/** A client's effective configuration (config.md section 2.6). */
export class ResolvedConfig {
  readonly #doc: Description;

  /** Wraps a `describe()` document. */
  constructor(doc: Description) {
    this.#doc = doc;
  }

  /** The profile resolved for, if any. */
  get profile(): string | undefined {
    return this.#doc.profile?.name;
  }

  /** The document: each setting with its value and source, secrets redacted. */
  describe(): Description {
    return JSON.parse(JSON.stringify(this.#doc)) as Description;
  }

  /** The same as {@link describe}. */
  toJSON(): Description {
    return this.describe();
  }
}

/**
 * Resolves a configuration as `Client.load` would, without building a client: what a
 * program would use and where each value came from.
 *
 * @throws {ConfigError} listing every problem.
 */
export function loadConfig(options: LoadClientOptions = {}): ResolvedConfig {
  return new ResolvedConfig(resolveFor(options, false).description);
}

function checkUrl(what: string, raw: string, originOnly: boolean): URL {
  let url: URL;
  try {
    url = new URL(raw);
  } catch (e) {
    throw new ConfigError(`${what} is not usable: ${describe(e)}`);
  }
  const host = url.hostname.replace(/^\[|\]$/g, "");
  const loopback = host === "localhost" || host === "::1" || /^127\./.test(host);
  if (!(url.protocol === "https:" || (url.protocol === "http:" && loopback))) {
    throw new ConfigError(
      `${what} is not usable: it must use https (plain http only to this machine)`,
    );
  }
  if (url.username !== "" || url.password !== "" || url.hash !== "") {
    throw new ConfigError(`${what} is not usable: it must not carry credentials or a fragment`);
  }
  if (originOnly && (url.pathname !== "/" || url.search !== "")) {
    throw new ConfigError(
      `${what} is not usable: it is an origin only, such as https://api.inorbit.hr`,
    );
  }
  return url;
}

interface Environment {
  readonly env?: Readonly<Record<string, string | undefined>>;
}

function environment(): Environment {
  return ((globalThis as { process?: Environment }).process ?? {}) as Environment;
}

/** `inorbithr-sdk-typescript/<version> <runtime>/<version> <os>/<arch>[ <suffix>]` (config.md section 7.6). */
function userAgent(suffix: string | undefined): string {
  const rt = runtimeName();
  const ua = `inorbithr-sdk-typescript/${SDK_VERSION} ${rt.name}/${rt.version} ${osArch()}`;
  return suffix === undefined || suffix === "" ? ua : `${ua} ${suffix}`;
}

function codeOptions(o: LoadClientOptions): Map<string, unknown> {
  const code = new Map<string, unknown>();
  const bag = o as Record<string, unknown>;
  for (const name of SETTING_NAMES) {
    const v = bag[camel(name)];
    if (v !== undefined) {
      code.set(name, v);
    }
  }
  if (o.profile !== undefined) {
    code.set("profile", o.profile);
  }
  if (o.configFile !== undefined) {
    code.set("config_file", o.configFile);
  }
  return code;
}

function resolveFor(o: LoadClientOptions, explicit: boolean): Resolution {
  return resolve({
    code: codeOptions(o),
    httpClient: o.fetch !== undefined || o.dispatcher !== undefined,
    tokenProvider: o.tokenProvider !== undefined,
    profileType: o.profileType,
    explicit,
    load: o.loadOptions,
  });
}

/** The checks of explicit construction, with the messages they always had. */
function checkExplicit(options: ClientOptions): void {
  checkUrl("the base URL", options.baseUrl ?? DEFAULT_BASE_URL, true);
  checkUrl("the token URL", options.tokenUrl ?? DEFAULT_TOKEN_URL, false);
  const streams = options.streams ?? "sse";
  if (streams !== "sse" && streams !== "socket") {
    throw new ConfigError(`streams is "sse" or "socket", not ${JSON.stringify(streams)}`);
  }
  if (
    options.tokenProvider !== undefined ||
    (options.token !== undefined && options.token !== "") ||
    (options.tokenFile !== undefined && options.tokenFile !== "")
  ) {
    return;
  }
  if (
    options.keyId !== undefined &&
    (options.keySecret !== undefined || options.keySecretFile !== undefined)
  ) {
    if (options.scopes === undefined || options.scopes.length === 0) {
      throw new ConfigError(
        'no scopes: set INORBIT_SCOPES (space-separated, such as "identity:read account:read")',
      );
    }
    return;
  }
  throw new ConfigError(
    "no credentials: set INORBIT_TOKEN, or INORBIT_KEY_ID and INORBIT_KEY_SECRET",
  );
}

/** The provider the chain decided on (config.md section 5.1). */
function providerFor(
  res: Resolution,
  custom: TokenProvider | undefined,
  send: typeof fetch,
  userAgentValue: string,
  cache: CachedTokenOptions,
): TokenProvider {
  const plan = res.credential;
  switch (plan.kind) {
    case "custom":
      return custom as TokenProvider;
    case "static_token":
      return new StaticToken(plan.token);
    case "token_file":
      return new TokenFile(plan.path);
    case "cli":
      return new CliToken({ profile: plan.profile, program: plan.program, cache });
    default:
      return new ClientCredentials({
        keyId: plan.keyId,
        ...(plan.keySecret === undefined ? {} : { keySecret: plan.keySecret }),
        ...(plan.keySecretFile === undefined ? {} : { keySecretFile: plan.keySecretFile }),
        scopes: (res.values.get("scopes") as string[] | undefined) ?? [],
        tokenUrl: String(res.values.get("token_url")),
        fetch: send,
        userAgent: userAgentValue,
        cache,
      });
  }
}

/**
 * The credential chain of config.md section 5.1 as a {@link TokenProvider}: code, the
 * environment, (workload identity, reserved), the config file, then the `iohr` login,
 * decided now. For a program that builds its own client or chain.
 *
 * @throws {ConfigError} listing every source tried when none has credentials.
 */
export class DefaultCredential implements TokenProvider {
  readonly #provider: TokenProvider;
  readonly #config: ResolvedConfig;

  /** Resolves the chain as `Client.load` would with `options`. */
  constructor(options: LoadClientOptions = {}) {
    const res = resolveFor(options, false);
    this.#config = new ResolvedConfig(res.description);
    this.#provider = providerFor(
      res,
      options.tokenProvider,
      options.fetch ?? globalThis.fetch.bind(globalThis),
      userAgent(res.values.get("user_agent_suffix") as string | undefined),
      {},
    );
  }

  /** Which source was chosen and what was tried. */
  config(): ResolvedConfig {
    return this.#config;
  }

  /** A token from the source chosen. */
  token(): Promise<Token> {
    return this.#provider.token();
  }

  /** Tells the source chosen that its token was refused. */
  async invalidate(): Promise<void> {
    await this.#provider.invalidate?.();
  }

  /** Never a token. */
  toJSON(): string {
    return `DefaultCredential(${this.#config.describe().credential?.source ?? "none"})`;
  }

  /** Never a token. */
  toString(): string {
    return this.toJSON();
  }
}

/** Options objects `load` resolved, so the constructor builds from the resolution. */
const LOADED = new WeakMap<object, Resolution>();

function randomRequestId(): string {
  const bytes = crypto.getRandomValues(new Uint8Array(8));
  return `iohr-${[...bytes].map((b) => b.toString(16).padStart(2, "0")).join("")}`;
}

/**
 * A client for one credential. Safe to share; it holds no per-call state.
 *
 * ```ts
 * const client = Client.load(); // the environment, the iohr config file, the iohr login
 * ```
 */
export class Client {
  readonly #base: URL;
  readonly #ctx: Context;
  readonly #pipeline: Pipeline;
  readonly #send: (req: SdkRequest) => Promise<SdkResponse>;
  readonly #hooks: readonly Hook[];
  readonly #log: Log;
  readonly #streams: StreamTransport;
  readonly #idle: number;
  readonly #config: ResolvedConfig;
  readonly #profile: string | undefined;
  readonly #dispatcher: unknown;
  #hub: SocketHub | undefined;

  /**
   * A client with `options` and nothing else: no environment, no file. For libraries
   * and tests; applications use {@link Client.load}.
   *
   * @throws {ConfigError} when no credential is given, a key has no scopes, a URL is not
   *   https (plain http only to this machine), or a setting is not usable.
   */
  constructor(options: ClientOptions) {
    let res = LOADED.get(options);
    const explicit = res === undefined;
    if (res === undefined) {
      checkExplicit(options);
      res = resolveFor(options, true);
    }
    const v = res.values;
    const num = (k: string): number => v.get(k) as number;
    this.#profile = res.profile;
    this.#base = new URL(String(v.get("base_url")));
    const ts = {
      proxy: res.proxy,
      noProxy: parseNoProxy((v.get("no_proxy") as string[] | undefined) ?? []),
      caBundle: v.get("ca_bundle") as string | undefined,
      systemTrust: v.get("system_trust") !== false,
      clientCert: v.get("client_cert") as string | undefined,
      clientKey: v.get("client_key") as string | undefined,
      clientKeyPassword: v.get("client_key_password") as string | undefined,
      pinnedKeys: v.get("pinned_keys") as string[] | undefined,
      connectTimeout: (v.get("connect_timeout") as number | undefined) ?? DEFAULT_CONNECT_TIMEOUT,
    };
    const send: typeof fetch =
      options.fetch ??
      (options.dispatcher === undefined && needsNodeTransport(ts)
        ? nodeFetch(ts)
        : globalThis.fetch.bind(globalThis));
    const ua = userAgent(v.get("user_agent_suffix") as string | undefined);
    this.#hooks = options.hooks ?? [];
    this.#log = new Log({
      level: (v.get("log") as LogLevel | undefined) ?? "off",
      logger: options.logger,
      redact: options.redact,
      profile: res.profile,
      headers: v.get("log_headers") === true,
      allow: v.get("log_allow_headers") as string[] | undefined,
    });
    let telemetryPromise: Promise<Telemetry | undefined> | undefined;
    const tracing = v.get("tracing") as boolean | undefined;
    const metrics = v.get("metrics") as boolean | undefined;
    const getTelemetry = (): Promise<Telemetry | undefined> => {
      telemetryPromise ??= telemetry({
        tracing,
        metrics,
        api: options.opentelemetry,
        tracerProvider: options.tracerProvider,
        meterProvider: options.meterProvider,
      }).catch(() => undefined);
      return telemetryPromise;
    };
    const log = this.#log;
    const source = res.credential.source;
    const cache: CachedTokenOptions = {
      onRefresh: (error) => {
        void getTelemetry().then((t) =>
          t?.exchanges?.add?.(1, {
            "inorbit.credential.source": source,
            ...(error === undefined
              ? {}
              : { "error.type": error instanceof InOrbitError ? error.kind : "error" }),
          }),
        );
      },
      onRefreshFailed: (error) => {
        log.emit("warn", {
          event: "token_refresh_failed",
          reason: error instanceof InOrbitError ? error.kind : "error",
        });
      },
    };
    const provider = providerFor(res, options.tokenProvider, send, ua, cache);
    const capacity = options.retryBudgetCapacity ?? RETRY_BUDGET_CAPACITY;
    this.#ctx = {
      host: this.#base.host,
      provider,
      staticToken: !explicit && res.credential.kind === "static_token",
      userAgent: ua,
      timeout: num("timeout"),
      totalTimeout: num("total_timeout"),
      maxRetries: num("max_retries"),
      retryBaseDelay: num("retry_base_delay"),
      retryMaxDelay: num("retry_max_delay"),
      retryAfterMax: num("retry_after_max"),
      budget: v.get("retry_budget") === false ? undefined : new RetryBudget(capacity),
      rateLimit: (v.get("rate_limit") as "observe" | "wait" | "off" | undefined) ?? "observe",
      log: this.#log,
      hooks: this.#hooks,
      telemetry: getTelemetry,
      latest: undefined,
    };
    const pipeline = new Pipeline(builtIns(this.#ctx));
    this.#pipeline = options.pipeline?.(pipeline) ?? pipeline;
    this.#send = transport(this.#base.host, send, options.dispatcher);
    this.#dispatcher = options.dispatcher;
    this.#streams = (v.get("streams") as StreamTransport | undefined) ?? "sse";
    this.#idle = num("stream_idle_timeout");
    this.#config = new ResolvedConfig({
      ...res.description,
      pipeline: this.#pipeline.names(),
    });
  }

  /**
   * A client configured from code, the environment, the `iohr` config file and the
   * `iohr` login, each setting from the first that sets it (config.md section 2). Reads
   * files and the environment now; contacts no host until the first call.
   *
   * ```ts
   * const client = Client.load({ timeout: 5_000 });
   * ```
   *
   * @throws {ConfigError} listing every problem found, or every credential source tried.
   */
  static load(options: LoadClientOptions = {}): Client {
    const res = resolveFor(options, false);
    const opts = { ...options };
    LOADED.set(opts, res);
    return new Client(opts);
  }

  /**
   * A client from the environment: `INORBIT_<PROFILE>_TOKEN`, or `INORBIT_<PROFILE>_KEY_ID`,
   * `_KEY_SECRET` and `_SCOPES`, and nothing else for a named profile; the bare
   * `INORBIT_*` names without one. `INORBIT_BASE_URL` and `INORBIT_TOKEN_URL` apply to
   * every profile. Kept as it is; {@link Client.load} reads more and is the one to use.
   *
   * @throws {ConfigError} naming the variables to set when no credential is there.
   */
  static fromEnv(profile?: string): Client {
    const env = environment().env ?? {};
    const prefix = profile === undefined || profile === "" ? "INORBIT_" : `INORBIT_${profile}_`;
    const v = (name: string): string | undefined => {
      const value = env[`${prefix}${name}`];
      return value === undefined || value === "" ? undefined : value;
    };
    const shared = (name: string): string | undefined =>
      v(name) ?? (env[`INORBIT_${name}`] || undefined);
    const base = { baseUrl: shared("BASE_URL"), tokenUrl: shared("TOKEN_URL") };
    const urls = Object.fromEntries(Object.entries(base).filter(([, x]) => x !== undefined));
    const token = v("TOKEN");
    if (token !== undefined) {
      return new Client({ ...urls, token });
    }
    const keyId = v("KEY_ID");
    const keySecret = v("KEY_SECRET");
    if (keyId !== undefined && keySecret !== undefined) {
      const scopes = v("SCOPES")
        ?.split(/\s+/)
        .filter((s) => s !== "");
      if (scopes === undefined || scopes.length === 0) {
        throw new ConfigError(
          `no scopes: set ${prefix}SCOPES (space-separated, such as "identity:read account:read")`,
        );
      }
      return new Client({ ...urls, keyId, keySecret, scopes });
    }
    throw new ConfigError(
      `no credentials: set ${prefix}TOKEN, or ${prefix}KEY_ID and ${prefix}KEY_SECRET`,
    );
  }

  /** The API's origin this client calls. */
  get baseUrl(): string {
    return this.#base.origin;
  }

  /** What this client uses and where each value came from (`describe()`), secrets redacted. */
  config(): ResolvedConfig {
    return this.#config;
  }

  /** The latest rate-limit snapshot any answer carried, or `undefined`. */
  rateLimit(): RateLimit | undefined {
    return this.#ctx.latest?.snapshot;
  }

  /**
   * Calls `op` and reads its JSON answer; `shape` and `shapes` convert 64-bit integers.
   *
   * @throws {ApiError} for an error answer, and the other {@link InOrbitError}s for
   *   connection, timeout, token, size and decoding failures.
   */
  async request<T>(
    op: Operation,
    options?: CallOptions,
    shape?: Shape,
    shapes?: Shapes,
  ): Promise<Response<T>> {
    const raw = await this.send(op, options);
    let value: unknown = {};
    if (raw.body.length > 0) {
      try {
        value = JSON.parse(raw.text());
      } catch (e) {
        throw new DecodeError(describe(e), raw);
      }
    }
    if (shape !== undefined && shapes !== undefined) {
      try {
        value = decode(value, shape, shapes);
      } catch (e) {
        throw new DecodeError(describe(e), raw);
      }
    }
    return { value: value as T, raw };
  }

  /** Calls `op` and hands back the answer as it came, a 2xx one only. */
  async send(op: Operation, options?: CallOptions): Promise<RawResponse> {
    return rawOf(await this.#call(op, options, false));
  }

  /**
   * Opens the stream `op` and yields each event, typed, until the server ends it
   * (design.md section 7). Opening goes through the pipeline like a `GET`; the first
   * `next()` opens it, so an opening error is thrown there. Stopping the loop, or aborting
   * `options.signal`, closes the stream.
   *
   * @throws {ApiError} for an error answer or an `error` event, {@link TimeoutError}
   *   after `streamIdleTimeout` of silence, and the other {@link InOrbitError}s.
   */
  async *stream<T>(
    op: StreamOperation,
    options?: CallOptions,
    shape?: Shape,
    shapes?: Shapes,
  ): AsyncGenerator<T, void, undefined> {
    const typed = (value: unknown): T =>
      (shape !== undefined && shapes !== undefined ? decode(value, shape, shapes) : value) as T;
    if (this.#streams === "socket") {
      if (op.rpc === undefined || op.rpc === "") {
        throw new ConfigError(
          `${op.name ?? op.path} names no RPC, so it cannot go over the socket`,
        );
      }
      this.#hub ??= this.#socketHub();
      for await (const body of this.#hub.call(op.rpc, op.fields ?? {}, options?.signal)) {
        yield typed(body);
      }
      return;
    }
    const resp = await this.#call(op, options, true);
    const requestId = resp.request.info.requestId ?? "";
    const reader = (resp.stream ?? new ReadableStream<Uint8Array>()).getReader();
    const text = new TextDecoder();
    const parser = new SseParser();
    try {
      for (;;) {
        let timer: ReturnType<typeof setTimeout> | undefined;
        const idle = new Promise<never>((_, reject) => {
          timer = setTimeout(
            () => reject(new TimeoutError(this.#base.host, Math.round(this.#idle / 1000))),
            this.#idle,
          );
        });
        let chunk: ReadableStreamReadResult<Uint8Array>;
        try {
          chunk = await Promise.race([reader.read(), idle]);
        } catch (e) {
          if (options?.signal?.aborted) {
            throw options.signal.reason;
          }
          if (e instanceof InOrbitError) {
            throw e;
          }
          throw new ConnectionError(this.#base.host, describe(e), { cause: e });
        } finally {
          clearTimeout(timer);
        }
        if (chunk.done) {
          return;
        }
        for (const event of parser.push(text.decode(chunk.value, { stream: true }))) {
          let value: unknown;
          try {
            value = JSON.parse(event.data);
          } catch (e) {
            throw new DecodeError(
              describe(e),
              new RawResponse({
                status: resp.status,
                headers: resp.headers,
                body: new TextEncoder().encode(event.data),
                requestId,
                attempts: Math.max(1, resp.request.info.attempt),
              }),
            );
          }
          if (event.event === "error") {
            throw envelopeError(value, requestId);
          }
          if (event.event === "message") {
            yield typed(value);
          }
        }
      }
    } finally {
      reader.cancel().catch(() => undefined);
    }
  }

  #socketHub(): SocketHub {
    const url = new URL("/v1/ws", this.#base);
    url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
    const ctx = this.#ctx;
    const names = this.#pipeline.names();
    return new SocketHub({
      url: url.href,
      host: this.#base.host,
      token: async () => (await ctx.provider.token()).access,
      invalidate: async () => {
        await ctx.provider.invalidate?.();
      },
      maxRetries: ctx.maxRetries,
      headers: () => ({
        ...(names.includes("request_id") ? { "x-request-id": randomRequestId() } : {}),
        ...(names.includes("user_agent") ? { "user-agent": ctx.userAgent } : {}),
      }),
      reconnect: () => ctx.budget?.take(10) ?? true,
      dispatcher: this.#dispatcher,
    });
  }

  async #call(
    op: Operation,
    options: CallOptions | undefined,
    stream: boolean,
  ): Promise<SdkResponse> {
    const url = this.#url(op);
    const name = op.name ?? op.path;
    if (options?.idempotencyKey !== undefined && op.idempotencyKey !== true) {
      throw new ConfigError(
        `${name} does not take an idempotency key: the API would ignore it, so repeating the call would not be safe`,
      );
    }
    const headers = new Headers({ accept: stream ? "text/event-stream" : "application/json" });
    let body: Uint8Array | undefined;
    if (op.body !== undefined) {
      headers.set("content-type", "application/json");
      body = new TextEncoder().encode(
        JSON.stringify(op.body, (_, x: unknown) => (typeof x === "bigint" ? x.toString() : x)),
      );
    }
    if (options?.traceparent !== undefined) {
      headers.set("traceparent", options.traceparent);
    }
    const state = newCallState(options?.timeout, options?.traceparent);
    const marks: OperationMarks = {
      idempotencyKey: op.idempotencyKey === true,
      key: options?.idempotencyKey,
    };
    const request = {
      method: op.method,
      url,
      headers,
      body,
      signal: options?.signal,
      info: {
        operation: name,
        template: op.template,
        idempotent: op.idempotent === true || IDEMPOTENT.has(op.method),
        idempotencyKey: undefined,
        requestId: undefined,
        attempt: 0,
        deadline: undefined,
        stream,
        profile: this.#profile,
        stage: "per_call" as const,
      },
      [CALL]: state,
      [OPERATION]: marks,
    };
    const started = Date.now();
    let resp: SdkResponse;
    try {
      resp = await this.#pipeline.run(request, this.#send);
    } catch (e) {
      if (!(e instanceof InOrbitError)) {
        throw e;
      }
      this.#failed(request, state.attempts, e, started);
      throw e;
    }
    if (resp.status >= 200 && resp.status < 300) {
      this.#log.emit("info", {
        event: "call",
        operation: name,
        status: resp.status,
        attempts: Math.max(1, resp.request.info.attempt),
        duration_ms: Date.now() - started,
        request_id: resp.request.info.requestId,
        server_request_id: resp.headers.get("x-request-id") ?? undefined,
      });
      return resp;
    }
    await resp.stream?.cancel().catch(() => undefined);
    const error = new ApiError(rawOf(resp));
    error.requestId = resp.request.info.requestId;
    error.idempotencyKey = resp.request.info.idempotencyKey;
    this.#failed(resp.request, Math.max(1, resp.request.info.attempt), error, started);
    throw error;
  }

  #failed(request: SdkRequest, attempts: number, error: InOrbitError, started: number): void {
    const attempt = {
      ...attemptOf(request),
      number: Math.max(1, attempts),
      requestId: error.requestId ?? request.info.requestId ?? "",
      idempotencyKey: error.idempotencyKey ?? request.info.idempotencyKey,
    };
    if (this.#pipeline.names().includes("hooks")) {
      for (const h of this.#hooks) {
        h.onError?.(attempt, error);
      }
    }
    const code = (error as { code?: unknown }).code;
    const status = (error as { status?: unknown }).status;
    const fields = {
      operation: request.info.operation,
      error_kind: error.kind,
      ...(typeof code === "string" ? { error_code: code } : {}),
      ...(typeof status === "number" ? { status } : {}),
      request_id: attempt.requestId,
    };
    this.#log.emit("info", {
      event: "call",
      ...fields,
      attempts: attempt.number,
      duration_ms: Date.now() - started,
      ...(error instanceof ApiError ? { server_request_id: error.raw.serverRequestId } : {}),
    });
    this.#log.emit("error", { event: "call_failed", ...fields });
  }

  #url(op: Operation): URL {
    const p = op.path;
    if (!p.startsWith("/") || p.startsWith("//") || p.includes("?") || p.includes("#")) {
      throw new ConfigError(
        `the path ${JSON.stringify(p)} is not usable: it starts with one / and has no query`,
      );
    }
    const url = new URL(p, this.#base);
    for (const [name, value] of op.query ?? []) {
      for (const v of Array.isArray(value) ? value : [value]) {
        if (v !== undefined && v !== null) {
          url.searchParams.append(name, String(v));
        }
      }
    }
    return url;
  }
}
