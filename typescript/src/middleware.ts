/**
 * The built-in middlewares of config.md section 7.2 and the transport after them. The
 * client builds one set per client; nothing here is exported from the package.
 *
 * @module
 */

import { describe, type TokenProvider } from "./auth.js";
import {
  AuthError,
  ConnectionError,
  InOrbitError,
  RawResponse,
  TimeoutError,
  TooLargeError,
} from "./errors.js";
import type { Attempt, Hook } from "./hooks.js";
import type { Middleware, Next, SdkRequest, SdkResponse } from "./pipeline.js";
import { type RateLimit, readRateLimit } from "./ratelimit.js";
import { backoffMs, requestId, retryableStatus, retryAfter, sleep } from "./retry.js";
import {
  type Log,
  logPath,
  parseTraceparent,
  redactedUrl,
  SPAN_KIND,
  type SpanContextLike,
  STATUS_ERROR,
  type Telemetry,
  traceparent,
} from "./telemetry.js";

/** The largest answer read, 16 MiB. */
export const MAX_BODY: number = 16 * 1024 * 1024;

/** What one call carries between middlewares, out of the user's sight. */
export interface CallState {
  /** Whether `auth` already fetched a fresh token after a 401. */
  refreshed: boolean;
  /** The per-call timeout, in milliseconds. */
  readonly timeout: number | undefined;
  /** The per-call `traceparent`. */
  readonly traceparent: string | undefined;
  /** The call span's context, once `call_tracing` started it. */
  callContext: unknown;
  /** The attempt span's ids, for log records. */
  span: SpanContextLike | undefined;
  /** The attempts made so far. */
  attempts: number;
}

/** Where a request keeps its call's state; spreading a request keeps it. */
export const CALL: unique symbol = Symbol("inorbithr.call");

/** A request with its call's state. */
export type StatefulRequest = SdkRequest & { [CALL]?: CallState };

/** The state of `request`'s call, or a fresh one for a request a user rebuilt. */
export function callState(request: SdkRequest): CallState {
  const r = request as StatefulRequest;
  if (r[CALL] === undefined) {
    (r as { [CALL]?: CallState })[CALL] = newCallState(undefined, undefined);
  }
  return r[CALL] as CallState;
}

/** A fresh call state. */
export function newCallState(
  timeout: number | undefined,
  traceparentValue: string | undefined,
): CallState {
  return {
    refreshed: false,
    timeout,
    traceparent: traceparentValue,
    callContext: undefined,
    span: undefined,
    attempts: 0,
  };
}

/** A token bucket for retries, after the AWS standard retry mode (config.md section 7.4). */
export class RetryBudget {
  readonly #capacity: number;
  #tokens: number;

  /** A full bucket of `capacity`. */
  constructor(capacity: number) {
    this.#capacity = capacity;
    this.#tokens = capacity;
  }

  /** Pays `cost` when the bucket can. */
  take(cost: number): boolean {
    if (this.#tokens < cost) {
      return false;
    }
    this.#tokens -= cost;
    return true;
  }

  /** Puts `amount` back, up to the capacity. */
  give(amount: number): void {
    this.#tokens = Math.min(this.#capacity, this.#tokens + amount);
  }

  /** What is left. */
  get tokens(): number {
    return this.#tokens;
  }
}

/** What the built-ins share: one client's settings and state. */
export interface Context {
  readonly host: string;
  readonly provider: TokenProvider;
  /** A refused static token from `load` ends the call with an {@link AuthError}. */
  readonly staticToken: boolean;
  readonly userAgent: string;
  readonly timeout: number;
  readonly totalTimeout: number;
  readonly maxRetries: number;
  readonly retryBaseDelay: number;
  readonly retryMaxDelay: number;
  readonly retryAfterMax: number;
  readonly budget: RetryBudget | undefined;
  readonly rateLimit: "observe" | "wait" | "off";
  readonly log: Log;
  readonly hooks: readonly Hook[];
  telemetry(): Promise<Telemetry | undefined>;
  /** The latest rate-limit snapshot, and when its window resets. */
  latest: { readonly snapshot: RateLimit; readonly resetAt: number | undefined } | undefined;
}

function annotate(e: unknown, fields: { requestId?: string; idempotencyKey?: string }): void {
  if (e instanceof InOrbitError) {
    if (fields.requestId !== undefined) {
      e.requestId ??= fields.requestId;
    }
    if (fields.idempotencyKey !== undefined) {
      e.idempotencyKey ??= fields.idempotencyKey;
    }
  }
}

function seconds(ms: number): number {
  return Math.max(1, Math.round(ms / 1000));
}

/** An attempt as hooks see it. */
export function attemptOf(request: SdkRequest): Attempt {
  return {
    operation: request.info.operation,
    method: request.method,
    path: logPath(request.url),
    number: Math.max(1, request.info.attempt),
    requestId: request.info.requestId ?? "",
    idempotencyKey: request.info.idempotencyKey,
    stage: request.info.stage,
  };
}

/** The raw answer a response stands for. */
export function rawOf(response: SdkResponse): RawResponse {
  return new RawResponse({
    status: response.status,
    headers: response.headers,
    body: response.body ?? new Uint8Array(),
    requestId: response.request.info.requestId ?? "",
    attempts: Math.max(1, response.request.info.attempt),
    idempotencyKey: response.request.info.idempotencyKey,
    rateLimit: response.rateLimit,
  });
}

/** The wait a retry-worthy answer asks for: `Retry-After`, else the envelope's `retry` detail. */
function askedWait(response: SdkResponse): number | undefined {
  const header = retryAfter(response.headers);
  if (header !== undefined) {
    return header;
  }
  try {
    const body = JSON.parse(new TextDecoder().decode(response.body ?? new Uint8Array())) as {
      details?: { type?: unknown; after_seconds?: unknown }[];
    };
    const d = body.details?.find((x) => x.type === "retry");
    const s = Number(d?.after_seconds);
    return d !== undefined && Number.isFinite(s) && s >= 0 ? s * 1000 : undefined;
  } catch {
    return undefined;
  }
}

/** Every built-in of one client, by name. */
export function builtIns(ctx: Context): Map<string, Middleware> {
  const m = (name: string, handle: Middleware["handle"]): [string, Middleware] => [
    name,
    { name, handle },
  ];
  return new Map<string, Middleware>([
    m("request_id", async (req, next) => {
      const id = requestId();
      req.headers.set("x-request-id", id);
      try {
        return await next({ ...req, info: { ...req.info, requestId: id } });
      } catch (e) {
        annotate(e, { requestId: id });
        throw e;
      }
    }),
    m("user_agent", (req, next) => {
      req.headers.set("user-agent", ctx.userAgent);
      return next(req);
    }),
    m("idempotency_key", async (req, next) => {
      const op = (req as StatefulRequest & { [OPERATION]?: OperationMarks })[OPERATION];
      if (op?.idempotencyKey !== true) {
        return next(req);
      }
      const key = op.key ?? crypto.randomUUID();
      req.headers.set("idempotency-key", key);
      try {
        return await next({
          ...req,
          info: { ...req.info, idempotencyKey: key, idempotent: true },
        });
      } catch (e) {
        annotate(e, { idempotencyKey: key });
        throw e;
      }
    }),
    m("call_tracing", (req, next) => callTracing(ctx, req, next)),
    m("deadline", async (req, next) => {
      const st = callState(req);
      const total = Math.min(ctx.totalTimeout, st.timeout ?? Number.POSITIVE_INFINITY);
      const deadline = Date.now() + total;
      const abort = new AbortController();
      const timer = setTimeout(
        () => abort.abort(new TimeoutError(ctx.host, seconds(total))),
        total,
      );
      const signal =
        req.signal === undefined ? abort.signal : AbortSignal.any([req.signal, abort.signal]);
      try {
        return await next({ ...req, signal, info: { ...req.info, deadline } });
      } finally {
        clearTimeout(timer);
      }
    }),
    m("retry", (req, next) => retry(ctx, req, next)),
    m("auth", async (req, next) => {
      const send = async (r: SdkRequest): Promise<SdkResponse> => {
        const token = await ctx.provider.token();
        r.headers.set("authorization", `Bearer ${token.access}`);
        return next(r);
      };
      const resp = await send(req);
      const st = callState(req);
      if (resp.status !== 401 || st.refreshed) {
        return resp;
      }
      st.refreshed = true;
      await resp.stream?.cancel().catch(() => undefined);
      if (ctx.staticToken) {
        const error = new AuthError(
          "the API refused the token (HTTP 401): it has expired or was revoked; create a new one in the console or with `iohr token create`, and set it again",
          "unauthenticated",
        );
        annotate(error, {
          ...(req.info.requestId === undefined ? {} : { requestId: req.info.requestId }),
        });
        throw error;
      }
      await ctx.provider.invalidate?.();
      return send({ ...req, headers: new Headers(req.headers) });
    }),
    m("rate_limit", async (req, next) => {
      if (ctx.rateLimit === "off") {
        return next(req);
      }
      if (ctx.rateLimit === "wait") {
        const latest = ctx.latest;
        const now = Date.now();
        if (
          latest?.snapshot.remaining === 0 &&
          latest.resetAt !== undefined &&
          latest.resetAt > now
        ) {
          const wait = latest.resetAt - now + Math.floor(Math.random() * 101);
          const deadline = req.info.deadline;
          if (deadline !== undefined && now + wait > deadline) {
            throw new TimeoutError(
              ctx.host,
              seconds(ctx.totalTimeout),
              "the rate-limit window to reset",
            );
          }
          ctx.log.emit("warn", {
            event: "rate_limit_wait",
            attempt: Math.max(1, req.info.attempt),
            reason: "rate_limit",
            delay_ms: wait,
            request_id: req.info.requestId,
          });
          await sleep(wait, req.signal);
        }
      }
      const resp = await next(req);
      const snapshot = readRateLimit(resp.headers);
      if (snapshot === undefined) {
        return resp;
      }
      ctx.latest = {
        snapshot,
        resetAt: snapshot.reset === undefined ? undefined : Date.now() + snapshot.reset,
      };
      return { ...resp, rateLimit: snapshot };
    }),
    m("attempt_tracing", (req, next) => attemptTracing(ctx, req, next)),
    m("logging", async (req, next) => {
      if (!ctx.log.on("debug")) {
        return next(req);
      }
      const st = callState(req);
      const trace =
        st.span === undefined ? {} : { trace_id: st.span.traceId, span_id: st.span.spanId };
      const base = {
        operation: req.info.operation,
        method: req.method,
        path: logPath(req.url),
        attempt: Math.max(1, req.info.attempt),
        request_id: req.info.requestId,
        ...trace,
      };
      ctx.log.emit("debug", {
        event: "request",
        ...base,
        ...(ctx.log.headers ? { headers: ctx.log.show(req.headers, false) } : {}),
      });
      const started = Date.now();
      const resp = await next(req);
      ctx.log.emit("debug", {
        event: "response",
        ...base,
        status: resp.status,
        duration_ms: Date.now() - started,
        server_request_id: resp.headers.get("x-request-id") ?? undefined,
        ...(ctx.log.headers ? { headers: ctx.log.show(resp.headers, true) } : {}),
      });
      return resp;
    }),
    m("hooks", async (req, next) => {
      if (ctx.hooks.length === 0) {
        return next(req);
      }
      const attempt = attemptOf(req);
      for (const h of ctx.hooks) {
        h.onRequest?.(attempt);
      }
      const resp = await next(req);
      if (ctx.hooks.some((h) => h.onResponse !== undefined)) {
        const raw = rawOf(resp);
        for (const h of ctx.hooks) {
          h.onResponse?.(attempt, raw);
        }
      }
      return resp;
    }),
    m("timeout", async (req, next) => {
      const ms = callState(req).timeout ?? ctx.timeout;
      const deadline = req.info.deadline;
      const limit = deadline === undefined ? ms : Math.max(0, Math.min(ms, deadline - Date.now()));
      const abort = new AbortController();
      const timer = setTimeout(() => abort.abort(new TimeoutError(ctx.host, seconds(ms))), limit);
      const signal =
        req.signal === undefined ? abort.signal : AbortSignal.any([req.signal, abort.signal]);
      try {
        return await next({ ...req, signal });
      } finally {
        clearTimeout(timer);
      }
    }),
  ]);
}

/** What the client tells `idempotency_key` about the operation. */
export interface OperationMarks {
  readonly idempotencyKey: boolean;
  readonly key: string | undefined;
}

/** Where a request keeps its operation's marks. */
export const OPERATION: unique symbol = Symbol("inorbithr.operation");

async function retry(ctx: Context, req: SdkRequest, next: Next): Promise<SdkResponse> {
  const st = callState(req);
  let retries = 0;
  let lastCost = 0;
  for (let attempt = 1; ; attempt++) {
    st.attempts = attempt;
    const areq: SdkRequest = {
      ...req,
      headers: new Headers(req.headers),
      info: { ...req.info, attempt, stage: "per_retry" },
    };
    let resp: SdkResponse | undefined;
    let error: unknown;
    try {
      resp = await next(areq);
    } catch (e) {
      error = e;
    }
    let reason: string;
    let wait: number;
    let cost = 10;
    if (resp !== undefined) {
      if (!retryableStatus(resp.status) || !req.info.idempotent) {
        if (resp.status >= 200 && resp.status < 300) {
          ctx.budget?.give(retries === 0 ? 1 : lastCost);
        }
        return resp;
      }
      reason = String(resp.status);
      const asked = askedWait(resp);
      if (asked !== undefined && asked > ctx.retryAfterMax) {
        return resp;
      }
      if (resp.status === 429 || (resp.status === 503 && asked !== undefined)) {
        cost = 5;
      }
      wait = asked ?? backoffMs(retries, ctx.retryBaseDelay, ctx.retryMaxDelay);
    } else {
      const retriable =
        (error instanceof ConnectionError || error instanceof TimeoutError) &&
        req.info.idempotent &&
        req.signal?.aborted !== true;
      if (!retriable) {
        throw error;
      }
      reason = (error as InOrbitError).kind;
      wait = backoffMs(retries, ctx.retryBaseDelay, ctx.retryMaxDelay);
    }
    const end = (): SdkResponse => {
      if (resp !== undefined) {
        return resp;
      }
      throw error;
    };
    if (retries >= ctx.maxRetries) {
      return end();
    }
    const deadline = req.info.deadline;
    if (deadline !== undefined && Date.now() + wait > deadline) {
      return end();
    }
    if (ctx.budget !== undefined && !ctx.budget.take(cost)) {
      return end();
    }
    lastCost = cost;
    const hooked = attemptOf(areq);
    for (const h of ctx.hooks) {
      h.onRetry?.(hooked, reason, wait);
    }
    ctx.log.emit("warn", {
      event: "retry",
      operation: req.info.operation,
      attempt,
      reason,
      delay_ms: wait,
      request_id: req.info.requestId,
    });
    void ctx.telemetry().then((t) =>
      t?.retries?.add?.(1, { "inorbit.operation": req.info.operation, "inorbit.retry.reason": reason }),
    );
    await resp?.stream?.cancel().catch(() => undefined);
    await sleep(wait, req.signal);
    retries++;
  }
}

function errorType(e: unknown): string {
  if (e instanceof InOrbitError) {
    const code = (e as { code?: unknown }).code;
    return typeof code === "string" ? code : e.kind;
  }
  return e instanceof Error ? e.name : "error";
}

/** Ends `span` when `stream` ends, however it ends. */
function endWith(stream: ReadableStream<Uint8Array>, done: () => void): ReadableStream<Uint8Array> {
  const reader = stream.getReader();
  let ended = false;
  const finish = (): void => {
    if (!ended) {
      ended = true;
      done();
    }
  };
  return new ReadableStream<Uint8Array>({
    async pull(c) {
      try {
        const r = await reader.read();
        if (r.done) {
          finish();
          c.close();
        } else {
          c.enqueue(r.value);
        }
      } catch (e) {
        finish();
        c.error(e);
      }
    },
    async cancel(reason) {
      finish();
      await reader.cancel(reason);
    },
  });
}

async function callTracing(ctx: Context, req: SdkRequest, next: Next): Promise<SdkResponse> {
  const t = await ctx.telemetry();
  if (t === undefined) {
    return next(req);
  }
  const st = callState(req);
  const started = performance.now();
  const record = (error?: string): void => {
    t.callDuration?.record?.((performance.now() - started) / 1000, {
      "inorbit.operation": req.info.operation,
      ...(error === undefined ? {} : { "error.type": error }),
    });
  };
  if (t.tracer === undefined) {
    try {
      const resp = await next(req);
      record(resp.status >= 400 ? String(resp.status) : undefined);
      return resp;
    } catch (e) {
      record(errorType(e));
      throw e;
    }
  }
  let parent = t.api.context.active();
  const remote = st.traceparent === undefined ? undefined : parseTraceparent(st.traceparent);
  if (remote !== undefined) {
    parent = t.api.trace.setSpanContext(parent, remote);
  }
  const attributes: Record<string, string> = { "inorbit.operation": req.info.operation };
  if (req.info.requestId !== undefined) {
    attributes["inorbit.request_id"] = req.info.requestId;
  }
  const span = t.tracer.startSpan(
    req.info.operation,
    { kind: SPAN_KIND.internal, attributes },
    parent,
  );
  st.callContext = t.api.trace.setSpan(parent, span);
  const fail = (type: string): void => {
    span.setAttribute("error.type", type);
    span.setStatus({ code: STATUS_ERROR });
  };
  try {
    const resp = await next(req);
    if (resp.status >= 400) {
      fail(String(resp.status));
    }
    if (resp.stream !== undefined) {
      return {
        ...resp,
        stream: endWith(resp.stream, () => {
          record();
          span.end();
        }),
      };
    }
    record(resp.status >= 400 ? String(resp.status) : undefined);
    span.end();
    return resp;
  } catch (e) {
    fail(errorType(e));
    record(errorType(e));
    span.end();
    throw e;
  }
}

async function attemptTracing(ctx: Context, req: SdkRequest, next: Next): Promise<SdkResponse> {
  const t = await ctx.telemetry();
  if (t === undefined) {
    return next(req);
  }
  const st = callState(req);
  const port = req.url.port === "" ? (req.url.protocol === "http:" ? 80 : 443) : Number(req.url.port);
  const common = {
    "http.request.method": req.method,
    "server.address": req.url.hostname.replace(/^\[|\]$/g, ""),
    "server.port": port,
  };
  const started = performance.now();
  const measure = (status: number | undefined, error: string | undefined): void => {
    t.requestDuration?.record?.((performance.now() - started) / 1000, {
      ...common,
      ...(status === undefined ? {} : { "http.response.status_code": status }),
      ...(error === undefined ? {} : { "error.type": error }),
    });
  };
  if (t.tracer === undefined) {
    try {
      const resp = await next(req);
      measure(resp.status, resp.status >= 400 ? String(resp.status) : undefined);
      return resp;
    } catch (e) {
      measure(undefined, errorType(e));
      throw e;
    }
  }
  const template = req.info.template;
  const attributes: Record<string, string | number | boolean> = {
    ...common,
    "url.full": redactedUrl(req.url),
  };
  if (template !== undefined) {
    attributes["url.template"] = template;
  }
  if (req.info.attempt > 1) {
    attributes["http.request.resend_count"] = req.info.attempt - 1;
  }
  const span = t.tracer.startSpan(
    template === undefined ? req.method : `${req.method} ${template}`,
    { kind: SPAN_KIND.client, attributes },
    st.callContext ?? t.api.context.active(),
  );
  const sc = span.spanContext();
  if (t.api.trace.isSpanContextValid(sc)) {
    st.span = sc;
    req.headers.set("traceparent", traceparent(sc));
    const state = sc.traceState?.serialize();
    if (state !== undefined && state !== "") {
      req.headers.set("tracestate", state);
    }
  }
  try {
    const resp = await next(req);
    span.setAttribute("http.response.status_code", resp.status);
    const server = resp.headers.get("x-request-id");
    if (server !== null) {
      span.setAttribute("inorbit.server_request_id", server);
    }
    if (resp.headers.get("idempotency-replayed")?.trim() === "true") {
      span.setAttribute("inorbit.idempotency_replayed", true);
    }
    if (resp.status >= 400) {
      span.setAttribute("error.type", String(resp.status));
      span.setStatus({ code: STATUS_ERROR });
    }
    measure(resp.status, resp.status >= 400 ? String(resp.status) : undefined);
    span.end();
    return resp;
  } catch (e) {
    span.setAttribute("error.type", errorType(e));
    span.setStatus({ code: STATUS_ERROR });
    measure(undefined, errorType(e));
    span.end();
    throw e;
  }
}

/** The innermost step: sends the request with `fetch` and reads the answer (16 MiB at most). */
export function transport(
  host: string,
  send: typeof fetch,
  dispatcher: unknown,
): (req: SdkRequest) => Promise<SdkResponse> {
  return async (req) => {
    const init: RequestInit & { dispatcher?: unknown } = {
      method: req.method,
      headers: req.headers,
      redirect: "manual",
    };
    if (req.signal !== undefined) {
      init.signal = req.signal;
    }
    if (req.body !== undefined) {
      init.body = req.body as BodyInit;
    }
    if (dispatcher !== undefined) {
      init.dispatcher = dispatcher;
    }
    let resp: Response;
    try {
      resp = await send(req.url, init);
    } catch (e) {
      if (req.signal?.aborted) {
        throw req.signal.reason;
      }
      throw new ConnectionError(host, describe(e), { cause: e });
    }
    const ok = resp.status >= 200 && resp.status < 300;
    if (req.info.stream && ok) {
      return {
        status: resp.status,
        headers: resp.headers,
        body: undefined,
        stream: resp.body ?? new ReadableStream({ start: (c) => c.close() }),
        request: req,
      };
    }
    const length = Number(resp.headers.get("content-length") ?? "0");
    if (length > MAX_BODY) {
      await resp.body?.cancel().catch(() => undefined);
      throw new TooLargeError();
    }
    let bytes: Uint8Array;
    try {
      bytes = new Uint8Array(await resp.arrayBuffer());
    } catch (e) {
      if (req.signal?.aborted) {
        throw req.signal.reason;
      }
      throw new ConnectionError(host, describe(e), { cause: e });
    }
    if (bytes.length > MAX_BODY) {
      throw new TooLargeError();
    }
    return { status: resp.status, headers: resp.headers, body: bytes, stream: undefined, request: req };
  };
}
