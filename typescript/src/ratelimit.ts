/**
 * The rate-limit snapshot an answer carries (config.md section 7.8): Envoy's draft-03
 * `X-RateLimit-*` headers, or the IETF draft 11 `RateLimit` and `RateLimit-Policy`
 * fields, which win when both are sent.
 *
 * @module
 */

/** The quota policy the IETF fields name. */
export interface RateLimitPolicy {
  /** The policy's name. */
  readonly name: string;
  /** Requests the window allows. */
  readonly quota?: number;
  /** The window, in milliseconds. */
  readonly window?: number;
}

/** What the last answer said about the rate limit. Fields the answer did not send are absent. */
export interface RateLimit {
  /** Requests the window allows. */
  readonly limit?: number;
  /** Requests left in the window. */
  readonly remaining?: number;
  /** Milliseconds until the window resets. */
  readonly reset?: number;
  /** The policy, when the IETF fields named one. */
  readonly policy?: RateLimitPolicy;
}

function count(v: string | null): number | undefined | null {
  if (v === null) {
    return undefined;
  }
  const t = v.trim();
  return /^\d+$/.test(t) ? Number(t) : null;
}

interface Item {
  readonly name: string;
  readonly params: ReadonlyMap<string, string>;
}

/** The first item of a structured-field list: `"name";k=v;k=v`. */
function firstItem(field: string): Item | undefined {
  const text = field.split(",")[0]?.trim() ?? "";
  const parts = text.split(";").map((p) => p.trim());
  const head = parts.shift() ?? "";
  const m = /^"([^"\\]*)"$/.exec(head) ?? /^([A-Za-z*][A-Za-z0-9_\-.:%*/]*)$/.exec(head);
  if (m === null) {
    return undefined;
  }
  const params = new Map<string, string>();
  for (const p of parts) {
    const eq = p.indexOf("=");
    if (eq > 0) {
      params.set(p.slice(0, eq).trim(), p.slice(eq + 1).trim());
    }
  }
  return { name: m[1] ?? "", params };
}

function int(v: string | undefined): number | undefined {
  return v !== undefined && /^\d+$/.test(v) ? Number(v) : undefined;
}

function strip<T extends object>(o: Readonly<Record<keyof T, unknown>>): T {
  return Object.fromEntries(Object.entries(o).filter(([, v]) => v !== undefined)) as T;
}

/** The snapshot `headers` carry, or `undefined` when they carry none (or a malformed one). */
export function readRateLimit(headers: Headers): RateLimit | undefined {
  const field = headers.get("ratelimit");
  if (field !== null) {
    const item = firstItem(field);
    if (item !== undefined) {
      const policyField = headers.get("ratelimit-policy");
      const policyItem = policyField === null ? undefined : firstItem(policyField);
      const reset = int(item.params.get("t"));
      const policy =
        policyItem === undefined || policyItem.name !== item.name
          ? undefined
          : strip<RateLimitPolicy>({
              name: policyItem.name,
              quota: int(policyItem.params.get("q")),
              window: ((w) => (w === undefined ? undefined : w * 1000))(
                int(policyItem.params.get("w")),
              ),
            });
      return strip<RateLimit>({
        limit: policy?.quota,
        remaining: int(item.params.get("r")),
        reset: reset === undefined ? undefined : reset * 1000,
        policy,
      });
    }
  }
  const limit = count(headers.get("x-ratelimit-limit"));
  const remaining = count(headers.get("x-ratelimit-remaining"));
  const reset = count(headers.get("x-ratelimit-reset"));
  if (limit === null || remaining === null || reset === null) {
    return undefined;
  }
  if (limit === undefined && remaining === undefined && reset === undefined) {
    return undefined;
  }
  return strip<RateLimit>({
    limit,
    remaining,
    reset: reset === undefined ? undefined : reset * 1000,
    policy: undefined,
  });
}
