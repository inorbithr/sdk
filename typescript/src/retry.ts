/** Retry-After is honoured up to a minute. */
export const RETRY_AFTER_CAP_MS = 60_000;
const BACKOFF_BASE_MS = 500;
const BACKOFF_CAP_MS = 8_000;

/** Statuses worth another attempt on an idempotent call. */
export function retryableStatus(status: number): boolean {
  return status === 429 || status === 503 || status === 504;
}

/**
 * The wait `Retry-After` asks for, in milliseconds: delay-seconds or an HTTP date
 * (RFC 9110 section 10.2.3); `undefined` when absent or unreadable.
 */
export function retryAfter(headers: Headers): number | undefined {
  const value = headers.get("retry-after")?.trim();
  if (value === undefined || value === "") {
    return undefined;
  }
  if (/^\d+$/.test(value)) {
    return Number(value) * 1000;
  }
  // IMF-fixdate, the one form a sender may generate: `Sun, 06 Nov 1994 08:49:37 GMT`.
  if (!/^[A-Z][a-z]{2}, \d{2} [A-Z][a-z]{2} \d{4} \d{2}:\d{2}:\d{2} GMT$/.test(value)) {
    return undefined;
  }
  const at = Date.parse(value);
  return Number.isFinite(at) ? Math.max(0, at - Date.now()) : undefined;
}

/** The wait `Retry-After` asks for, in milliseconds, capped at a minute. */
export function retryAfterMs(headers: Headers): number | undefined {
  const ms = retryAfter(headers);
  return ms === undefined ? undefined : Math.min(ms, RETRY_AFTER_CAP_MS);
}

/** Full jitter: a random wait up to `base` (0.5 s) doubled per retry, at most `cap` (8 s). */
export function backoffMs(
  retry: number,
  base: number = BACKOFF_BASE_MS,
  cap: number = BACKOFF_CAP_MS,
): number {
  const ceiling = Math.min(base * 2 ** Math.min(retry, 30), cap);
  // Jitter only spreads retries out; it is not a secret, so Math.random is the right tool.
  return Math.floor(Math.random() * (ceiling + 1));
}

/** `iohr-<16 hex>`, the id each call is sent with. */
export function requestId(): string {
  const bytes = crypto.getRandomValues(new Uint8Array(8));
  return `iohr-${[...bytes].map((b) => b.toString(16).padStart(2, "0")).join("")}`;
}

/** Waits `ms`, or rejects when `signal` aborts. */
export function sleep(ms: number, signal?: AbortSignal): Promise<void> {
  return new Promise((resolve, reject) => {
    if (signal?.aborted) {
      reject(signal.reason);
      return;
    }
    const timer = setTimeout(resolve, ms);
    signal?.addEventListener(
      "abort",
      () => {
        clearTimeout(timer);
        reject(signal.reason);
      },
      { once: true },
    );
  });
}
