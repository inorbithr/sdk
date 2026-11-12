// The pages of this UI that can be put in maintenance, the pure part (RFC 0217, slice 2). The Mac app keeps the same
// idea (iohr-mac src/lib/features.ts): a feature is a page or a part of one, it is online or down, a down one says why
// and when it is back, and the last change can be taken back for a short while. Here a feature is a page of
// `iohr ui serve`; the state is kept in this browser and means "this page is shown as under maintenance on this
// machine". It does not stop anything on a server. No React, no DOM, so node tests it.

export type Feature = { id: string; name: string };

/** What can be put in maintenance. A page joins this list in the change that adds the page. */
export const FEATURES: Feature[] = [
  { id: "home", name: "Home" },
  { id: "planner", name: "Planner" },
  { id: "extensions", name: "Extension panels" },
];

export type Mode = "online" | "maintenance";
export type Entry = { mode: Mode; reason: string; eta: string };
export type States = Record<string, Entry>;

/** Pressing Undo works for ten seconds after a change. */
export const UNDO_MS = 10_000;

const ONLINE: Entry = { mode: "online", reason: "", eta: "" };

export const featureOf = (id: string) => FEATURES.find((f) => f.id === id);
export const entryOf = (s: States, id: string): Entry => s[id] ?? ONLINE;
export const isDown = (e: Entry) => e.mode === "maintenance";
export const closedIds = (s: States) => FEATURES.map((f) => f.id).filter((id) => isDown(entryOf(s, id)));

/** What the page says when it is down: the reason if there is one, else a plain line. */
export const whyClosed = (e: Entry) => (e.reason !== "" ? e.reason : "Put in maintenance.");

const clean = (v: string) => v.trim().slice(0, 200);

/** Puts a known feature in maintenance with an optional reason and ETA; the same state comes back when nothing changes. */
export function putInMaintenance(s: States, id: string, why: { reason: string; eta: string }): States {
  if (!featureOf(id)) return s;
  const e: Entry = { mode: "maintenance", reason: clean(why.reason), eta: clean(why.eta) };
  const old = entryOf(s, id);
  return old.mode === e.mode && old.reason === e.reason && old.eta === e.eta ? s : { ...s, [id]: e };
}

export function bringBack(s: States, id: string): States {
  if (!isDown(entryOf(s, id))) return s;
  const { [id]: _gone, ...rest } = s;
  return rest;
}

export const bringEverythingOnline = (s: States): States => (closedIds(s).length === 0 ? s : {});

/** A change that can be taken back: the states before it, what the button says, and until when. */
export type Undo = { label: string; before: States; until: number };

export const undoOf = (label: string, before: States, now: number): Undo => ({ label, before, until: now + UNDO_MS });
export const undoLive = (u: Undo | null, now: number): u is Undo => u !== null && now < u.until;

/** A stored state made whole: unknown features, bad modes and non-text fields are dropped. */
export function parseStates(o: unknown): States {
  const r = typeof o === "object" && o !== null && !Array.isArray(o) ? (o as Record<string, unknown>) : {};
  const out: States = {};
  for (const f of FEATURES) {
    const e = r[f.id];
    if (typeof e !== "object" || e === null) continue;
    const v = e as Record<string, unknown>;
    if (v.mode !== "maintenance") continue;
    out[f.id] = {
      mode: "maintenance",
      reason: typeof v.reason === "string" ? clean(v.reason) : "",
      eta: typeof v.eta === "string" ? clean(v.eta) : "",
    };
  }
  return out;
}
