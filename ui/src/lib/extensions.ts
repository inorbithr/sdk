// The UI's side of the extension panel seam (RFC 0217 C3): `GET /api/extensions` on the `iohr ui serve` listener.
// The server already decides which extensions have a panel (`ext::install::Store::panels`, which runs
// `manifest::check_panel` again on read); this file only reads the answer's shape and never repeats that rule.

/** One installed extension that declares a `[panel]`, as `/api/extensions` lists it. */
export type Panel = { name: string; version: string; title: string; entry: string };

export type Listing = { state: "ok"; panels: Panel[] } | { state: "error"; reason: string };

const isPanel = (v: unknown): v is Panel => {
  if (typeof v !== "object" || v === null) return false;
  const p = v as Record<string, unknown>;
  return (
    typeof p.name === "string" &&
    typeof p.version === "string" &&
    typeof p.title === "string" &&
    typeof p.entry === "string"
  );
};

/** The panels in a `/api/extensions` body, or null when the body is not that shape. Malformed rows are dropped. */
export function parseExtensions(body: unknown): Panel[] | null {
  if (typeof body !== "object" || body === null) return null;
  const list = (body as Record<string, unknown>).extensions;
  if (!Array.isArray(list)) return null;
  return list.filter(isPanel).map(({ name, version, title, entry }) => ({ name, version, title, entry }));
}

/** Reads the listing from this page's own listener. Never throws: a failure is a `Listing` with a reason. */
export async function loadExtensions(fetcher: typeof fetch = fetch): Promise<Listing> {
  let res: Response;
  try {
    res = await fetcher("/api/extensions", { headers: { accept: "application/json" }, cache: "no-store" });
  } catch {
    return { state: "error", reason: "The local `iohr ui serve` did not answer." };
  }
  if (!res.ok) return { state: "error", reason: `The extension list failed (HTTP ${res.status}).` };
  let body: unknown;
  try {
    body = await res.json();
  } catch {
    return { state: "error", reason: "The extension list was not JSON." };
  }
  const panels = parseExtensions(body);
  return panels ? { state: "ok", panels } : { state: "error", reason: "The extension list had an unknown shape." };
}
