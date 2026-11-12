// One canvas held in the browser (PRD 0065, RFC 0217). There is no canvas API in the platform yet, so the web
// board is a per-viewer draft in localStorage: each canvas is the same `iohr.canvas` JSON file the Mac app keeps,
// under `iohr-ui.canvas.body.<id>`, with the viewer's pan and zoom beside it. When a canvas API exists this file
// is the one place that changes: `KV` becomes a save call and nothing above it knows.
//
// The controller mirrors the Mac's `useCanvas`: an edit applies at once and is saved a moment later; undo and redo
// keep up to 80 steps; a save that finds a newer revision (another tab saved first) replays the edits not yet
// saved on top of it. It has no React in it, so node tests drive it with a fake storage and a fake clock.

import { type Canvas, emptyCanvas, newId, parseCanvas, serialize } from "./schema.ts";
import type { View } from "./viewport.ts";

/** The storage the controller talks to: a Storage-like object, so tests pass a Map. */
export type KV = {
  get(k: string): string | null;
  set(k: string, v: string): void;
  remove(k: string): void;
  keys(): string[];
};

export const BODY = "iohr-ui.canvas.body.";
export const VIEW = "iohr-ui.canvas.view.";
export const LAST = "iohr-ui.canvas.last";
export const HISTORY = 80;
export const SAVE_AFTER = 400;
export const NO_CANVAS = "There is no canvas with that id in this browser.";

export const memoryKV = (m = new Map<string, string>()): KV => ({
  get: (k) => m.get(k) ?? null,
  set: (k, v) => void m.set(k, v),
  remove: (k) => void m.delete(k),
  keys: () => [...m.keys()],
});

export function browserKV(): KV {
  return {
    get: (k) => {
      try {
        return localStorage.getItem(k);
      } catch {
        return null;
      }
    },
    set: (k, v) => localStorage.setItem(k, v),
    remove: (k) => {
      try {
        localStorage.removeItem(k);
      } catch {
        // nothing to remove
      }
    },
    keys: () => {
      try {
        return Array.from({ length: localStorage.length }, (_, i) => localStorage.key(i) ?? "").filter(Boolean);
      } catch {
        return [];
      }
    },
  };
}

// ---------------------------------------------------------------- the list

export type Summary = {
  canvas_id: string;
  title: string;
  updated_at: string;
  items: number;
  revision: number;
  group?: string;
};

/** Every canvas kept here, newest first. A body that does not parse is left out of the list, never thrown. */
export function listCanvases(kv: KV): Summary[] {
  const out: Summary[] = [];
  for (const k of kv.keys()) {
    if (!k.startsWith(BODY)) continue;
    const p = parseCanvas(kv.get(k) ?? "");
    if (!p.ok) continue;
    const c = p.canvas;
    out.push({
      canvas_id: c.canvas_id,
      title: c.title,
      updated_at: c.updated_at,
      items: c.items.length,
      revision: c.revision,
      ...(c.group ? { group: c.group } : {}),
    });
  }
  return out.sort((a, b) => (a.updated_at < b.updated_at ? 1 : a.updated_at > b.updated_at ? -1 : 0));
}

export function readCanvas(kv: KV, id: string): Canvas {
  const text = kv.get(BODY + id);
  if (text === null) throw new Error(NO_CANVAS);
  const p = parseCanvas(text);
  if (!p.ok) throw new Error(`This canvas can't be read: ${p.error}`);
  return p.canvas;
}

export function createCanvas(kv: KV, title: string, now = new Date().toISOString()): Canvas {
  const c = emptyCanvas(title.trim() || "Untitled canvas", now);
  kv.set(BODY + c.canvas_id, serialize(c));
  return c;
}

export function deleteCanvas(kv: KV, id: string) {
  kv.remove(BODY + id);
  kv.remove(VIEW + id);
  if (kv.get(LAST) === id) kv.remove(LAST);
}

/** A copy under a new id, titled "<title> (copy)". */
export function copyCanvas(kv: KV, id: string, now = new Date().toISOString()): Canvas {
  const c = readCanvas(kv, id);
  const copy: Canvas = {
    ...structuredClone(c),
    canvas_id: newId("cnv"),
    title: `${c.title} (copy)`,
    created_at: now,
    updated_at: now,
    revision: 0,
  };
  kv.set(BODY + copy.canvas_id, serialize(copy));
  return copy;
}

export function renameCanvas(kv: KV, id: string, title: string, now = new Date().toISOString()) {
  const c = readCanvas(kv, id);
  kv.set(BODY + id, serialize({ ...c, title: title.trim() || c.title, revision: c.revision + 1, updated_at: now }));
}

// ---------------------------------------------------------------- per-viewer view and last canvas

export function readView(kv: KV, id: string): View | null {
  try {
    const v = JSON.parse(kv.get(VIEW + id) ?? "null");
    if (v && [v.x, v.y, v.zoom].every((n) => typeof n === "number" && Number.isFinite(n)) && v.zoom > 0)
      return { x: v.x, y: v.y, zoom: v.zoom };
  } catch {
    // an unreadable view is no view
  }
  return null;
}

export function writeView(kv: KV, id: string, v: View) {
  try {
    kv.set(
      VIEW + id,
      JSON.stringify({ x: Math.round(v.x), y: Math.round(v.y), zoom: Math.round(v.zoom * 1000) / 1000 }),
    );
  } catch {
    // the view is not worth an error
  }
}

export const lastCanvas = (kv: KV): string | null => kv.get(LAST) || null;
export function rememberCanvas(kv: KV, id: string) {
  try {
    kv.set(LAST, id);
  } catch {
    // not kept
  }
}

// ---------------------------------------------------------------- the controller

export type Snapshot = {
  canvas: Canvas | null;
  error: string | null;
  saving: boolean;
  /** When the last save reached storage. */
  savedAt: string | null;
  past: number;
  future: number;
  /** Edits made and not yet saved. */
  dirty: boolean;
};

export type Clock = { set: (f: () => void, ms: number) => unknown; clear: (t: unknown) => void; now: () => string };
const realClock: Clock = {
  set: (f, ms) => setTimeout(f, ms),
  clear: (t) => clearTimeout(t as ReturnType<typeof setTimeout>),
  now: () => new Date().toISOString(),
};

export class Board {
  private cur: Canvas | null = null;
  private past: Canvas[] = [];
  private future: Canvas[] = [];
  private pending: ((c: Canvas) => Canvas)[] = [];
  private timer: unknown;
  private error: string | null = null;
  private savedAt: string | null = null;
  private subs = new Set<() => void>();
  private snap: Snapshot;

  readonly id: string;
  private kv: KV;
  private clock: Clock;

  constructor(id: string, kv: KV, clock: Clock = realClock) {
    this.id = id;
    this.kv = kv;
    this.clock = clock;
    this.snap = this.build();
    this.load();
  }

  private build(): Snapshot {
    return {
      canvas: this.cur,
      error: this.error,
      saving: false,
      savedAt: this.savedAt,
      past: this.past.length,
      future: this.future.length,
      dirty: this.pending.length > 0,
    };
  }
  private publish() {
    this.snap = this.build();
    for (const f of this.subs) f();
  }

  subscribe = (f: () => void) => {
    this.subs.add(f);
    return () => void this.subs.delete(f);
  };
  getSnapshot = (): Snapshot => this.snap;

  /** Reads the canvas from storage; edits made before it was read are replayed on it. */
  load() {
    try {
      let c = readCanvas(this.kv, this.id);
      for (const op of this.pending) c = op(c);
      this.cur = c;
      this.error = null;
    } catch (e) {
      this.error = e instanceof Error ? e.message : String(e);
    }
    this.publish();
  }

  /** Applies an edit now and saves it in a moment. `undoable: false` for the steps of a drag. */
  change(f: (c: Canvas) => Canvas, opts: { undoable?: boolean } = {}) {
    if (!this.cur) return;
    const before = this.cur;
    const after = f(before);
    if (after === before) return;
    if (opts.undoable !== false) {
      this.past = [...this.past.slice(-(HISTORY - 1)), before];
      this.future = [];
    }
    this.cur = after;
    this.pending.push(f);
    this.schedule();
    this.publish();
  }

  /** Marks the state a drag starts from, so the whole drag is one undo step. */
  checkpoint() {
    if (!this.cur) return;
    this.past = [...this.past.slice(-(HISTORY - 1)), this.cur];
    this.future = [];
    this.publish();
  }

  private replaceAll(to: Canvas) {
    this.cur = to;
    this.pending.push(() => to);
    this.schedule();
    this.publish();
  }

  undo() {
    const prev = this.past.pop();
    if (!prev || !this.cur) return;
    this.future.push(this.cur);
    this.replaceAll(prev);
  }

  redo() {
    const next = this.future.pop();
    if (!next || !this.cur) return;
    this.past.push(this.cur);
    this.replaceAll(next);
  }

  private schedule() {
    this.clock.clear(this.timer);
    this.timer = this.clock.set(() => this.flush(), SAVE_AFTER);
  }

  /** Saves now. A newer revision in storage (another tab) gets our unsaved edits replayed on it. */
  flush() {
    this.clock.clear(this.timer);
    if (!this.pending.length || !this.cur) return;
    const ops = this.pending;
    const now = this.clock.now();
    try {
      let base = this.cur;
      let replayed = false;
      const disk = this.kv.get(BODY + this.id);
      if (disk !== null) {
        const p = parseCanvas(disk);
        if (p.ok && p.canvas.revision > this.cur.revision) {
          let c = p.canvas;
          for (const op of ops) c = op(c);
          base = c;
          replayed = true;
        }
      }
      const saved: Canvas = { ...base, revision: (replayed ? base.revision : this.cur.revision) + 1, updated_at: now };
      this.kv.set(BODY + this.id, serialize(saved));
      this.cur = saved;
      this.pending = [];
      this.savedAt = now;
      this.error = null;
    } catch (e) {
      const full = e instanceof DOMException && (e.name === "QuotaExceededError" || e.code === 22);
      this.error = full
        ? "Not saved: this browser's storage is full. Delete an image or a canvas."
        : `Not saved: ${e instanceof Error ? e.message : String(e)}`;
    }
    this.publish();
  }

  /** Another tab saved this canvas: read it, unless there are edits of ours on the way. */
  external() {
    if (this.pending.length) return;
    const text = this.kv.get(BODY + this.id);
    if (text === null) return;
    const p = parseCanvas(text);
    if (p.ok && (!this.cur || p.canvas.revision > this.cur.revision)) {
      this.cur = p.canvas;
      this.publish();
    }
  }

  dispose() {
    this.flush();
    this.clock.clear(this.timer);
    this.subs.clear();
  }
}
