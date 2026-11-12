// The coordinator's picture as the page folds it: one snapshot, then deltas by seq (WatchCoordinator,
// GET /v1/coordinator/picture/events). Pure, so the fold is tested without a stream. A port of the Mac
// app's coord-work.ts folding (applyFrame), kept to what the 3D workload page reads.

export type Rec = Record<string, unknown>;
export const rec = (v: unknown): Rec => (v && typeof v === "object" && !Array.isArray(v) ? (v as Rec) : {});
export const list = (v: unknown): Rec[] =>
  Array.isArray(v) ? v.filter((x) => x && typeof x === "object").map(rec) : [];
const camel = (k: string) => k.replace(/_([a-z])/g, (_, c: string) => c.toUpperCase());
/** A string field by its proto name or its JSON camelCase name. */
export const str = (o: Rec, k: string): string => {
  const v = o[k] ?? o[camel(k)];
  return typeof v === "string" ? v : typeof v === "number" ? String(v) : "";
};
export const num = (o: Rec, k: string): number => {
  const v = o[k] ?? o[camel(k)];
  return typeof v === "number" ? v : typeof v === "string" && v.trim() && Number.isFinite(Number(v)) ? Number(v) : 0;
};
export const strs = (o: Rec, k: string): string[] => {
  const v = o[k] ?? o[camel(k)];
  return Array.isArray(v) ? v.filter((x): x is string => typeof x === "string") : [];
};

export type Tier = "full" | "redacted" | "unreported";

export type Picture = {
  /** The highest seq applied: resume with from_seq=seq. */
  seq: number;
  tier: Tier;
  jobs: Map<string, Rec>;
  runners: Map<string, Rec>;
  asks: Map<string, Rec>;
  open: Map<string, Rec>;
  transitions: Rec[];
};

export const emptyPicture = (): Picture => ({
  seq: 0,
  tier: "unreported",
  jobs: new Map(),
  runners: new Map(),
  asks: new Map(),
  open: new Map(),
  transitions: [],
});

const tierOf = (o: Rec): Tier => {
  const t = str(o, "tier").toLowerCase();
  return t === "full" ? "full" : t === "public" || t === "redacted" ? "redacted" : "unreported";
};

const byId = (xs: Rec[]): Map<string, Rec> =>
  new Map(xs.map((x) => [str(x, "id") || str(x, "job_id"), x] as [string, Rec]).filter(([k]) => !!k));

/** One frame of the watch. A snapshot replaces everything; a delta upserts or removes one entity. */
export function applyFrame(p: Picture, raw: unknown): Picture {
  let f = rec(raw);
  if (f.result && typeof f.result === "object") f = rec(f.result); // the gateway wraps a stream message in {"result": ...}
  if (f.snapshot) {
    const o = rec(f.snapshot);
    return {
      seq: num(o, "seq"),
      tier: tierOf(o),
      jobs: byId(list(o.jobs)),
      runners: byId(list(o.runners)),
      asks: byId(list(o.asks)),
      open: byId(list(o.open_items ?? o.openItems)),
      transitions: list(o.recent_transitions ?? o.recentTransitions).slice(-50),
    };
  }
  if (f.delta) {
    const d = rec(f.delta);
    const next: Picture = {
      ...p,
      seq: Math.max(p.seq, num(d, "seq")),
      jobs: new Map(p.jobs),
      runners: new Map(p.runners),
      asks: new Map(p.asks),
      open: new Map(p.open),
    };
    const entity = str(d, "entity");
    const id = str(d, "id");
    const maps: Record<string, Map<string, Rec>> = {
      job: next.jobs,
      runner: next.runners,
      ask: next.asks,
      open_item: next.open,
    };
    if (entity === "transition") {
      const t = rec(d.transition);
      if (Object.keys(t).length) next.transitions = [...p.transitions, t].slice(-50);
    } else if (maps[entity] && id) {
      if (str(d, "op") === "remove") maps[entity].delete(id);
      else maps[entity].set(id, rec(d[entity]));
    }
    return next;
  }
  if (f.heartbeat) return { ...p, seq: Math.max(p.seq, num(rec(f.heartbeat), "seq")) };
  return p;
}
