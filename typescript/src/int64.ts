/**
 * A 64-bit integer. The API sends it as a decimal string; the SDK hands it over as a
 * `bigint`, so no value above 2^53 loses precision.
 */
export type Int64 = bigint;

/**
 * Reads a 64-bit integer from the wire: a decimal string, or a JSON number.
 *
 * @throws {TypeError} when the value is neither.
 */
export function parseInt64(value: unknown): Int64 {
  if (typeof value === "bigint") {
    return value;
  }
  if (typeof value === "string" && /^-?\d+$/.test(value)) {
    return BigInt(value);
  }
  if (typeof value === "number" && Number.isInteger(value)) {
    return BigInt(value);
  }
  throw new TypeError(`not a 64-bit integer: ${JSON.stringify(value)}`);
}

/** Writes a 64-bit integer as the API reads it: a decimal string. */
export function formatInt64(value: Int64): string {
  return value.toString();
}
