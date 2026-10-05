/**
 * Logging (config.md section 7.9) and OpenTelemetry (section 7.10). Logging is off until
 * `log` is set and goes to a `logger` or the console; records hold metadata only, and
 * headers only from the allowlist. OpenTelemetry is an optional peer dependency
 * (`@opentelemetry/api`): used when installed, or when passed as `opentelemetry`.
 *
 * @module
 */

import { SDK_VERSION } from "./version.js";

/** How much is logged. */
export type LogLevel = "off" | "error" | "warn" | "info" | "debug";

/** One log record: an `event` and the fields of config.md section 7.9. */
export interface LogRecord {
  /** `request`, `response`, `call`, `retry`, `token_refresh_failed`, `rate_limit_wait` or `call_failed`. */
  readonly event: string;
  /** The other fields. */
  readonly [field: string]: unknown;
}

/** Where records go: one method per level, each given a short message and the record. */
export interface Logger {
  debug(message: string, record: LogRecord): void;
  info(message: string, record: LogRecord): void;
  warn(message: string, record: LogRecord): void;
  error(message: string, record: LogRecord): void;
}

/** Sees every record last: returns it changed, or `undefined` to drop it (SR-14). */
export type Redact = (record: LogRecord) => LogRecord | undefined;

const LEVELS: readonly LogLevel[] = ["off", "error", "warn", "info", "debug"];

/** Headers never logged, whatever the settings. */
export const NEVER_LOGGED: readonly string[] = [
  "authorization",
  "proxy-authorization",
  "cookie",
  "set-cookie",
];

/** Request headers whose values are logged with `logHeaders`. */
export const REQUEST_ALLOWLIST: readonly string[] = [
  "accept",
  "content-type",
  "content-length",
  "user-agent",
  "x-request-id",
  "traceparent",
  "idempotency-key",
];

/** Response headers whose values are logged with `logHeaders`. */
export const RESPONSE_ALLOWLIST: readonly string[] = [
  "content-type",
  "content-length",
  "date",
  "retry-after",
  "x-request-id",
  "idempotency-replayed",
  "x-ratelimit-limit",
  "x-ratelimit-remaining",
  "x-ratelimit-reset",
  "ratelimit",
  "ratelimit-policy",
];

/** The log sink of one client. */
export class Log {
  readonly #level: number;
  readonly #logger: Logger | undefined;
  readonly #redact: Redact | undefined;
  readonly #profile: string | undefined;
  readonly #request: ReadonlySet<string>;
  readonly #response: ReadonlySet<string>;
  /** Whether header values from the allowlist are logged. */
  readonly headers: boolean;

  /** A sink at `level`. */
  constructor(options: {
    level: LogLevel;
    logger?: Logger | undefined;
    redact?: Redact | undefined;
    profile?: string | undefined;
    headers?: boolean;
    allow?: readonly string[] | undefined;
  }) {
    this.#level = LEVELS.indexOf(options.level);
    this.#logger = options.logger;
    this.#redact = options.redact;
    this.#profile = options.profile;
    this.headers = options.headers === true;
    const extra = (options.allow ?? [])
      .map((h) => h.toLowerCase())
      .filter((h) => !NEVER_LOGGED.includes(h));
    this.#request = new Set([...REQUEST_ALLOWLIST, ...extra]);
    this.#response = new Set([...RESPONSE_ALLOWLIST, ...extra]);
  }

  /** Whether records at `level` are kept. */
  on(level: Exclude<LogLevel, "off">): boolean {
    return this.#level >= LEVELS.indexOf(level);
  }

  /** The headers as a record may show them: allowlisted values, every other name `REDACTED`. */
  show(headers: Headers, response: boolean): Record<string, string> {
    const allow = response ? this.#response : this.#request;
    const out: Record<string, string> = {};
    headers.forEach((value, name) => {
      out[name] = allow.has(name) && !NEVER_LOGGED.includes(name) ? value : "REDACTED";
    });
    return out;
  }

  /** Emits `record` at `level` when the level is on. */
  emit(level: Exclude<LogLevel, "off">, record: LogRecord): void {
    if (!this.on(level)) {
      return;
    }
    let r: LogRecord | undefined =
      this.#profile === undefined ? record : { ...record, profile: this.#profile };
    if (this.#redact !== undefined) {
      try {
        r = this.#redact(r);
      } catch {
        return;
      }
    }
    if (r === undefined) {
      return;
    }
    const sink = this.#logger ?? (console as unknown as Logger);
    try {
      sink[level](`inorbithr ${r.event}`, r);
    } catch {
      // A failing sink never fails a call.
    }
  }
}

/** The path without its query, as records show it. */
export function logPath(url: URL): string {
  return url.pathname;
}

/** The URL with every query value replaced by `REDACTED`, for `url.full`. */
export function redactedUrl(url: URL): string {
  if (url.search === "") {
    return url.href;
  }
  const u = new URL(url.href);
  const names = [...new Set([...u.searchParams.keys()])];
  u.search = names.map((n) => `${encodeURIComponent(n)}=REDACTED`).join("&");
  return u.href;
}

/** The span context OpenTelemetry hands out. */
export interface SpanContextLike {
  readonly traceId: string;
  readonly spanId: string;
  readonly traceFlags: number;
  readonly isRemote?: boolean;
  readonly traceState?: { serialize(): string };
}

/** The parts of an OpenTelemetry span this package uses. */
export interface SpanLike {
  setAttribute(key: string, value: string | number | boolean): unknown;
  setStatus(status: { code: number; message?: string }): unknown;
  spanContext(): SpanContextLike;
  end(): void;
}

/** The parts of an OpenTelemetry tracer this package uses. */
export interface TracerLike {
  startSpan(
    name: string,
    options: { kind: number; attributes?: Record<string, string | number | boolean> },
    context?: unknown,
  ): SpanLike;
}

/** A tracer provider (`trace.getTracerProvider()`, or your SDK's). */
export interface TracerProviderLike {
  getTracer(name: string, version?: string): TracerLike;
}

interface Instrument {
  record?(value: number, attributes?: Record<string, string | number | boolean>): void;
  add?(value: number, attributes?: Record<string, string | number | boolean>): void;
}

interface MeterLike {
  createHistogram(name: string, options?: Record<string, unknown>): Instrument;
  createCounter(name: string, options?: Record<string, unknown>): Instrument;
}

/** A meter provider (`metrics.getMeterProvider()`, or your SDK's). */
export interface MeterProviderLike {
  getMeter(name: string, version?: string): MeterLike;
}

/** The parts of `@opentelemetry/api` this package uses; pass the module as `opentelemetry`. */
export interface OpenTelemetryApi {
  readonly trace: {
    getTracerProvider(): TracerProviderLike;
    setSpan(context: unknown, span: SpanLike): unknown;
    setSpanContext(context: unknown, spanContext: SpanContextLike): unknown;
    getSpan(context: unknown): SpanLike | undefined;
    isSpanContextValid(spanContext: SpanContextLike): boolean;
  };
  readonly context: { active(): unknown };
  readonly metrics?: { getMeterProvider(): MeterProviderLike };
}

const OTEL_MODULE = "@opentelemetry/api";
let installed: Promise<OpenTelemetryApi | undefined> | undefined;

/** `@opentelemetry/api` when it is installed, loaded once. */
export function findOpenTelemetry(): Promise<OpenTelemetryApi | undefined> {
  installed ??= (async () => {
    try {
      const specifier = OTEL_MODULE;
      const m = (await import(specifier)) as Partial<OpenTelemetryApi> & {
        default?: Partial<OpenTelemetryApi>;
      };
      const api = m.trace === undefined ? m.default : m;
      return api?.trace !== undefined && api.context !== undefined
        ? (api as OpenTelemetryApi)
        : undefined;
    } catch {
      return undefined;
    }
  })();
  return installed;
}

/** Span kinds, as `@opentelemetry/api` numbers them. */
export const SPAN_KIND = { internal: 0, client: 2 } as const;
/** The error status code. */
export const STATUS_ERROR = 2;

/** One client's tracer and meters, when tracing or metrics are on. */
export interface Telemetry {
  readonly api: OpenTelemetryApi;
  readonly tracer: TracerLike | undefined;
  readonly requestDuration: Instrument | undefined;
  readonly callDuration: Instrument | undefined;
  readonly retries: Instrument | undefined;
  readonly exchanges: Instrument | undefined;
}

/** Builds the client's telemetry from the settings; `undefined` when both are off or no API is there. */
export async function telemetry(options: {
  tracing: boolean | undefined;
  metrics: boolean | undefined;
  api: OpenTelemetryApi | undefined;
  tracerProvider: TracerProviderLike | undefined;
  meterProvider: MeterProviderLike | undefined;
}): Promise<Telemetry | undefined> {
  if (options.tracing === false && options.metrics === false) {
    return undefined;
  }
  const api = options.api ?? (await findOpenTelemetry());
  if (api === undefined) {
    return undefined;
  }
  const tracingOn = options.tracing ?? true;
  const metricsOn = options.metrics ?? tracingOn;
  const tracer = tracingOn
    ? (options.tracerProvider ?? api.trace.getTracerProvider()).getTracer("inorbithr", SDK_VERSION)
    : undefined;
  const meterProvider = metricsOn
    ? (options.meterProvider ?? api.metrics?.getMeterProvider())
    : undefined;
  const meter = meterProvider?.getMeter("inorbithr", SDK_VERSION);
  return {
    api,
    tracer,
    requestDuration: meter?.createHistogram("http.client.request.duration", {
      unit: "s",
      description: "Duration of HTTP client requests.",
      advice: {
        explicitBucketBoundaries: [
          0.005, 0.01, 0.025, 0.05, 0.075, 0.1, 0.25, 0.5, 0.75, 1, 2.5, 5, 7.5, 10,
        ],
      },
    }),
    callDuration: meter?.createHistogram("inorbit.client.call.duration", { unit: "s" }),
    retries: meter?.createCounter("inorbit.client.retries", { unit: "{retry}" }),
    exchanges: meter?.createCounter("inorbit.client.token.exchanges", { unit: "{exchange}" }),
  };
}

/** Parses a W3C `traceparent`, or `undefined` when it is not one. */
export function parseTraceparent(value: string): SpanContextLike | undefined {
  const m = /^00-([0-9a-f]{32})-([0-9a-f]{16})-([0-9a-f]{2})$/.exec(value.trim());
  if (m === null || /^0+$/.test(m[1] ?? "") || /^0+$/.test(m[2] ?? "")) {
    return undefined;
  }
  return {
    traceId: m[1] ?? "",
    spanId: m[2] ?? "",
    traceFlags: Number.parseInt(m[3] ?? "0", 16),
    isRemote: true,
  };
}

/** The `traceparent` for a span context. */
export function traceparent(sc: SpanContextLike): string {
  return `00-${sc.traceId}-${sc.spanId}-${(sc.traceFlags & 0xff).toString(16).padStart(2, "0")}`;
}
