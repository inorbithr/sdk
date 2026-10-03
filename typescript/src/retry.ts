/** Retry-After is honoured up to a minute. */
export const RETRY_AFTER_CAP_MS = 60_000;
const BACKOFF_BASE_MS = 500;
const BACKOFF_CAP_MS = 8_000;

/** Statuses worth another attempt on an idempotent call. */
export function retryableStatus(status: number): boolean {
  return status === 429 || status === 503 || status === 504;
}

/** The wait `Retry-After` asks for, in milliseconds, capped; seconds only. */
export function retryAfterMs(headers: Headers): number | undefined {
  const value = headers.get("retry-after")?.trim();
  if (value === undefined || !/^\d+$/.test(value)) {
    return undefined;
  }
  return Math.min(Number(value) * 1000, RETRY_AFTER_CAP_MS);
}

/** Full jitter: a random wait up to 0.5 s doubled per retry, at most 8 s. */
export function backoffMs(retry: number): number {
  const ceiling = Math.min(BACKOFF_BASE_MS * 2 ** Math.min(retry, 5), BACKOFF_CAP_MS);
  // A uniform fraction in [0, 1) from 32 random bits: scaling, not modulo, so no bias.
  const [r = 0] = crypto.getRandomValues(new Uint32Array(1));
  return Math.floor((r / 2 ** 32) * (ceiling + 1));
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
