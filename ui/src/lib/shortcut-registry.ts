// The one list of every keyboard shortcut of the web UI (RFC 0217), ported from the Mac app's
// src/lib/shortcut-registry.ts and folded into the frame's own tables: nothing is listed twice.
//
// Four sources feed it, each a JSON table, none copied by hand:
//   1. frame/shortcuts.json: the frame's global keys, the file iohr-mac's native menu reads too,
//   2. frame/layout-shortcuts.json: the window layouts,
//   3. shortcuts/web-shortcuts.json: the keys only the web UI has (the `g` then a letter page jumps, `?`),
//   4. shortcuts/pages/*.keys.json: one file per page or surface, so each area registers its own keys as data.
// The terminal panel's keys (shortcuts/terminal-keys.json) are listed beside them for comparison; a test checks
// each against the Rust file that binds it.
//
// Data, not code, is what lets a test see every binding at once: it fails when two keys collide, when a key is
// the browser's own, or when a row is malformed. A handler is attached by id at run time (use-shortcuts.ts).
//
// Pure: no React, no DOM, so node tests it.

import { canon, type KeyLike, matches } from "../frame/shortcuts.ts";

export type Source = "frame" | "layout" | "web" | "page";

/** A row of a table. A row has a chord (`accel`, "CmdOrCtrl+K") or a sequence (`seq`, ["g", "d"]), never both. */
export type Row = {
  id: string;
  label: string;
  accel?: string;
  /** Keys pressed one after the other, each alone: ["g", "d"]. */
  seq?: string[];
  /** The page id this key goes to or belongs to; the help page marks it when the build has no such page. */
  page?: string;
  /** The key the Mac app uses for the same thing, shown beside it (a browser keeps that key for itself). */
  mac?: string;
  /** The id of a global binding this one stands in front of, on its own scope only (the Decisions ⌘K). */
  overrides?: string;
  /** Works while a text field has the focus (a modifier chord always does; a bare key never does unless set). */
  inText?: boolean;
  where?: string;
};

export type Group = { scope: string; title: string; source: Source; page?: string; where?: string; rows: Row[] };

export type Binding = Row & { scope: string; title: string; source: Source; where: string };

export const GLOBAL = "global";

/** Every row of every group, with its group's scope, title and place. */
export function flatten(groups: Group[]): Binding[] {
  return groups.flatMap((g) =>
    g.rows.map((r) => ({
      ...r,
      page: r.page ?? g.page,
      scope: g.scope,
      title: g.title,
      source: g.source,
      where: r.where ?? g.where ?? (g.scope === GLOBAL ? "Anywhere in the page" : g.title),
    })),
  );
}

// ---------------------------------------------------------------- collisions

/** The key as one comparable string: a chord in its one spelling, a sequence lower-cased. */
export const keyOf = (b: Pick<Row, "accel" | "seq">): string =>
  b.seq ? `seq ${b.seq.map((k) => k.toLowerCase()).join(" ")}` : b.accel ? `chord ${canon(b.accel)}` : "none";

const bare = (b: Row) => b.accel !== undefined && !b.accel.includes("+") && b.accel.length === 1;

const overlap = (a: Binding, b: Binding) => a.scope === b.scope || a.scope === GLOBAL || b.scope === GLOBAL;

/** Whether two bindings take the same key where both are live (or one is the start of the other's sequence). */
export function clash(a: Binding, b: Binding): boolean {
  if (!overlap(a, b)) return false;
  if (a.overrides === b.id || b.overrides === a.id) return false;
  if (keyOf(a) === "none" || keyOf(b) === "none") return false;
  if (a.seq && b.seq) {
    const [s, l] = a.seq.length <= b.seq.length ? [a.seq, b.seq] : [b.seq, a.seq];
    return s.every((k, i) => k.toLowerCase() === l[i].toLowerCase());
  }
  if (a.seq || b.seq) {
    const [s, c] = a.seq ? [a, b] : [b, a];
    return bare(c) && s.seq?.[0].toLowerCase() === c.accel?.toLowerCase();
  }
  return keyOf(a) === keyOf(b);
}

/** Every pair of bindings that take the same key. Empty is the goal. */
export function collisions(bs: Binding[]): [Binding, Binding][] {
  const out: [Binding, Binding][] = [];
  for (let i = 0; i < bs.length; i++)
    for (let j = i + 1; j < bs.length; j++) if (clash(bs[i], bs[j])) out.push([bs[i], bs[j]]);
  return out;
}

// ---------------------------------------------------------------- the browser's own keys

export type Reserved = { accel: string; why: string; allowIn?: string[] };

/** Cmd and Ctrl are one key for this check: a rule written for one holds for the other. */
const loose = (accel: string) => canon(accel).replace("Ctrl+", "Cmd+");

/** Why a chord is not free to take in this scope, or null. */
export function reservedWhy(accel: string, scope: string, reserved: Reserved[]): string | null {
  const c = loose(accel);
  const hit = reserved.find((r) => loose(r.accel) === c && !r.allowIn?.includes(scope));
  return hit ? hit.why : null;
}

// ---------------------------------------------------------------- checking a whole table

/** What is wrong with the bindings, in words; empty when they are sound. */
export function problems(bs: Binding[], reserved: Reserved[]): string[] {
  const out: string[] = [];
  const seen = new Set<string>();
  for (const b of bs) {
    if (seen.has(b.id)) out.push(`${b.id}: listed twice`);
    seen.add(b.id);
    if (b.label.trim() === "") out.push(`${b.id}: no label`);
    if ((b.accel === undefined) === (b.seq === undefined))
      out.push(`${b.id}: needs an accel or a seq, not both and not neither`);
    if (b.seq && (b.seq.length < 2 || b.seq.some((k) => k.length !== 1)))
      out.push(`${b.id}: a seq is two or more single keys`);
    if (b.accel !== undefined) {
      const why = reservedWhy(b.accel, b.scope, reserved);
      if (why) out.push(`${b.id}: ${b.accel} is the browser's (${why})`);
    }
    if (b.overrides !== undefined) {
      const target = bs.find((x) => x.id === b.overrides);
      if (!target || target.scope !== GLOBAL)
        out.push(`${b.id}: overrides ${b.overrides}, which is not a global binding`);
      if (b.scope === GLOBAL) out.push(`${b.id}: a global binding cannot override`);
    }
  }
  for (const [a, b] of collisions(bs))
    out.push(`${a.id} and ${b.id} both take ${a.accel ?? a.seq?.join(" ")} / ${b.accel ?? b.seq?.join(" ")}`);
  return out;
}

// ---------------------------------------------------------------- reading a key event

/** The part of an event's target that decides whether the person is typing. */
export type Target = { tagName?: string; isContentEditable?: boolean; type?: string } | null;

/** Whether the keys go into a text field (so a bare key must not act as a shortcut). */
export function typingIn(t: Target | undefined): boolean {
  if (!t) return false;
  if (t.isContentEditable) return true;
  const tag = (t.tagName ?? "").toUpperCase();
  if (tag === "TEXTAREA" || tag === "SELECT") return true;
  if (tag !== "INPUT") return false;
  return !["button", "checkbox", "radio", "range", "submit", "reset", "color", "file", "image"].includes(
    (t.type ?? "text").toLowerCase(),
  );
}

/** Whether a key event is this chord. A bare symbol such as ? matches however the layout reaches it. */
export function chordMatches(accel: string, e: KeyLike): boolean {
  if (accel.length === 1 && !/[a-z0-9]/i.test(accel)) return e.key === accel && !e.metaKey && !e.ctrlKey && !e.altKey;
  return matches(accel, e);
}

const hasCmdOrCtrl = (accel: string) => /(^|\+)(cmdorctrl|cmd|ctrl|command|control)\+/i.test(accel);

/** Whether a chord may fire while a text field has the focus. */
export const worksInText = (b: Binding): boolean =>
  b.inText === true || b.accel?.toLowerCase() === "escape" || (b.accel !== undefined && hasCmdOrCtrl(b.accel));

// ---------------------------------------------------------------- sequences

export type SeqState = { prefix: string[]; at: number };
export const SEQ_START: SeqState = { prefix: [], at: 0 };
/** How long the second key of a sequence may take. */
export const SEQ_WINDOW_MS = 1200;

/**
 * One key of a sequence: the new state and the id it completed, if any. Only plain keys count (no Cmd, Ctrl or
 * Alt); a pause, a non-fitting key or a modifier starts over, and a key that fits no sequence may start one.
 */
export function stepSeq(
  state: SeqState,
  e: KeyLike,
  seqs: { id: string; seq: string[] }[],
  now: number,
): { state: SeqState; fire?: string; holds: boolean } {
  if (e.metaKey || e.ctrlKey || e.altKey || e.key.length !== 1) return { state: SEQ_START, holds: false };
  const k = e.key.toLowerCase();
  const old = now - state.at <= SEQ_WINDOW_MS ? state.prefix : [];
  for (const prefix of [old, []]) {
    const cand = [...prefix, k];
    const done = seqs.find((s) => s.seq.length === cand.length && s.seq.every((x, i) => x.toLowerCase() === cand[i]));
    if (done) return { state: SEQ_START, fire: done.id, holds: false };
    if (seqs.some((s) => s.seq.length > cand.length && cand.every((x, i) => x === s.seq[i].toLowerCase())))
      return { state: { prefix: cand, at: now }, holds: true };
  }
  return { state: SEQ_START, holds: false };
}

// ---------------------------------------------------------------- the host: who handles a key

export type Handlers = Record<string, (() => void) | undefined>;
export type Press = KeyLike & { target?: Target; repeat?: boolean; preventDefault?: () => void };

/**
 * Finds the handler of a key press among what is mounted. A page scope is asked before the global one, so a
 * page may stand in front of a global key it names in `overrides`. Without a handler a binding does nothing and
 * the key passes through to the page (and to the browser).
 */
export function createHost(bindings: Binding[]) {
  type Mount = { scope: string; handlers: () => Handlers };
  const mounted = new Set<Mount>();
  let seq = SEQ_START;

  /** Live bindings with their handler, page scopes first. */
  const live = () => {
    const out: { b: Binding; run: () => void }[] = [];
    for (const m of mounted) {
      const h = m.handlers();
      for (const b of bindings) {
        const run = b.scope === m.scope ? h[b.id] : undefined;
        if (run) out.push({ b, run });
      }
    }
    return out.sort((x, y) => Number(x.b.scope === GLOBAL) - Number(y.b.scope === GLOBAL));
  };

  return {
    /** Attaches handlers for a scope while it is mounted; returns the way to detach. */
    mount(scope: string, handlers: () => Handlers): () => void {
      const m = { scope, handlers };
      mounted.add(m);
      return () => void mounted.delete(m);
    },
    /** Handles one key press; true when a binding took it (and its default was prevented). */
    press(e: Press, now: number): boolean {
      const typing = typingIn(e.target);
      const bs = live();
      const chord = bs.find(
        ({ b }) => b.accel !== undefined && chordMatches(b.accel, e) && (!typing || worksInText(b)),
      );
      if (chord) {
        seq = SEQ_START;
        e.preventDefault?.();
        chord.run();
        return true;
      }
      if (typing || e.repeat) {
        seq = SEQ_START;
        return false;
      }
      const seqs = bs.flatMap(({ b }) => (b.seq ? [{ id: b.id, seq: b.seq }] : []));
      const step = stepSeq(seq, e, seqs, now);
      seq = step.state;
      if (step.fire) {
        e.preventDefault?.();
        bs.find(({ b }) => b.id === step.fire)?.run();
        return true;
      }
      return false;
    },
  };
}
