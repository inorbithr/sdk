// Where a comment hangs (PRD 0065): a document heading (Decisions' heading id), a passage in a
// section (the Decisions quote shape of RFC 0062: exact, up to 64 characters either side, and
// its offset), a diagram node or edge (its element id), or a whole item. Pure, so it is tested
// on its own (tests/canvas-anchors.test.ts).

import type { Quote, Target } from "./schema.ts";

/**
 * A heading's anchor as Decisions makes it (the console's `slugify`, ui/console/src/lib/lab-core):
 * lowercase, backticks and brackets dropped, anything but letters, digits, spaces and "-" dropped,
 * spaces to "-". At most 200, as the comment API allows; "section" when nothing is left.
 */
export function headingId(title: string, max = 200): string {
  const id = title
    .toLowerCase()
    .replace(/[`[\]]/g, "")
    .replace(/[^a-z0-9\s-]/g, "")
    .trim()
    .replace(/\s+/g, "-")
    .slice(0, max);
  return id || "section";
}

export type Section = {
  /** The heading id, "" for the text before the first heading. */
  anchor: string;
  title: string;
  level: number;
  /** The section's text without its heading line. */
  body: string;
  /** Where the section's body starts in the document text (after the front matter is cut). */
  offset: number;
  /** Where its heading line starts, in the same text. */
  head: number;
};

/** The front matter, if any, and the text after it. */
export function splitFrontMatter(text: string): { meta: Record<string, string>; body: string; skipped: number } {
  const t = text.replace(/\r\n/g, "\n");
  if (!t.startsWith("---\n")) return { meta: {}, body: t, skipped: 0 };
  const end = t.indexOf("\n---", 4);
  if (end < 0) return { meta: {}, body: t, skipped: 0 };
  const meta: Record<string, string> = {};
  for (const line of t.slice(4, end).split("\n")) {
    const m = /^([a-z_]+):\s*(.*)$/.exec(line);
    if (m) meta[m[1]] = m[2];
  }
  const after = t.indexOf("\n", end + 4);
  const skipped = after < 0 ? t.length : after + 1;
  return { meta, body: t.slice(skipped), skipped };
}

/**
 * The document cut at its headings (levels 1 to 4), each with the anchor Decisions gives it, so a
 * comment on a section is a comment on that heading. Fenced code is never read as a heading.
 * A repeated title gets "-1", "-2", as the console numbers them.
 */
export function sections(text: string): Section[] {
  const { body } = splitFrontMatter(text);
  const lines = body.split("\n");
  const out: Section[] = [];
  const used = new Map<string, number>();
  let cur: Section = { anchor: "", title: "", level: 0, body: "", offset: 0, head: 0 };
  let buf: string[] = [];
  let fenced = false;
  let pos = 0;
  const flush = () => {
    const raw = buf.join("\n");
    const lead = /^\n*/.exec(raw)![0].length;
    cur.body = raw.slice(lead).replace(/\n+$/, "");
    cur.offset += lead;
    if (cur.anchor || cur.body.trim()) out.push(cur);
  };
  for (const line of lines) {
    const t = line.trimStart();
    if (t.startsWith("```") || t.startsWith("~~~")) fenced = !fenced;
    const h = !fenced && /^(#{1,4})\s+(.+?)\s*#*\s*$/.exec(line);
    if (h) {
      flush();
      const title = h[2].replace(/\*\*|__|`/g, "");
      let anchor = headingId(title);
      const n = used.get(anchor) ?? 0;
      used.set(anchor, n + 1);
      if (n) anchor = `${anchor}-${n}`;
      cur = { anchor, title, level: h[1].length, body: "", offset: pos + line.length + 1, head: pos };
      buf = [];
    } else buf.push(line);
    pos += line.length + 1;
  }
  flush();
  return out;
}

/** The Decisions quote for a selection: `start` is the offset in the document's text after its front matter. */
export function quoteFor(text: string, start: number, end: number): Quote | null {
  const { body } = splitFrontMatter(text);
  const exact = body.slice(start, end);
  if (!exact.trim() || exact.length > 1000) return null;
  return { exact, prefix: body.slice(Math.max(0, start - 64), start), suffix: body.slice(end, end + 64), start };
}

/** A selection inside one section's body, mapped to a quote on the whole document. */
export function quoteInSection(text: string, section: Section, selected: string, approxIndex = 0): Quote | null {
  const exact = selected.replace(/\s+$/, "");
  if (!exact.trim()) return null;
  // The rendered section is the body less its Markdown marks; find the passage in the body nearest the hint.
  let at = -1;
  let best = Infinity;
  for (let i = section.body.indexOf(exact); i >= 0; i = section.body.indexOf(exact, i + 1)) {
    const d = Math.abs(i - approxIndex);
    if (d < best) {
      best = d;
      at = i;
    }
  }
  if (at < 0) {
    // Marks broke the match (`code`, **bold**): match on the plain words instead.
    const re = new RegExp(
      exact
        .split(/\s+/)
        .map((w) => w.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"))
        .join("[\\s*_`]+"),
    );
    const m = re.exec(section.body);
    if (!m) return null;
    return quoteFor(text, section.offset + m.index, section.offset + m.index + m[0].length);
  }
  return quoteFor(text, section.offset + at, section.offset + at + exact.length);
}

/**
 * Finds a quote in a (possibly newer) text: the exact passage whose surroundings match best,
 * preferring the one nearest its old offset. Null when the passage is gone.
 */
export function locateQuote(text: string, q: Quote): { start: number; end: number } | null {
  const { body } = splitFrontMatter(text);
  if (!q.exact) return null;
  let best: { start: number; score: number } | null = null;
  for (let i = body.indexOf(q.exact); i >= 0; i = body.indexOf(q.exact, i + 1)) {
    let score = 0;
    if (q.prefix) score += common(body.slice(Math.max(0, i - q.prefix.length), i), q.prefix, true);
    if (q.suffix)
      score += common(body.slice(i + q.exact.length, i + q.exact.length + q.suffix.length), q.suffix, false);
    if (q.start !== undefined) score -= Math.min(50, Math.abs(i - q.start) / 100);
    if (!best || score > best.score) best = { start: i, score };
  }
  return best ? { start: best.start, end: best.start + q.exact.length } : null;
}

/** How many characters two strings share at their joining end (the prefix's end, the suffix's start). */
function common(a: string, b: string, fromEnd: boolean): number {
  let n = 0;
  const len = Math.min(a.length, b.length);
  for (let i = 0; i < len; i++) {
    const x = fromEnd ? a[a.length - 1 - i] : a[i];
    const y = fromEnd ? b[b.length - 1 - i] : b[i];
    if (x !== y) break;
    n++;
  }
  return n;
}

/** The heading a passage falls under: the anchor Decisions wants with a quote. */
export function anchorAt(text: string, start: number): string {
  let anchor = "";
  for (const s of sections(text)) if (s.offset <= start) anchor = s.anchor;
  return anchor;
}

/** A target as one short key, for markers and lookups: "itm_x", "itm_x#node-id", "itm_x#anchor~quote". */
export function targetKey(t: Target): string {
  return `${t.item_id}${t.element ? `#${t.element}` : ""}${t.quote ? `~${t.quote.start ?? ""}:${t.quote.exact.slice(0, 40)}` : ""}`;
}

/** Decisions comments of a document mapped onto the card's sections: anchor → open thread count. */
export function countByAnchor(
  comments: { anchor?: string; parent_id?: string; resolved?: boolean }[],
): Map<string, number> {
  const out = new Map<string, number>();
  for (const c of comments) {
    if (c.parent_id || c.resolved) continue;
    const a = c.anchor ?? "";
    out.set(a, (out.get(a) ?? 0) + 1);
  }
  return out;
}

/** Pull request links in a text, de-duplicated, in order. */
export function prsIn(text: string): string[] {
  const out: string[] = [];
  for (const m of text.matchAll(/https:\/\/github\.com\/[\w.-]+\/[\w.-]+\/pull\/\d+/g))
    if (!out.includes(m[0])) out.push(m[0]);
  return out;
}

/** Replaces one section's body (up to the next heading of any level), keeping the heading and everything else byte for byte. */
export function replaceSection(text: string, anchor: string, newBody: string): string | null {
  const { body, skipped } = splitFrontMatter(text);
  const all = sections(text);
  const i = all.findIndex((s) => s.anchor === anchor);
  if (i < 0) return null;
  const s = all[i];
  const next = all[i + 1];
  const end = next ? next.head : body.length;
  const before = body.slice(0, s.offset).replace(/\n+$/, "\n");
  const after = body.slice(end);
  const middle = `\n${newBody.replace(/^\n+|\n+$/g, "")}\n${after ? "\n" : ""}`;
  return text.replace(/\r\n/g, "\n").slice(0, skipped) + before + middle + after;
}
