// The picker's rules: what a typed reference to a card means, so the page only draws the form.
// A card is a Decisions document (space and document id), a pull request (owner/repo#N) or a decision (PRD 0065, RFC 0217).

import { type Item, newId } from "./schema.ts";

export type CardKind = "document" | "pr" | "decision";
export type Picked = { ok: true; make: (at: { x: number; y: number }) => Item } | { ok: false; why: string };

const PR = /^([A-Za-z0-9_.-]{1,100})\/([A-Za-z0-9_.-]{1,100})#(\d{1,7})$/;
const DECISION = /^(prd|rfc|adr|study)\s*(\d{1,5}(?:\.\d+)?)$/i;
const DOC = /^(?:ldoc|doc)_[0-9A-Za-z]{8,40}$/;
const SPACE = /^(?:lab|spc)_[0-9A-Za-z]{8,40}$/;

const id = () => newId("itm");
const size = { w: 280, h: 120 };

/** Reads "owner/repo#12", "RFC 0217" or "space_id document_id". */
export function pick(kind: CardKind, text: string): Picked {
  const t = text.trim();
  if (!t) return { ok: false, why: "Type a reference first." };
  if (kind === "pr") {
    const m = PR.exec(t);
    if (!m) return { ok: false, why: "A pull request looks like owner/repo#12." };
    const [, owner, repo, n] = m;
    return {
      ok: true,
      make: (at) => ({
        item_id: id(),
        type: "pr",
        title: `${owner}/${repo}#${n}`,
        repo: `${owner}/${repo}`,
        number: Number(n),
        ...at,
        ...size,
      }),
    };
  }
  if (kind === "decision") {
    const m = DECISION.exec(t);
    if (!m) return { ok: false, why: "A decision looks like RFC 0217, PRD 0065, ADR 0230 or study 12." };
    const k = m[1].toLowerCase() as "prd" | "rfc" | "adr" | "study";
    return {
      ok: true,
      make: (at) => ({
        item_id: id(),
        type: "decision",
        title: `${k.toUpperCase()} ${m[2]}`,
        kind: k,
        number: m[2],
        ...at,
        ...size,
      }),
    };
  }
  const [space, doc] = t.split(/\s+/);
  if (!space || !doc || !SPACE.test(space) || !DOC.test(doc))
    return { ok: false, why: "Type the space id, a space, then the document id." };
  return {
    ok: true,
    make: (at) => ({
      item_id: id(),
      type: "document",
      title: doc,
      document: { space_id: space, document_id: doc },
      ...at,
      w: 360,
      h: 240,
    }),
  };
}
