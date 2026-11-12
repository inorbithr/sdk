// Orbits for this UI, the pure part (RFC 0217, slice 2). In the Mac app an orbit is a whole page the person picked,
// at a place on a grid, and they move between orbits the way macOS Spaces work (iohr-mac src/lib/orbits.ts, PRD 0105).
// A browser tab has one page at a time, so here the picked pages are a short list in a set order: you go to one from
// the Orbits menu or step to the previous or the next. A page is named by its route key: "home" or "ext:<name>".
// No React, no DOM, so node tests it.

export const MAX_ORBITS = 12;
export const HOME = "home";

export type Orbits = { version: 1; pages: string[] };

export const defaultOrbits = (): Orbits => ({ version: 1, pages: [HOME] });

export const has = (o: Orbits, page: string) => o.pages.includes(page);

/** Adds a page at the end; nothing changes for a page already in, or when the set is full. */
export function add(o: Orbits, page: string): Orbits {
  if (has(o, page) || o.pages.length >= MAX_ORBITS || page === "") return o;
  return { ...o, pages: [...o.pages, page] };
}

/** Takes a page out. Home is always an orbit, so the set is never empty and the way home is never lost. */
export function remove(o: Orbits, page: string): Orbits {
  if (page === HOME || !has(o, page)) return o;
  return { ...o, pages: o.pages.filter((p) => p !== page) };
}

/** The page one step from `page` in the list; null at either end or for a page that is not an orbit. */
export function neighbour(o: Orbits, page: string, step: -1 | 1): string | null {
  const i = o.pages.indexOf(page);
  if (i < 0) return null;
  return o.pages[i + step] ?? null;
}

/** A stored set made whole: junk, duplicates and pages that no longer exist are dropped, and Home is always first. */
export function parseOrbits(raw: unknown, exists: (page: string) => boolean = () => true): Orbits {
  const r = typeof raw === "object" && raw !== null && !Array.isArray(raw) ? (raw as Record<string, unknown>) : {};
  const seen = new Set<string>([HOME]);
  const pages = [HOME];
  if (r.version === 1 && Array.isArray(r.pages)) {
    for (const p of r.pages) {
      if (typeof p !== "string" || seen.has(p) || !exists(p) || pages.length >= MAX_ORBITS) continue;
      seen.add(p);
      pages.push(p);
    }
  }
  return { version: 1, pages };
}
