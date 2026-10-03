/**
 * Every way a call fails, as one tree under {@link InOrbitError}. `instanceof` and
 * `err.code` tell them apart; the message says what failed and never holds a secret.
 *
 * @module
 */

/** The platform's error codes (`spec/problem.json`), with the HTTP status of each. */
export const CODES: Readonly<Record<KnownCode, number>> = {
  bad_request: 400,
  failed_precondition: 400,
  unauthenticated: 401,
  forbidden: 403,
  not_found: 404,
  method_not_allowed: 405,
  already_exists: 409,
  conflict: 409,
  payload_too_large: 413,
  unsupported_media_type: 415,
  rate_limited: 429,
  quota_exceeded: 429,
  cancelled: 499,
  internal: 500,
  unimplemented: 501,
  unavailable: 503,
  timeout: 504,
};

/** A code this version of the SDK knows. */
export type KnownCode =
  | "bad_request"
  | "failed_precondition"
  | "unauthenticated"
  | "forbidden"
  | "not_found"
  | "method_not_allowed"
  | "already_exists"
  | "conflict"
  | "payload_too_large"
  | "unsupported_media_type"
  | "rate_limited"
  | "quota_exceeded"
  | "cancelled"
  | "internal"
  | "unimplemented"
  | "unavailable"
  | "timeout";

/** An error code: a known one, or a newer one kept as the API wrote it. */
export type Code = KnownCode | (string & {});

/** One entry of an error's `details`; a type this version does not know is kept as it came. */
export type Detail =
  | { readonly type: "field"; readonly field: string; readonly description: string }
  | {
      readonly type: "info";
      readonly reason: string;
      readonly domain: string;
      readonly metadata: Readonly<Record<string, string>>;
    }
  | { readonly type: "retry"; readonly afterSeconds: number }
  | { readonly type: "unknown"; readonly value: unknown };

/** An HTTP answer as it came, for anything the typed result does not carry. */
export class RawResponse {
  /** The HTTP status. */
  readonly status: number;
  /** The response headers. */
  readonly headers: Headers;
  /** The body, at most 16 MiB. */
  readonly body: Uint8Array;
  /** The `x-request-id` this SDK sent. */
  readonly requestId: string;
  /** The request id the API answered with, if any. */
  readonly serverRequestId: string | undefined;
  /** How many attempts the call took. */
  readonly attempts: number;

  /** An answer read off the wire. */
  constructor(init: {
    status: number;
    headers: Headers;
    body: Uint8Array;
    requestId: string;
    attempts: number;
  }) {
    this.status = init.status;
    this.headers = init.headers;
    this.body = init.body;
    this.requestId = init.requestId;
    this.serverRequestId = init.headers.get("x-request-id") ?? undefined;
    this.attempts = init.attempts;
  }

  /** The body as text. */
  text(): string {
    return new TextDecoder().decode(this.body);
  }

  /** The body as JSON, or `undefined` when it is not JSON. */
  json(): unknown {
    try {
      return JSON.parse(this.text());
    } catch {
      return undefined;
    }
  }
}

/** The base of every error this SDK throws. */
export class InOrbitError extends Error {
  /** A stable kind: `api`, `connection`, `timeout`, `auth`, `config`, `too_large`, `decode`. */
  readonly kind: string;

  /** An error of `kind` with `message`. */
  constructor(kind: string, message: string, options?: ErrorOptions) {
    super(message, options);
    this.name = "InOrbitError";
    this.kind = kind;
  }
}

const GATEWAY_MESSAGES: Readonly<Record<number, string>> = {
  401: "the token was refused: it is missing, expired or revoked",
  403: "this credential may not call this route: its scopes or role do not allow it",
  404: "no such route or resource",
  429: "too many requests; try again shortly",
};

function gatewayMessage(status: number): string {
  return (
    GATEWAY_MESSAGES[status] ??
    (status >= 500 ? "the API failed to answer" : "the request was refused")
  );
}

/** The code a plain-text answer with `status` stands for. */
export function codeForStatus(status: number): Code {
  const byStatus: Readonly<Record<number, KnownCode>> = {
    400: "bad_request",
    401: "unauthenticated",
    403: "forbidden",
    404: "not_found",
    405: "method_not_allowed",
    409: "conflict",
    413: "payload_too_large",
    415: "unsupported_media_type",
    429: "rate_limited",
    499: "cancelled",
    501: "unimplemented",
    503: "unavailable",
    504: "timeout",
  };
  return byStatus[status] ?? (status >= 500 ? "internal" : `http_${status}`);
}

function readDetail(d: unknown): Detail {
  const v = (typeof d === "object" && d !== null ? d : {}) as Record<string, unknown>;
  const str = (k: string): string => (typeof v[k] === "string" ? (v[k] as string) : "");
  switch (v.type) {
    case "field":
      return { type: "field", field: str("field"), description: str("description") };
    case "info": {
      const metadata = (
        typeof v.metadata === "object" && v.metadata !== null ? v.metadata : {}
      ) as Record<string, string>;
      return { type: "info", reason: str("reason"), domain: str("domain"), metadata };
    }
    case "retry":
      return { type: "retry", afterSeconds: Number(v.after_seconds ?? 0) };
    default:
      return { type: "unknown", value: d };
  }
}

function truncate(s: string, max: number): string {
  const chars = [...s];
  return chars.length > max ? `${chars.slice(0, max).join("")}…` : s;
}

/** The API answered with the problem envelope, or a plain-text error from the gateway. */
export class ApiError extends InOrbitError {
  /** The HTTP status. */
  readonly status: number;
  /** The error code. */
  readonly code: Code;
  /** What the API said went wrong. */
  readonly problem: string;
  /** Typed details. */
  readonly details: readonly Detail[];
  /** The answer as it came. */
  readonly raw: RawResponse;

  /** The error an answer stands for. */
  constructor(raw: RawResponse) {
    const wire = raw.json() as { code?: unknown; error?: unknown; details?: unknown } | undefined;
    const hasEnvelope =
      wire !== undefined &&
      typeof wire === "object" &&
      wire !== null &&
      ((typeof wire.code === "string" && wire.code !== "") ||
        (typeof wire.error === "string" && wire.error !== ""));
    let code: Code;
    let message: string;
    let details: Detail[] = [];
    if (hasEnvelope) {
      code =
        typeof wire.code === "string" && wire.code !== "" ? wire.code : codeForStatus(raw.status);
      message = typeof wire.error === "string" ? wire.error : "";
      details = Array.isArray(wire.details) ? wire.details.map(readDetail) : [];
    } else {
      code = codeForStatus(raw.status);
      message = raw.text().trim();
    }
    message = message === "" ? gatewayMessage(raw.status) : truncate(message, 300);
    const id = raw.serverRequestId === undefined ? "" : `, request id ${raw.serverRequestId}`;
    super("api", `${message} (${code}, HTTP ${raw.status}${id})`);
    this.name = "ApiError";
    this.status = raw.status;
    this.code = code;
    this.problem = message;
    this.details = details;
    this.raw = raw;
  }

  /** Seconds the API asked to wait, from a `retry` detail or `Retry-After`. */
  retryAfterSeconds(): number | undefined {
    const detail = this.details.find((d) => d.type === "retry");
    if (detail !== undefined && detail.type === "retry") {
      return detail.afterSeconds;
    }
    const header = this.raw.headers.get("retry-after");
    return header !== null && /^\d+$/.test(header.trim()) ? Number(header.trim()) : undefined;
  }
}

/** The API could not be reached: DNS, TCP, TLS, or a reset before an answer. */
export class ConnectionError extends InOrbitError {
  /** The unreachable host. */
  readonly host: string;

  /** Cannot reach `host`, because of `reason`. */
  constructor(host: string, reason: string, options?: ErrorOptions) {
    super("connection", `cannot reach ${host}: ${reason}`, options);
    this.name = "ConnectionError";
    this.host = host;
  }
}

/** An attempt or the caller's deadline ran out. */
export class TimeoutError extends InOrbitError {
  /** The host that did not answer. */
  readonly host: string;

  /** `host` did not answer within `seconds`. */
  constructor(host: string, seconds: number) {
    super("timeout", `${host} did not answer within ${seconds} s`);
    this.name = "TimeoutError";
    this.host = host;
  }
}

/** The token exchange, or a custom token provider, failed. */
export class AuthError extends InOrbitError {
  /** The token endpoint's `error`, or `HTTP <status>`; empty for a transport failure. */
  readonly error: string;

  /** An auth failure with its message. */
  constructor(message: string, error = "", options?: ErrorOptions) {
    super("auth", message, options);
    this.name = "AuthError";
    this.error = error;
  }
}

/** The client was configured in a way it cannot work with. */
export class ConfigError extends InOrbitError {
  /** A configuration problem. */
  constructor(message: string) {
    super("config", message);
    this.name = "ConfigError";
  }
}

/** The answer is larger than 16 MiB. */
export class TooLargeError extends InOrbitError {
  /** An answer over the cap. */
  constructor() {
    super("too_large", "the answer is larger than 16 MiB; refusing to read it");
    this.name = "TooLargeError";
  }
}

/** The API answered something this version cannot read. */
export class DecodeError extends InOrbitError {
  /** The answer as it came. */
  readonly raw: RawResponse;

  /** An answer that does not read. */
  constructor(reason: string, raw: RawResponse) {
    super(
      "decode",
      `the API answered something this version of @inorbithr/sdk cannot read: ${reason}`,
    );
    this.name = "DecodeError";
    this.raw = raw;
  }
}
