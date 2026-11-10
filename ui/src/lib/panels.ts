// Starting and stopping a panel from the page `iohr ui serve` serves (RFC 0217 C4b). The server does every check:
// it starts only an installed panel, through `ext/run.rs` with the full trust check, and only for this page (its
// origin and the per-serve token in `<meta name="iohr-token">`). The client sends the name and nothing else.

/** What a start or stop answered: a port, or why not. Never throws. */
export type Started = { state: "ok"; port: number } | { state: "error"; reason: string };

/** The per-serve start token the server put in this page, or null when there is none (a dev server). */
export function startToken(doc: Pick<Document, "querySelector"> = document): string | null {
  const meta = doc.querySelector('meta[name="iohr-token"]');
  const token = meta?.getAttribute("content") ?? "";
  return /^[0-9a-f]{64}$/.test(token) ? token : null;
}

/** A loopback port a panel may hold. */
export const isPanelPort = (v: unknown): v is number =>
  typeof v === "number" && Number.isInteger(v) && v >= 1024 && v <= 65535;

const post = async (fetcher: typeof fetch, path: string, token: string | null): Promise<Response | string> => {
  if (token === null) return "This page has no start token: open it from `iohr ui serve`.";
  try {
    return await fetcher(path, { method: "POST", headers: { "x-iohr-token": token }, cache: "no-store" });
  } catch {
    return "The local `iohr ui serve` did not answer.";
  }
};

const failed: Record<number, string> = {
  404: "That extension is not installed.",
  409: "That panel is still starting.",
  502: "The panel did not start; the terminal running `iohr ui serve` says why.",
  503: "`iohr ui serve` is stopping.",
};

/** Asks the server to start `name`'s panel (or answer the one already running). */
export async function startPanel(name: string, token = startToken(), fetcher: typeof fetch = fetch): Promise<Started> {
  const res = await post(fetcher, `/api/extensions/${encodeURIComponent(name)}/start`, token);
  if (typeof res === "string") return { state: "error", reason: res };
  if (!res.ok) return { state: "error", reason: failed[res.status] ?? `The start failed (HTTP ${res.status}).` };
  let body: unknown;
  try {
    body = await res.json();
  } catch {
    return { state: "error", reason: "The start answer was not JSON." };
  }
  const port = typeof body === "object" && body !== null ? (body as Record<string, unknown>).port : undefined;
  return isPanelPort(port) ? { state: "ok", port } : { state: "error", reason: "The start answer had no port." };
}

/** Asks the server to stop `name`'s panel. True once it answered; stopping one that is not running is fine. */
export async function stopPanel(name: string, token = startToken(), fetcher: typeof fetch = fetch): Promise<boolean> {
  const res = await post(fetcher, `/api/extensions/${encodeURIComponent(name)}/stop`, token);
  return typeof res !== "string" && res.ok;
}

/** The frame address for a panel at `port`: plain HTTP on 127.0.0.1, and only a path under it, never another site. */
export function panelSrc(port: number, entry: string): string | null {
  if (!isPanelPort(port) || !entry.startsWith("/") || entry.startsWith("//")) return null;
  const base = `http://127.0.0.1:${port}`;
  const url = new URL(entry, base);
  return url.origin === base ? url.href : null;
}

/** The page route of a panel: `/ext/<name>`. */
export const panelRoute = (name: string): string => `/ext/${encodeURIComponent(name)}`;

/** The extension a page route names, or null. */
export function routedPanel(pathname: string): string | null {
  const m = /^\/ext\/([^/]+)\/?$/.exec(pathname);
  if (!m?.[1]) return null;
  try {
    return decodeURIComponent(m[1]);
  } catch {
    return null;
  }
}
