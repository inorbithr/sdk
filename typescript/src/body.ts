/**
 * The cap on an answer's body, applied while it is read (docs/config.md section 6.5).
 *
 * @module
 */

import { TooLargeError } from "./errors.js";

/** The largest answer read, 16 MiB, decoded. */
export const MAX_BODY: number = 16 * 1024 * 1024;

/**
 * The whole of `body`, refused with `TooLargeError` as soon as more than `max` bytes have
 * arrived: the rest is cancelled unread, so an oversized answer is never held whole.
 */
export async function readCapped(
  body: ReadableStream<Uint8Array> | null,
  max: number = MAX_BODY,
): Promise<Uint8Array> {
  if (body === null) {
    return new Uint8Array(0);
  }
  const reader = body.getReader();
  const parts: Uint8Array[] = [];
  let size = 0;
  for (;;) {
    const { done, value } = await reader.read();
    if (done) {
      break;
    }
    size += value.byteLength;
    if (size > max) {
      await reader.cancel().catch(() => undefined);
      throw new TooLargeError();
    }
    parts.push(value);
  }
  const out = new Uint8Array(size);
  let at = 0;
  for (const p of parts) {
    out.set(p, at);
    at += p.byteLength;
  }
  return out;
}
