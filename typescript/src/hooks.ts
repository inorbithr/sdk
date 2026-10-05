import type { InOrbitError, RawResponse } from "./errors.js";

/** One attempt of a call, as hooks see it. Never a header or a body. */
export interface Attempt {
  /** The operation (`radar.list_digests`), or the path for a raw call. */
  readonly operation: string;
  /** The HTTP method. */
  readonly method: string;
  /** The path, parameters bound. */
  readonly path: string;
  /** 1 for the first attempt. */
  readonly number: number;
  /** The `x-request-id` sent. */
  readonly requestId: string;
  /** The `Idempotency-Key` sent, on an operation that takes one. */
  readonly idempotencyKey?: string | undefined;
  /** The pipeline stage the hooks ran in (`per_retry` from the built-in `hooks`). */
  readonly stage?: "per_call" | "per_retry";
}

/** Observes calls: logging, metrics, tracing. Every method is optional. A hook that must change a request is a middleware. */
export interface Hook {
  /** Before an attempt is sent. */
  onRequest?(attempt: Attempt): void;
  /** After an answer arrived, whatever its status. */
  onResponse?(attempt: Attempt, response: RawResponse): void;
  /** When the call fails for good. */
  onError?(attempt: Attempt, error: InOrbitError): void;
  /** Before the wait of each retry: why (`503`, `connection`, `timeout`) and how long, in milliseconds. */
  onRetry?(attempt: Attempt, reason: string, delay: number): void;
}
