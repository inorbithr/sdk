/**
 * The middleware pipeline (config.md section 7): the ordered, named list every call goes
 * through, with a per-call stage outside `retry` and a per-retry stage inside it.
 *
 * ```ts
 * const probe: Middleware = {
 *   name: "probe",
 *   async handle(request, next) {
 *     request.headers.set("x-team", "payments");
 *     return next(request);
 *   },
 * };
 * const client = Client.load({ pipeline: (p) => p.addPerRetry(probe).remove("rate_limit") });
 * ```
 *
 * @module
 */

import { ConfigError } from "./errors.js";
import type { RateLimit } from "./ratelimit.js";
import { BUILT_INS } from "./settings.js";

/** Where a middleware runs: once per call, or on every attempt. */
export type Stage = "per_call" | "per_retry";

/** What a middleware may read about the call (config.md section 7.13). */
export interface CallInfo {
  /** The operation (`radar.list_digests`), or the path for a raw call. */
  readonly operation: string;
  /** The path template (`/v1/radar/digests/{digest_id}`), when the surface knows it. */
  readonly template: string | undefined;
  /** Whether `retry` may repeat the call. */
  readonly idempotent: boolean;
  /** The `Idempotency-Key` the call is sent with, once `idempotency_key` set it. */
  readonly idempotencyKey: string | undefined;
  /** The call's `x-request-id`, once `request_id` set it. */
  readonly requestId: string | undefined;
  /** The attempt, 1-based; 0 in the per-call stage. */
  readonly attempt: number;
  /** When the call must end, in milliseconds since the epoch, once `deadline` set it. */
  readonly deadline: number | undefined;
  /** Whether the answer is a stream, whose body a middleware must not read. */
  readonly stream: boolean;
  /** The configuration profile the client was loaded for. */
  readonly profile: string | undefined;
  /** The stage the middleware runs in. */
  readonly stage: Stage;
}

/** A request on its way through the pipeline. Its headers are a fresh copy per attempt. */
export interface SdkRequest {
  /** The HTTP method. */
  readonly method: string;
  /** The full URL, query included. */
  readonly url: URL;
  /** The headers; a middleware may change them. */
  readonly headers: Headers;
  /** The body as bytes; `undefined` when there is none. */
  readonly body: Uint8Array | undefined;
  /** What the call is. */
  readonly info: CallInfo;
  /** Aborts the request: the caller's signal, the deadline and the attempt's timeout. */
  readonly signal: AbortSignal | undefined;
}

/** An answer on its way back. */
export interface SdkResponse {
  /** The HTTP status. */
  readonly status: number;
  /** The response headers. */
  readonly headers: Headers;
  /** The body, read whole; `undefined` for a stream. */
  readonly body: Uint8Array | undefined;
  /** A stream's body, which a middleware must not read; `undefined` otherwise. */
  readonly stream: ReadableStream<Uint8Array> | undefined;
  /** The request as it was sent, after every middleware. */
  readonly request: SdkRequest;
  /** The rate-limit snapshot, once `rate_limit` read it. */
  readonly rateLimit?: RateLimit | undefined;
}

/** The rest of the pipeline. */
export type Next = (request: SdkRequest) => Promise<SdkResponse>;

/**
 * One named step. It may change the request, call `next` zero times (answering itself),
 * once, or more (each call of `next` from the per-call stage is a fresh attempt), and
 * inspect what comes back. It must not log secrets or bodies.
 */
export interface Middleware {
  /** Its name, unique in the pipeline. */
  readonly name: string;
  /** Handles `request`, calling `next` for the rest of the pipeline. */
  handle(request: SdkRequest, next: Next): Promise<SdkResponse>;
}

const KEPT = ["retry", "auth", "timeout"];

/** One position of the pipeline: the name it holds and the middleware there. */
interface Slot {
  readonly name: string;
  readonly middleware: Middleware;
}

/**
 * The client's pipeline, edited by name at construction: the built-ins of config.md
 * section 7.2, outermost first, and what you add. Settings switch built-ins off without
 * removing them (`rateLimit: "off"`, `tracing: false`, `log: "off"`).
 */
export class Pipeline {
  #slots: Slot[];

  /** The built-in pipeline; `builtIns` holds a middleware for each built-in name. */
  constructor(builtIns: ReadonlyMap<string, Middleware>) {
    this.#slots = BUILT_INS.map((name) => {
      const m = builtIns.get(name);
      if (m === undefined) {
        throw new ConfigError(`the built-in middleware ${name} is missing`);
      }
      return { name, middleware: m };
    });
  }

  #index(name: string): number {
    const i = this.#slots.findIndex((s) => s.name === name);
    if (i < 0) {
      throw new ConfigError(
        `the pipeline has no middleware named ${JSON.stringify(name)}; it has ${this.names().join(", ")}`,
      );
    }
    return i;
  }

  #insert(at: number, m: Middleware): this {
    if (typeof m?.name !== "string" || m.name === "" || typeof m.handle !== "function") {
      throw new ConfigError("a middleware needs a name and a handle function");
    }
    if (this.#slots.some((s) => s.name === m.name)) {
      throw new ConfigError(`the pipeline already has a middleware named ${JSON.stringify(m.name)}`);
    }
    this.#slots.splice(at, 0, { name: m.name, middleware: m });
    return this;
  }

  /** Adds `m` once per call, just before `retry`, after earlier additions. */
  addPerCall(m: Middleware): this {
    return this.#insert(this.#index("retry"), m);
  }

  /** Adds `m` on every attempt, just before `timeout`, after earlier additions. */
  addPerRetry(m: Middleware): this {
    return this.#insert(this.#index("timeout"), m);
  }

  /** Adds `m` just before the middleware named `name`. */
  insertBefore(name: string, m: Middleware): this {
    return this.#insert(this.#index(name), m);
  }

  /** Adds `m` just after the middleware named `name`. */
  insertAfter(name: string, m: Middleware): this {
    return this.#insert(this.#index(name) + 1, m);
  }

  /** Swaps the middleware named `name` for `m`, which takes its name and place. */
  replace(name: string, m: Middleware): this {
    if (typeof m?.handle !== "function") {
      throw new ConfigError("a middleware needs a handle function");
    }
    this.#slots[this.#index(name)] = { name, middleware: m };
    return this;
  }

  /** Drops the middleware named `name`; `retry`, `auth` and `timeout` can be replaced only. */
  remove(name: string): this {
    if (KEPT.includes(name)) {
      throw new ConfigError(
        `${name} cannot be removed, only replaced: ${
          name === "retry"
            ? "set maxRetries: 0 to retry nothing"
            : name === "auth"
              ? "without it no credential is sent"
              : "without it an attempt could wait forever"
        }`,
      );
    }
    this.#slots.splice(this.#index(name), 1);
    return this;
  }

  /** The names, outermost first. */
  names(): string[] {
    return this.#slots.map((s) => s.name);
  }

  /** Runs `request` through every middleware, then `transport`. */
  run(request: SdkRequest, transport: Next): Promise<SdkResponse> {
    const slots = [...this.#slots];
    const retryAt = slots.findIndex((s) => s.name === "retry");
    const step = (i: number, req: SdkRequest): Promise<SdkResponse> => {
      const slot = slots[i];
      if (slot === undefined) {
        return transport(req);
      }
      const stage: Stage = retryAt >= 0 && i > retryAt ? "per_retry" : "per_call";
      const seen = req.info.stage === stage ? req : { ...req, info: { ...req.info, stage } };
      return slot.middleware.handle(seen, (r) => step(i + 1, r));
    };
    return step(0, request);
  }
}
