// The bodies of the Planner's writes (RFC 0217), after the coordinator's plan routes: PUT /v1/coordinator/plans/{p}/items/{i}
// takes `{item, expected_version, holder}`, and an empty field clears it, so an edit is always sent as the whole item.
// Pure, so node tests the shapes; planner-data.ts sends them through `apiSend`.

import type { ApiItem } from "./plan-api.ts";
import { ITEM_KINDS, STATUSES } from "./planner.ts";

/** What a person changes in Write mode; a field left out keeps what the item has. */
export type ItemEdit = { title?: string; body?: string; status?: string; kind?: string; parent?: string; prs?: string[] };

export const TITLE_MAX = 300;
export const BODY_MAX = 20_000;

/** The first thing wrong with an edit, in a person's words; null when it can be sent. */
export function editProblem(e: ItemEdit): string | null {
  if (e.title !== undefined && e.title.trim() === "") return "The title cannot be empty.";
  if (e.title !== undefined && e.title.length > TITLE_MAX) return `The title is at most ${TITLE_MAX} characters.`;
  if (e.body !== undefined && e.body.length > BODY_MAX) return `The text is at most ${BODY_MAX} characters.`;
  if (e.status !== undefined && !(STATUSES as readonly string[]).includes(e.status)) return `A status is one of ${STATUSES.join(", ")}.`;
  if (e.kind !== undefined && !(ITEM_KINDS as readonly string[]).includes(e.kind)) return `A kind is one of ${ITEM_KINDS.join(", ")}.`;
  return null;
}

/** The request body for one item: the item the plan holds, the edit over it, and its version so a newer save is refused (not overwritten). A new item has no version. */
export function upsertBody(had: ApiItem | undefined, e: ItemEdit, holder = ""): { item: Record<string, unknown>; expected_version?: number; holder?: string } {
  const item: Record<string, unknown> = {
    kind: e.kind ?? had?.kind ?? "step",
    parent: e.parent ?? had?.parent ?? "",
    title: (e.title ?? had?.title ?? "").trim(),
    status: e.status ?? had?.status ?? "proposed",
    body: e.body ?? had?.body ?? "",
    doc: had?.doc ?? "",
    diagram: had?.diagram ?? "",
    preview: had?.preview ?? "",
    prs: e.prs ?? had?.prs ?? [],
    position: had?.position ?? 0,
  };
  return { item, ...(had?.version ? { expected_version: had.version } : {}), ...(holder ? { holder } : {}) };
}

/** An item id for a new item: the title's words, lower case, with a short suffix so two of the same title differ. */
export function newItemId(title: string, taken: ReadonlySet<string>, salt = Date.now()): string {
  const base = title.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-+|-+$/g, "").slice(0, 40) || "item";
  if (!taken.has(base)) return base;
  let n = salt.toString(36).slice(-4);
  while (taken.has(`${base}-${n}`)) n += "x";
  return `${base}-${n}`;
}

export const noteBody = (text: string): { body: string } => ({ body: text.trim() });
export const claimBody = (holder: string, seconds = 900): { holder: string; seconds: number } => ({ holder, seconds });
export const reasonBody = (reason: string): { reason: string } => ({ reason: reason.trim() });

const enc = encodeURIComponent;
export const planPath = (planId: string) => `/v1/coordinator/plans/${enc(planId)}`;
export const itemPath = (planId: string, itemId: string, tail = "") => `${planPath(planId)}/items/${enc(itemId)}${tail}`;
