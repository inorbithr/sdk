/**
 * What a generated surface imports from the runtime, and nothing else does. This module
 * is a contract with `iohr sdk generate`: a change that breaks generated code bumps
 * {@link VERSION}, and a surface generated for another version fails to compile.
 *
 * @module
 */

import { formatInt64, parseInt64 } from "./int64.js";

/** The surface contract this runtime implements. */
export const VERSION = 1 as const;

/**
 * Percent-encodes `value` as one path segment: every byte but the RFC 3986 unreserved
 * characters, so a slash or a space in an id never changes the route.
 */
export function pathSegment(value: string): string {
  return encodeURIComponent(value).replace(
    /[!'()*]/g,
    (c) => `%${c.charCodeAt(0).toString(16).toUpperCase()}`,
  );
}

/** Where a value holds a 64-bit integer: the integer itself, a model, a list or a map. */
export type Shape =
  | "i64"
  | { readonly ref: string }
  | { readonly array: Shape }
  | {
      readonly map: Shape;
    };

/** The fields of each model that hold a 64-bit integer, by model name. */
export type Shapes = Readonly<Record<string, Readonly<Record<string, Shape>>>>;

function walk(
  value: unknown,
  shape: Shape,
  shapes: Shapes,
  leaf: (v: unknown) => unknown,
): unknown {
  if (value === null || value === undefined) {
    return value;
  }
  if (shape === "i64") {
    return leaf(value);
  }
  if ("array" in shape) {
    return Array.isArray(value) ? value.map((v) => walk(v, shape.array, shapes, leaf)) : value;
  }
  if ("map" in shape) {
    if (typeof value !== "object") {
      return value;
    }
    return Object.fromEntries(
      Object.entries(value).map(([k, v]) => [k, walk(v, shape.map, shapes, leaf)]),
    );
  }
  const fields = shapes[shape.ref];
  if (fields === undefined || typeof value !== "object") {
    return value;
  }
  const out: Record<string, unknown> = { ...value };
  for (const [name, inner] of Object.entries(fields)) {
    if (name in out) {
      out[name] = walk(out[name], inner, shapes, leaf);
    }
  }
  return out;
}

/** Converts a value read from the wire: decimal strings become `bigint`. */
export function decode(value: unknown, shape: Shape, shapes: Shapes): unknown {
  return walk(value, shape, shapes, parseInt64);
}

/** Converts a value for the wire: `bigint` becomes a decimal string. */
export function encode(value: unknown, shape: Shape, shapes: Shapes): unknown {
  return walk(value, shape, shapes, (v) => (typeof v === "bigint" ? formatInt64(v) : v));
}
