/**
 * The client: configuration, the one request path every operation goes through, and the
 * retry and token rules of design.md sections 3 to 6.
 *
 * @module
 */

import {
  ClientCredentials,
  DEFAULT_TOKEN_URL,
  describe,
  StaticToken,
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
  TooLargeError,
} from "./errors.js";
import type { Attempt, Hook } from "./hooks.js";
import { backoffMs, requestId, retryAfterMs, retryableStatus, sleep } from "./retry.js";
import { envelopeError, SocketHub, SseParser, type StreamTransport } from "./stream.js";
import { SDK_VERSION } from "./version.js";

/** Where the API is. */
export const DEFAULT_BASE_URL = "https://api.inorbit.hr";

/** The largest answer read, 16 MiB. */
export const MAX_BODY: number = 16 * 1024 * 1024;

/** An HTTP method. */
export type Method = "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD";

const IDEMPOTENT: ReadonlySet<Method> = new Set(["GET", "PUT", "DELETE", "HEAD"]);

/** One call, as a generated surface builds it. */
export interface Operation {
  /** What hooks see (`radar.list_digests`). */
  readonly name?: string;
  /** The method. */
  readonly method: Method;
  /** The path, parameters bound and encoded. */
  readonly path: string;
  /** Query parameters; `undefined` ones are left out, arrays repeat the name. */
  readonly query?: ReadonlyArray<readonly [string, unknown]>;
  /** The JSON body. */
  readonly body?: unknown;
  /** The scopes the operation needs, for the record. */
  readonly scopes?: readonly string[];
  /** Retry it like an idempotent method although its method is not. */
  readonly idempotent?: boolean;
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
  /** Milliseconds each attempt may take (default: the client's). */
  readonly timeout?: number;
}

/** A typed answer and the raw one beside it. */
export interface Response<T> {
  /** The answer, typed. */
  readonly value: T;
  /** The answer as it came. */
  readonly raw: RawResponse;
}

/** How to build a client. Give `token`, or `keyId` with `keySecret` and `scopes`, or a `tokenProvider`. */
export interface ClientOptions {
  /** An API token (from the console or `iohr token create`). */
  readonly token?: string;
  /** An API key's id. */
  readonly keyId?: string;
  /** An API key's secret. */
  readonly keySecret?: string;
  /** The scopes to ask for with a key; no default. */
  readonly scopes?: readonly string[];
  /** Your own token source. */
  readonly tokenProvider?: TokenProvider;
  /** The API's origin (default `https://api.inorbit.hr`; plain HTTP only to this machine). */
  readonly baseUrl?: string;
  /** The token endpoint for a key (default `https://auth.inorbit.hr/oauth2/token`). */
  readonly tokenUrl?: string;
  /** Milliseconds each attempt may take (default 30 000). */
  readonly timeout?: number;
  /** Retries after the first attempt (default 2; 0 disables). */
  readonly maxRetries?: number;
  /** Appended to the user agent. */
  readonly userAgentSuffix?: string;
  /** Observers of every attempt. */
  readonly hooks?: readonly Hook[];
  /** The `fetch` to use (default the global one). */
  readonly fetch?: typeof fetch;
  /**
   * How streams open (default `"sse"`): server-sent events, or `"socket"`, every stream
   * over one `/v1/ws` connection. The socket sends the token as a header, which the
   * browsers' WebSocket cannot: use it on Node, Deno and Bun; browsers use `"sse"`.
   */
  readonly streams?: StreamTransport;
  /** Milliseconds a stream may be silent, not even a keep-alive, before it fails (default 45 000). */
  readonly streamIdleTimeout?: number;
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
  readonly version?: string;
  readonly platform?: string;
  readonly arch?: string;
}

function environment(): Environment {
  return ((globalThis as { process?: Environment }).process ?? {}) as Environment;
}

function userAgent(suffix: string | undefined): string {
  const p = environment();
  const runtime = p.version === undefined ? "js/unknown" : `node/${p.version.replace(/^v/, "")}`;
  const ua = `inorbithr-sdk-typescript/${SDK_VERSION} ${runtime} ${p.platform ?? "unknown"}/${p.arch ?? "unknown"}`;
  return suffix === undefined || suffix === "" ? ua : `${ua} ${suffix}`;
}

/** What one attempt came to. */
type Outcome =
  | { readonly kind: "done"; readonly raw: RawResponse }
  | {
      readonly kind: "open";
      readonly resp: globalThis.Response;
      readonly abort: AbortController;
      readonly attempt: Attempt;
    }
  | { readonly kind: "unauthorized"; readonly raw: RawResponse }
  | {
      readonly kind: "retry";
      readonly waitMs: number | undefined;
      readonly raw?: RawResponse;
      readonly error?: InOrbitError;
    };

/**
 * A client for one credential. Safe to share; it holds no per-call state.
 *
 * ```ts
 * const client = Client.fromEnv(); // INORBIT_TOKEN, or INORBIT_KEY_ID + _KEY_SECRET + _SCOPES
 * ```
 */
export class Client {
  readonly #base: URL;
  readonly #provider: TokenProvider;
  readonly #timeout: number;
  readonly #maxRetries: number;
  readonly #userAgent: string;
  readonly #hooks: readonly Hook[];
  readonly #fetch: typeof fetch;
  readonly #streams: StreamTransport;
  readonly #idle: number;
  #hub: SocketHub | undefined;

  /**
   * A client with `options`.
   *
   * @throws {ConfigError} when no credential is given, a key has no scopes, or a URL is
   *   not https (plain http only to this machine).
   */
  constructor(options: ClientOptions) {
    this.#fetch = options.fetch ?? globalThis.fetch.bind(globalThis);
    this.#base = checkUrl("the base URL", options.baseUrl ?? DEFAULT_BASE_URL, true);
    const tokenUrl = checkUrl("the token URL", options.tokenUrl ?? DEFAULT_TOKEN_URL, false);
    if (options.tokenProvider !== undefined) {
      this.#provider = options.tokenProvider;
    } else if (options.token !== undefined && options.token !== "") {
      this.#provider = new StaticToken(options.token);
    } else if (options.keyId !== undefined && options.keySecret !== undefined) {
      if (options.scopes === undefined || options.scopes.length === 0) {
        throw new ConfigError(
          'no scopes: set INORBIT_SCOPES (space-separated, such as "identity:read account:read")',
        );
      }
      this.#provider = new ClientCredentials({
        keyId: options.keyId,
        keySecret: options.keySecret,
        scopes: options.scopes,
        tokenUrl: tokenUrl.href,
        fetch: this.#fetch,
      });
    } else {
      throw new ConfigError(
        "no credentials: set INORBIT_TOKEN, or INORBIT_KEY_ID and INORBIT_KEY_SECRET",
      );
    }
    this.#timeout = options.timeout ?? 30_000;
    this.#maxRetries = options.maxRetries ?? 2;
    this.#userAgent = userAgent(options.userAgentSuffix);
    this.#hooks = options.hooks ?? [];
    this.#streams = options.streams ?? "sse";
    if (this.#streams !== "sse" && this.#streams !== "socket") {
      throw new ConfigError(`streams is "sse" or "socket", not ${JSON.stringify(this.#streams)}`);
    }
    this.#idle = options.streamIdleTimeout ?? 45_000;
  }

  /**
   * A client from the environment: `INORBIT_<PROFILE>_TOKEN`, or `INORBIT_<PROFILE>_KEY_ID`,
   * `_KEY_SECRET` and `_SCOPES`, and nothing else for a named profile; the bare
   * `INORBIT_*` names without one. `INORBIT_BASE_URL` and `INORBIT_TOKEN_URL` apply to
   * every profile.
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
    const outcome = await this.#call(op, options, false);
    if (outcome.kind !== "done") {
      throw new ConfigError("a stream's answer was read as a call's");
    }
    return outcome.raw;
  }

  /**
   * Opens the stream `op` and yields each event, typed, until the server ends it
   * (design.md section 7). Opening follows the rules of a `GET`; the first `next()` opens
   * it, so an opening error is thrown there. Stopping the loop, or aborting
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
    const outcome = await this.#call(op, options, true);
    if (outcome.kind !== "open") {
      throw new ConfigError("a call's answer was read as a stream's");
    }
    const { resp, abort, attempt } = outcome;
    const body = resp.body;
    if (body === null) {
      return;
    }
    const reader = body.getReader();
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
                requestId: attempt.requestId,
                attempts: attempt.number,
              }),
            );
          }
          if (event.event === "error") {
            throw envelopeError(value, attempt.requestId);
          }
          if (event.event === "message") {
            yield typed(value);
          }
        }
      }
    } finally {
      abort.abort();
      reader.cancel().catch(() => undefined);
    }
  }

  #socketHub(): SocketHub {
    const url = new URL("/v1/ws", this.#base);
    url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
    const provider = this.#provider;
    return new SocketHub({
      url: url.href,
      host: this.#base.host,
      token: async () => (await provider.token()).access,
      invalidate: async () => {
        await provider.invalidate?.();
      },
      maxRetries: this.#maxRetries,
    });
  }

  async #call(op: Operation, options: CallOptions | undefined, stream: boolean): Promise<Outcome> {
    const url = this.#url(op);
    const id = requestId();
    const retrySafe = op.idempotent === true || IDEMPOTENT.has(op.method);
    let retries = 0;
    let refreshed = false;
    for (let number = 1; ; number++) {
      const attempt: Attempt = {
        operation: op.name ?? op.path,
        method: op.method,
        path: op.path,
        number,
        requestId: id,
      };
      let outcome: Outcome;
      try {
        outcome = await this.#attempt(op, url, attempt, options, stream);
      } catch (e) {
        this.#failed(attempt, e);
        throw e;
      }
      if (outcome.kind === "done" || outcome.kind === "open") {
        return outcome;
      }
      if (outcome.kind === "unauthorized" && !refreshed) {
        await this.#provider.invalidate?.();
        refreshed = true;
        continue;
      }
      if (outcome.kind === "retry" && retrySafe && retries < this.#maxRetries) {
        await sleep(outcome.waitMs ?? backoffMs(retries), options?.signal);
        retries++;
        continue;
      }
      const error =
        outcome.raw === undefined
          ? (outcome as { error: InOrbitError }).error
          : new ApiError(outcome.raw);
      this.#failed(attempt, error);
      throw error;
    }
  }

  #failed(attempt: Attempt, e: unknown): void {
    for (const h of this.#hooks) {
      h.onError?.(attempt, e as InOrbitError);
    }
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

  async #attempt(
    op: Operation,
    url: URL,
    attempt: Attempt,
    options: CallOptions | undefined,
    stream: boolean,
  ): Promise<Outcome> {
    const token = await this.#provider.token();
    const headers: Record<string, string> = {
      authorization: `Bearer ${token.access}`,
      accept: stream ? "text/event-stream" : "application/json",
      "user-agent": this.#userAgent,
      "x-request-id": attempt.requestId,
    };
    let body: string | undefined;
    if (op.body !== undefined) {
      headers["content-type"] = "application/json";
      body = JSON.stringify(op.body, (_, v: unknown) => (typeof v === "bigint" ? v.toString() : v));
    }
    for (const h of this.#hooks) {
      h.onRequest?.(attempt);
    }
    const timeout = options?.timeout ?? this.#timeout;
    // A stream's body outlives the attempt's clock, so the clock aborts the headers only.
    const abort = new AbortController();
    const clock = setTimeout(() => abort.abort(new Error("timeout")), timeout);
    const timer = abort.signal;
    const signal = options?.signal === undefined ? timer : AbortSignal.any([timer, options.signal]);
    let resp: globalThis.Response;
    try {
      const init: RequestInit = { method: op.method, headers, redirect: "manual", signal };
      resp = await this.#fetch(url, body === undefined ? init : { ...init, body });
    } catch (e) {
      clearTimeout(clock);
      if (options?.signal?.aborted) {
        throw options.signal.reason;
      }
      const error = timer.aborted
        ? new TimeoutError(this.#base.host, Math.round(timeout / 1000))
        : new ConnectionError(this.#base.host, describe(e), { cause: e });
      return { kind: "retry", waitMs: undefined, error };
    }
    if (stream && resp.status >= 200 && resp.status < 300) {
      clearTimeout(clock);
      return { kind: "open", resp, abort, attempt };
    }
    const length = Number(resp.headers.get("content-length") ?? "0");
    if (length > MAX_BODY) {
      clearTimeout(clock);
      throw new TooLargeError();
    }
    let bytes: Uint8Array;
    try {
      bytes = new Uint8Array(await resp.arrayBuffer());
      clearTimeout(clock);
    } catch (e) {
      clearTimeout(clock);
      if (options?.signal?.aborted) {
        throw options.signal.reason;
      }
      return {
        kind: "retry",
        waitMs: undefined,
        error: new ConnectionError(this.#base.host, describe(e), { cause: e }),
      };
    }
    if (bytes.length > MAX_BODY) {
      throw new TooLargeError();
    }
    const raw = new RawResponse({
      status: resp.status,
      headers: resp.headers,
      body: bytes,
      requestId: attempt.requestId,
      attempts: attempt.number,
    });
    for (const h of this.#hooks) {
      h.onResponse?.(attempt, raw);
    }
    if (raw.status === 401) {
      return { kind: "unauthorized", raw };
    }
    if (retryableStatus(raw.status)) {
      return { kind: "retry", waitMs: retryAfterMs(raw.headers), raw };
    }
    if (raw.status >= 200 && raw.status < 300) {
      return { kind: "done", raw };
    }
    throw new ApiError(raw);
  }
}
