/**
 * Where a call's bearer token comes from: a token you hold ({@link StaticToken}), an API
 * key exchanged for short-lived tokens ({@link ClientCredentials}), or your own
 * {@link TokenProvider}.
 *
 * @module
 */

import { AuthError } from "./errors.js";
import { backoffMs, retryAfterMs, retryableStatus, sleep } from "./retry.js";

/** The audience every token for the API is asked for. */
export const AUDIENCE = "iohr-api";

/** Where an API key is exchanged for a token. */
export const DEFAULT_TOKEN_URL = "https://auth.inorbit.hr/oauth2/token";

const TOKEN_RETRIES = 2;
const TOKEN_TIMEOUT_MS = 30_000;
const REDACTED = "<redacted>";

/** A bearer token and, when known, when it stops working (milliseconds since the epoch). */
export interface Token {
  /** The token. Never log it. */
  readonly access: string;
  /** When it expires, if the provider knows. */
  readonly expiresAt?: number;
}

/** Hands out a valid token; told when the API refused the last one. */
export interface TokenProvider {
  /** A token for the next attempt. */
  token(): Promise<Token>;
  /** The API answered 401 with the last token: drop any cached one. */
  invalidate?(): void | Promise<void>;
}

/** A token you already hold, such as an API token from the console. */
export class StaticToken implements TokenProvider {
  readonly #token: string;

  /** A provider that always hands out `token`. */
  constructor(token: string) {
    this.#token = token;
  }

  /** The token. */
  token(): Promise<Token> {
    return Promise.resolve({ access: this.#token });
  }

  /** Never the token. */
  toJSON(): string {
    return `StaticToken(${REDACTED})`;
  }

  /** Never the token. */
  toString(): string {
    return this.toJSON();
  }
}

/** What exchanging an API key needs. */
export interface ClientCredentialsOptions {
  /** The key's id (`ak_…`). */
  readonly keyId: string;
  /** The key's secret. Never log it. */
  readonly keySecret: string;
  /** The scopes to ask for, a subset of the key's. */
  readonly scopes: readonly string[];
  /** The token endpoint; plain HTTP only to this machine. */
  readonly tokenUrl?: string;
  /** The `fetch` to use. */
  readonly fetch?: typeof fetch;
}

interface Cached {
  readonly access: string;
  readonly issued: number;
  readonly lifetimeMs: number;
}

/**
 * Exchanges an API key for 15-minute tokens (OAuth client credentials), caches the
 * token, refreshes it when less than a fifth of its life is left or after a 401, and
 * makes one exchange however many calls wait for it.
 */
export class ClientCredentials implements TokenProvider {
  readonly #keyId: string;
  readonly #secret: string;
  readonly #scopes: readonly string[];
  readonly #tokenUrl: string;
  readonly #fetch: typeof fetch;
  #cache: Cached | undefined;
  #pending: Promise<Cached> | undefined;

  /** A provider for one key. */
  constructor(options: ClientCredentialsOptions) {
    this.#keyId = options.keyId;
    this.#secret = options.keySecret;
    this.#scopes = options.scopes;
    this.#tokenUrl = options.tokenUrl ?? DEFAULT_TOKEN_URL;
    this.#fetch = options.fetch ?? globalThis.fetch.bind(globalThis);
  }

  /** The key id this provider exchanges. */
  get keyId(): string {
    return this.#keyId;
  }

  /** A cached token while four fifths of its life are left, else a fresh one. */
  async token(): Promise<Token> {
    const now = Date.now();
    const cached = this.#cache;
    if (cached !== undefined && now - cached.issued < cached.lifetimeMs * 0.8) {
      return { access: cached.access, expiresAt: cached.issued + cached.lifetimeMs };
    }
    this.#pending ??= this.#exchange().finally(() => {
      this.#pending = undefined;
    });
    const fresh = await this.#pending;
    this.#cache = fresh;
    return { access: fresh.access, expiresAt: fresh.issued + fresh.lifetimeMs };
  }

  /** Drops the cached token, so the next call exchanges again. */
  invalidate(): void {
    this.#cache = undefined;
  }

  async #exchange(): Promise<Cached> {
    const form = new URLSearchParams({
      grant_type: "client_credentials",
      audience: AUDIENCE,
      scope: this.#scopes.join(" "),
    });
    const basic = btoa(`${encodeURIComponent(this.#keyId)}:${encodeURIComponent(this.#secret)}`);
    for (let attempt = 0; ; attempt++) {
      let resp: globalThis.Response;
      try {
        resp = await this.#fetch(this.#tokenUrl, {
          method: "POST",
          headers: {
            authorization: `Basic ${basic}`,
            "content-type": "application/x-www-form-urlencoded",
          },
          body: form,
          redirect: "manual",
          signal: AbortSignal.timeout(TOKEN_TIMEOUT_MS),
        });
      } catch (e) {
        if (attempt < TOKEN_RETRIES) {
          await sleep(backoffMs(attempt));
          continue;
        }
        throw new AuthError(`the token endpoint: ${describe(e)}`, "", { cause: e });
      }
      if (retryableStatus(resp.status) && attempt < TOKEN_RETRIES) {
        await sleep(retryAfterMs(resp.headers) ?? backoffMs(attempt));
        continue;
      }
      const text = await resp.text();
      if (resp.status < 200 || resp.status >= 300) {
        const refusal = parseObject(text);
        const error =
          typeof refusal.error === "string" && refusal.error !== ""
            ? refusal.error
            : `HTTP ${resp.status}`;
        const description =
          typeof refusal.error_description === "string" && refusal.error_description !== ""
            ? ` (${refusal.error_description})`
            : "";
        throw new AuthError(
          `the token exchange for key ${this.#keyId} failed: ${error}${description}`,
          error,
        );
      }
      const answer = parseObject(text);
      if (typeof answer.access_token !== "string") {
        throw new AuthError("the token endpoint: the token answer could not be read");
      }
      const expiresIn = typeof answer.expires_in === "number" ? answer.expires_in : 900;
      return { access: answer.access_token, issued: Date.now(), lifetimeMs: expiresIn * 1000 };
    }
  }

  /** Never the secret. */
  toJSON(): string {
    return `ClientCredentials(${this.#keyId}, secret: ${REDACTED})`;
  }

  /** Never the secret. */
  toString(): string {
    return this.toJSON();
  }
}

function parseObject(text: string): Record<string, unknown> {
  try {
    const v: unknown = JSON.parse(text);
    return typeof v === "object" && v !== null ? (v as Record<string, unknown>) : {};
  } catch {
    return {};
  }
}

/** A transport failure in words, without the request. */
export function describe(e: unknown): string {
  if (e instanceof Error) {
    const cause = e.cause instanceof Error ? `: ${e.cause.message}` : "";
    return `${e.message}${cause}`;
  }
  return String(e);
}

// Node's `util.inspect` (and `console.log`) print what `toJSON` does: never a secret.
for (const cls of [StaticToken, ClientCredentials]) {
  Object.defineProperty(cls.prototype, Symbol.for("nodejs.util.inspect.custom"), {
    value(this: { toJSON(): string }): string {
      return this.toJSON();
    },
  });
}
