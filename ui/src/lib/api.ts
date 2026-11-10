// What the page knows of who it acts as, and how it reads the API (RFC 0217, round 4). The page never holds a
// credential: `iohr ui serve` signs in as `iohr login` does and calls the API for the page, so every read goes to
// `/api/proxy/v1/...` on this origin with the per-serve token in a header, and the answer is only what the API said.
import { startToken } from "./panels.ts";

/** How the sign-in the page started stands. */
export type LoginPhase = "idle" | "waiting" | "failed";

/** `GET /api/session`: no credential, ever. */
export type SessionState =
  | {
      state: "signed-in";
      profile: string;
      kind: string;
      account: string;
      subject: string;
      scopes: string[];
      login: LoginPhase;
    }
  | { state: "signed-out"; login: LoginPhase; reason: string | null }
  | { state: "unreachable"; reason: string };

/** A read's answer: the JSON the API sent, or why not. Never throws. */
export type Read<T> =
  | { state: "ok"; data: T }
  | { state: "signed-out" }
  | { state: "error"; status: number; reason: string };

const phase = (v: unknown): LoginPhase => (v === "waiting" || v === "failed" ? v : "idle");
const text = (v: unknown): string => (typeof v === "string" ? v : "");

const own = (token: string | null): HeadersInit => (token === null ? {} : { "x-iohr-token": token });

/** Who this machine is signed in as, and how a sign-in started from the page is going. */
export async function readSession(token = startToken(), fetcher: typeof fetch = fetch): Promise<SessionState> {
  if (token === null)
    return { state: "unreachable", reason: "This page has no start token: open it with `iohr ui open`." };
  let res: Response;
  try {
    res = await fetcher("/api/session", { headers: own(token), cache: "no-store" });
  } catch {
    return { state: "unreachable", reason: "The local `iohr ui serve` did not answer." };
  }
  if (res.status === 401)
    return {
      state: "unreachable",
      reason: "This page is not opened from `iohr ui open`: its launch link has not been used.",
    };
  if (!res.ok) return { state: "unreachable", reason: `The session answer failed (HTTP ${res.status}).` };
  let body: unknown;
  try {
    body = await res.json();
  } catch {
    return { state: "unreachable", reason: "The session answer was not JSON." };
  }
  const v = typeof body === "object" && body !== null ? (body as Record<string, unknown>) : {};
  const login = phase(v.login);
  if (v.signed_in === true) {
    const scopes = Array.isArray(v.scopes) ? v.scopes.filter((s): s is string => typeof s === "string") : [];
    return {
      state: "signed-in",
      profile: text(v.profile),
      kind: text(v.kind),
      account: text(v.account),
      subject: text(v.subject),
      scopes,
      login,
    };
  }
  return { state: "signed-out", login, reason: typeof v.reason === "string" ? v.reason : null };
}

const send = async (path: string, token: string | null, fetcher: typeof fetch): Promise<boolean> => {
  if (token === null) return false;
  try {
    return (await fetcher(path, { method: "POST", headers: own(token), cache: "no-store" })).ok;
  } catch {
    return false;
  }
};

/** Starts the sign-in: the person's browser opens, as with `iohr login --web`. True once the server took it. */
export const signIn = (token = startToken(), fetcher: typeof fetch = fetch): Promise<boolean> =>
  send("/api/session/login", token, fetcher);

/** Signs out as `iohr logout` does. True once it did. */
export const signOut = (token = startToken(), fetcher: typeof fetch = fetch): Promise<boolean> =>
  send("/api/session/logout", token, fetcher);

/** The proxy path of an API path (`/v1/...`) and its query; null when it is not a plain read the server would take. */
export function proxyPath(path: string, query: Record<string, string> = {}): string | null {
  if (!path.startsWith("/v1/") || path.includes("..") || path.includes("//") || /[?#\\]/.test(path)) return null;
  const pairs = Object.entries(query).map(([k, v]) => `${encodeURIComponent(k)}=${encodeURIComponent(v)}`);
  return `/api/proxy${path}${pairs.length === 0 ? "" : `?${pairs.join("&")}`}`;
}

/** One read of the API through the local server: GET only, and the API's own status. */
export async function apiGet<T = unknown>(
  path: string,
  query: Record<string, string> = {},
  token = startToken(),
  fetcher: typeof fetch = fetch,
): Promise<Read<T>> {
  const url = proxyPath(path, query);
  if (url === null) return { state: "error", status: 400, reason: "That is not a path the page may read." };
  if (token === null)
    return { state: "error", status: 0, reason: "This page has no start token: open it with `iohr ui open`." };
  let res: Response;
  try {
    res = await fetcher(url, { headers: own(token), cache: "no-store" });
  } catch {
    return { state: "error", status: 0, reason: "The local `iohr ui serve` did not answer." };
  }
  if (res.status === 401) return { state: "signed-out" };
  if (!res.ok) {
    let reason = `The API answered HTTP ${res.status}.`;
    try {
      const e = (await res.json()) as { error?: unknown };
      const m = typeof e.error === "string" ? e.error : (e.error as { message?: unknown } | undefined)?.message;
      if (typeof m === "string" && m !== "") reason = m;
    } catch {
      // the status line says it
    }
    return { state: "error", status: res.status, reason };
  }
  try {
    return { state: "ok", data: (await res.json()) as T };
  } catch {
    return { state: "error", status: res.status, reason: "The API answer was not JSON." };
  }
}
