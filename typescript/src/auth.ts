/**
 * Where a call's bearer token comes from: a token you hold ({@link StaticToken}), an API
 * key exchanged for short-lived tokens ({@link ClientCredentials}), a token file
 * ({@link TokenFile}), the `iohr` login ({@link CliToken}), or your own
 * {@link TokenProvider}, cached by {@link CachedToken} and chained by
 * {@link ChainedCredential} (config.md section 5).
 *
 * @module
 */

import { AuthError, InOrbitError } from "./errors.js";
import { builtin, fs } from "./platform.js";
import { backoffMs, retryAfterMs, retryableStatus, sleep } from "./retry.js";

/** The audience every token for the API is asked for. */
export const AUDIENCE = "iohr-api";

/** Where an API key is exchanged for a token. */
export const DEFAULT_TOKEN_URL = "https://auth.inorbit.hr/oauth2/token";

const TOKEN_RETRIES = 2;
const TOKEN_TIMEOUT_MS = 30_000;
const REDACTED = "<redacted>";
const SOFT_EXPIRY_RETRY_MS = 5_000;
const FILE_CHECK_MS = 60_000;
const CLI_TIMEOUT_MS = 10_000;

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

/** What a {@link CachedToken} reports, for logs and metrics. */
export interface CachedTokenOptions {
  /** After each fetch of a fresh token, with the error when it failed. */
  readonly onRefresh?: (error?: unknown) => void;
  /** A refresh failed while the cached token was still valid, which the call then uses. */
  readonly onRefreshFailed?: (error: unknown) => void;
}

interface Held {
  readonly token: Token;
  /** When to fetch a fresh one: four fifths into its life. */
  readonly refreshAt: number;
}

/**
 * Gives any {@link TokenProvider} the caching rules of config.md section 5.3: the token
 * is kept in memory, refreshed when a fifth of its life is left, by one refresh however
 * many callers wait, and a still-valid token is used when a refresh fails (tried again
 * no sooner than 5 s later). A token without an expiry is kept until it is refused.
 */
export class CachedToken implements TokenProvider {
  readonly #source: TokenProvider;
  readonly #options: CachedTokenOptions;
  #held: Held | undefined;
  #pending: Promise<Held> | undefined;
  #quietUntil = 0;

  /** Caches what `source` hands out. */
  constructor(source: TokenProvider, options: CachedTokenOptions = {}) {
    this.#source = source;
    this.#options = options;
  }

  /** The cached token while four fifths of its life are left, else a fresh one. */
  async token(): Promise<Token> {
    const now = Date.now();
    const held = this.#held;
    if (held !== undefined && now < held.refreshAt) {
      return held.token;
    }
    const valid = held?.token.expiresAt !== undefined && now < held.token.expiresAt;
    if (held !== undefined && valid && now < this.#quietUntil) {
      return held.token;
    }
    this.#pending ??= this.#fetch().finally(() => {
      this.#pending = undefined;
    });
    try {
      const fresh = await this.#pending;
      this.#held = fresh;
      return fresh.token;
    } catch (e) {
      const still = this.#held;
      if (still?.token.expiresAt !== undefined && Date.now() < still.token.expiresAt) {
        this.#quietUntil = Date.now() + SOFT_EXPIRY_RETRY_MS;
        this.#options.onRefreshFailed?.(e);
        return still.token;
      }
      throw e;
    }
  }

  /** Drops the cached token and tells the source, so the next call fetches a fresh one. */
  async invalidate(): Promise<void> {
    this.#held = undefined;
    this.#quietUntil = 0;
    await this.#source.invalidate?.();
  }

  async #fetch(): Promise<Held> {
    const fetchedAt = Date.now();
    let token: Token;
    try {
      token = await this.#source.token();
    } catch (e) {
      this.#options.onRefresh?.(e);
      throw e;
    }
    this.#options.onRefresh?.();
    const refreshAt =
      token.expiresAt === undefined
        ? Number.POSITIVE_INFINITY
        : fetchedAt + Math.max(0, token.expiresAt - fetchedAt) * 0.8;
    return { token, refreshAt };
  }

  /** Never the token. */
  toJSON(): string {
    return `CachedToken(${REDACTED})`;
  }

  /** Never the token. */
  toString(): string {
    return this.toJSON();
  }
}

/** What exchanging an API key needs. Give `keySecret`, or `keySecretFile` for a secret that rotates. */
export interface ClientCredentialsOptions {
  /** The key's id (`ak_…`). */
  readonly keyId: string;
  /** The key's secret. Never log it. */
  readonly keySecret?: string;
  /** A file holding the secret, read before every exchange, so a rotated secret is picked up. */
  readonly keySecretFile?: string;
  /** The scopes to ask for, a subset of the key's. */
  readonly scopes: readonly string[];
  /** The token endpoint; plain HTTP only to this machine. */
  readonly tokenUrl?: string;
  /** The `fetch` to use. */
  readonly fetch?: typeof fetch;
  /** The user agent the exchange sends (the client's own when it builds the provider). */
  readonly userAgent?: string;
  /** What the cache reports. */
  readonly cache?: CachedTokenOptions;
}

async function readSecretFile(path: string, what: string): Promise<string> {
  const f = fs();
  if (f === undefined) {
    throw new AuthError(`cannot read ${what} ${path}: this runtime has no file system`);
  }
  try {
    return new TextDecoder().decode(await f.promises.readFile(path)).trim();
  } catch (e) {
    throw new AuthError(`cannot read ${what} ${path}`, "", { cause: e });
  }
}

/**
 * Exchanges an API key for 15-minute tokens (OAuth client credentials), caches the
 * token by {@link CachedToken}'s rules, refreshes it when less than a fifth of its life
 * is left or after a 401, and makes one exchange however many calls wait for it. A
 * `keySecretFile` is read before every exchange.
 */
export class ClientCredentials implements TokenProvider {
  readonly #keyId: string;
  readonly #secret: string | undefined;
  readonly #secretFile: string | undefined;
  readonly #scopes: readonly string[];
  readonly #tokenUrl: string;
  readonly #fetch: typeof fetch;
  readonly #userAgent: string | undefined;
  readonly #cache: CachedToken;

  /** A provider for one key. */
  constructor(options: ClientCredentialsOptions) {
    this.#keyId = options.keyId;
    this.#secret = options.keySecret;
    this.#secretFile = options.keySecretFile;
    this.#scopes = options.scopes;
    this.#tokenUrl = options.tokenUrl ?? DEFAULT_TOKEN_URL;
    this.#fetch = options.fetch ?? globalThis.fetch.bind(globalThis);
    this.#userAgent = options.userAgent;
    this.#cache = new CachedToken({ token: () => this.#exchange() }, options.cache);
  }

  /** The key id this provider exchanges. */
  get keyId(): string {
    return this.#keyId;
  }

  /** A cached token while four fifths of its life are left, else a fresh one. */
  token(): Promise<Token> {
    return this.#cache.token();
  }

  /** Drops the cached token, so the next call exchanges again. */
  invalidate(): void {
    void this.#cache.invalidate();
  }

  async #exchange(): Promise<Token> {
    const secret =
      this.#secretFile === undefined
        ? (this.#secret ?? "")
        : await readSecretFile(this.#secretFile, "the key secret file");
    const form = new URLSearchParams({
      grant_type: "client_credentials",
      audience: AUDIENCE,
      scope: this.#scopes.join(" "),
    });
    const basic = btoa(`${encodeURIComponent(this.#keyId)}:${encodeURIComponent(secret)}`);
    const headers: Record<string, string> = {
      authorization: `Basic ${basic}`,
      "content-type": "application/x-www-form-urlencoded",
    };
    if (this.#userAgent !== undefined) {
      headers["user-agent"] = this.#userAgent;
    }
    for (let attempt = 0; ; attempt++) {
      let resp: globalThis.Response;
      try {
        resp = await this.#fetch(this.#tokenUrl, {
          method: "POST",
          headers,
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
        await resp.body?.cancel().catch(() => undefined);
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
      return { access: answer.access_token, expiresAt: Date.now() + expiresIn * 1000 };
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

/** When a JWT's `exp` claim says it expires (read, not verified), or `undefined`. */
function jwtExpiry(token: string): number | undefined {
  const parts = token.split(".");
  if (parts.length !== 3) {
    return undefined;
  }
  try {
    const b64 = (parts[1] ?? "").replace(/-/g, "+").replace(/_/g, "/");
    const claims = JSON.parse(atob(b64.padEnd(Math.ceil(b64.length / 4) * 4, "="))) as {
      exp?: unknown;
    };
    return typeof claims.exp === "number" ? claims.exp * 1000 : undefined;
  } catch {
    return undefined;
  }
}

/**
 * A bearer token read from a file, such as a mounted Kubernetes Secret: read at first
 * use, again when the file's modification time or size changes (checked at most once a
 * minute), and at once after the API refused the token. A JWT's `exp` is its expiry.
 * When the file disappears, the token read last is used until it is refused.
 */
export class TokenFile implements TokenProvider {
  readonly #path: string;
  #token: Token | undefined;
  #stamp = "";
  #checked = 0;
  #reread = true;

  /** A provider for the token in `path`. */
  constructor(path: string) {
    this.#path = path;
  }

  /** The file this provider reads. */
  get path(): string {
    return this.#path;
  }

  /** The token in the file. */
  async token(): Promise<Token> {
    const now = Date.now();
    const held = this.#token;
    if (!this.#reread && held !== undefined && now - this.#checked < FILE_CHECK_MS) {
      return held;
    }
    const f = fs();
    if (f === undefined) {
      throw new AuthError(
        `cannot read the token file ${this.#path}: this runtime has no file system`,
      );
    }
    this.#checked = now;
    try {
      const stat = await f.promises.stat(this.#path);
      const stamp = `${stat.mtimeMs}:${stat.size}`;
      if (this.#reread || stamp !== this.#stamp || held === undefined) {
        const access = new TextDecoder().decode(await f.promises.readFile(this.#path)).trim();
        if (access === "") {
          throw new Error("the file is empty");
        }
        const expiresAt = jwtExpiry(access);
        this.#token = expiresAt === undefined ? { access } : { access, expiresAt };
        this.#stamp = stamp;
      }
    } catch (e) {
      if (held === undefined || this.#reread) {
        this.#token = undefined;
        throw new AuthError(`cannot read the token file ${this.#path}: ${describe(e)}`, "", {
          cause: e,
        });
      }
    }
    this.#reread = false;
    const token = this.#token ?? held;
    if (token === undefined) {
      throw new AuthError(`cannot read the token file ${this.#path}`);
    }
    return token;
  }

  /** The token was refused: read the file again before the next call. */
  invalidate(): void {
    this.#reread = true;
  }

  /** Never the token. */
  toJSON(): string {
    return `TokenFile(${this.#path})`;
  }

  /** Never the token. */
  toString(): string {
    return this.toJSON();
  }
}

/** What the `iohr` login source needs. */
export interface CliTokenOptions {
  /** The `iohr` profile whose login to use. */
  readonly profile: string;
  /** The command line to run (default `iohr` on `PATH`). */
  readonly program?: string;
  /** Milliseconds the command may take (default 10 000). */
  readonly timeout?: number;
  /** What the cache reports. */
  readonly cache?: CachedTokenOptions;
}

interface ChildProcess {
  readonly stdout: { on(e: "data", f: (d: Uint8Array) => void): void };
  readonly stderr: { on(e: "data", f: (d: Uint8Array) => void): void };
  on(e: "error", f: (err: Error) => void): void;
  on(e: "close", f: (code: number | null) => void): void;
  kill(): boolean;
}

interface ChildProcessModule {
  spawn(
    command: string,
    args: readonly string[],
    options: { stdio: readonly string[]; shell: false; windowsHide: true },
  ): ChildProcess;
}

function concat(chunks: readonly Uint8Array[]): Uint8Array {
  const out = new Uint8Array(chunks.reduce((n, c) => n + c.length, 0));
  let at = 0;
  for (const c of chunks) {
    out.set(c, at);
    at += c.length;
  }
  return out;
}

/**
 * The developer's `iohr login` (config.md section 5.4): runs `iohr auth token --profile
 * <name> --format json` without a shell and with standard input closed, and caches the
 * token it prints by {@link CachedToken}'s rules. Node, Deno and Bun only.
 */
export class CliToken implements TokenProvider {
  readonly #profile: string;
  readonly #program: string;
  readonly #timeout: number;
  readonly #cache: CachedToken;

  /** A provider for one `iohr` profile. */
  constructor(options: CliTokenOptions) {
    this.#profile = options.profile;
    this.#program = options.program ?? "iohr";
    this.#timeout = options.timeout ?? CLI_TIMEOUT_MS;
    this.#cache = new CachedToken({ token: () => this.#run() }, options.cache);
  }

  /** A cached token, or a fresh one from `iohr auth token`. */
  token(): Promise<Token> {
    return this.#cache.token();
  }

  /** Drops the cached token, so the next call runs `iohr` again. */
  invalidate(): Promise<void> {
    return this.#cache.invalidate();
  }

  #run(): Promise<Token> {
    const cp = builtin<ChildProcessModule>("node:child_process");
    if (cp === undefined) {
      return Promise.reject(new AuthError("the iohr login: this runtime cannot run iohr"));
    }
    const args = ["auth", "token", "--profile", this.#profile, "--format", "json"];
    return new Promise((resolve, reject) => {
      let child: ChildProcess;
      try {
        child = cp.spawn(this.#program, args, {
          stdio: ["ignore", "pipe", "pipe"],
          shell: false,
          windowsHide: true,
        });
      } catch (e) {
        reject(new AuthError(`the iohr login: cannot run ${this.#program}`, "", { cause: e }));
        return;
      }
      const out: Uint8Array[] = [];
      const err: Uint8Array[] = [];
      child.stdout.on("data", (d) => out.push(d));
      child.stderr.on("data", (d) => err.push(d));
      const timer = setTimeout(() => {
        child.kill();
        reject(
          new AuthError(
            `the iohr login: ${this.#program} did not answer within ${Math.round(this.#timeout / 1000)} s`,
          ),
        );
      }, this.#timeout);
      child.on("error", (e) => {
        clearTimeout(timer);
        reject(new AuthError(`the iohr login: cannot run ${this.#program}`, "", { cause: e }));
      });
      child.on("close", (code) => {
        clearTimeout(timer);
        if (code !== 0) {
          const line = new TextDecoder().decode(concat(err)).split("\n")[0] ?? "";
          const first = [...line.trim()].slice(0, 200).join("");
          reject(
            new AuthError(
              `the iohr login for profile ${this.#profile} failed (exit ${code}): ${first || "no message"}`,
              `exit_${code}`,
            ),
          );
          return;
        }
        // Standard output holds the token: it is parsed, never quoted in an error.
        const answer = parseObject(new TextDecoder().decode(concat(out)));
        if (typeof answer.access_token !== "string" || answer.access_token === "") {
          reject(new AuthError("the iohr login: iohr auth token printed no token"));
          return;
        }
        const expires =
          typeof answer.expires_at === "string" ? Date.parse(answer.expires_at) : Number.NaN;
        resolve(
          Number.isFinite(expires)
            ? { access: answer.access_token, expiresAt: expires }
            : { access: answer.access_token },
        );
      });
    });
  }

  /** Never the token. */
  toJSON(): string {
    return `CliToken(${this.#profile})`;
  }

  /** Never the token. */
  toString(): string {
    return this.toJSON();
  }
}

/**
 * Tries providers in order and keeps the first that hands out a token. When none does,
 * the {@link AuthError} lists why each failed.
 */
export class ChainedCredential implements TokenProvider {
  readonly #providers: readonly TokenProvider[];
  #chosen: TokenProvider | undefined;

  /** A chain of `providers`, tried in this order. */
  constructor(providers: readonly TokenProvider[]) {
    this.#providers = providers;
  }

  /** A token from the provider chosen, choosing it on the first call. */
  async token(): Promise<Token> {
    if (this.#chosen !== undefined) {
      return this.#chosen.token();
    }
    const failures: string[] = [];
    for (const p of this.#providers) {
      try {
        const t = await p.token();
        this.#chosen = p;
        return t;
      } catch (e) {
        failures.push(`${String(p)}: ${describe(e)}`);
      }
    }
    throw new AuthError(
      `no credential in the chain gave a token; tried:\n  ${failures.join("\n  ")}`,
    );
  }

  /** Tells the provider chosen. */
  async invalidate(): Promise<void> {
    await this.#chosen?.invalidate?.();
  }

  /** Never a token. */
  toJSON(): string {
    return `ChainedCredential(${this.#providers.length})`;
  }

  /** Never a token. */
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
  if (e instanceof InOrbitError) {
    return e.message;
  }
  if (e instanceof Error) {
    const cause = e.cause instanceof Error ? `: ${e.cause.message}` : "";
    return `${e.message}${cause}`;
  }
  return String(e);
}

// Node's `util.inspect` (and `console.log`) print what `toJSON` does: never a secret.
for (const cls of [
  StaticToken,
  ClientCredentials,
  CachedToken,
  TokenFile,
  CliToken,
  ChainedCredential,
]) {
  Object.defineProperty(cls.prototype, Symbol.for("nodejs.util.inspect.custom"), {
    value(this: { toJSON(): string }): string {
      return this.toJSON();
    },
  });
}
