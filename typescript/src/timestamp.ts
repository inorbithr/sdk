/**
 * Timestamps as the API sends them: RFC 3339 strings, `""` when unset (rule N5 of
 * `spec/README.md`, settled behaviour). Models keep the string; this reads it.
 *
 * @module
 */

const RFC3339 = /^\d{4}-\d{2}-\d{2}[Tt]\d{2}:\d{2}:\d{2}(\.\d+)?([Zz]|[+-]\d{2}:\d{2})$/;

/**
 * Reads a timestamp field: `undefined` for `""` (the field is unset), the instant for
 * an RFC 3339 string.
 *
 * @throws {RangeError} when the value is neither `""` nor RFC 3339.
 */
export function parseTimestamp(value: string): Date | undefined {
  if (value === "") return undefined;
  const ms = RFC3339.test(value) ? Date.parse(value) : Number.NaN;
  if (Number.isNaN(ms)) throw new RangeError("not an RFC 3339 timestamp");
  return new Date(ms);
}
