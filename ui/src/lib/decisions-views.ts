// The pure parts of the Decisions views (the account-wide list, filters, the figures on the
// overview and the command palette's ranking), kept apart from React so tests can run them.

export type ListDoc = {
  document_id: string;
  space_id?: string;
  kind: string;
  number?: number;
  display_number?: string;
  title: string;
  status?: string;
  stage?: string;
  summary?: string;
  created_by?: string;
  decider?: string;
  updated_at?: string;
  review_overdue?: boolean;
};

export const KIND_LABEL: Record<string, string> = { prd: "PRD", rfc: "RFC", adr: "ADR", study: "Study" };
export const KIND_ORDER = ["prd", "rfc", "adr", "study"];

export const numberOf = (d: Pick<ListDoc, "number" | "display_number">) =>
  d.display_number || String(d.number ?? "").padStart(4, "0");
export const docLabel = (d: Pick<ListDoc, "kind" | "number" | "display_number">) =>
  `${KIND_LABEL[d.kind] ?? d.kind.toUpperCase()} ${numberOf(d)}`;

export type Filters = {
  query: string;
  kind: string;
  status: string;
  stage: string;
  owner: string;
  /** "" any, else the last N days. */
  days: number;
  /** Only the documents this subject created or decides. */
  mine: string;
  space: string;
};
export const NO_FILTERS: Filters = {
  query: "",
  kind: "",
  status: "",
  stage: "",
  owner: "",
  days: 0,
  mine: "",
  space: "",
};

export type SortKey = "updated" | "number" | "title" | "status";

export function filterDocs(docs: readonly ListDoc[], f: Filters, now = Date.now()): ListDoc[] {
  const q = f.query.trim().toLowerCase();
  const since = f.days > 0 ? now - f.days * 86_400_000 : 0;
  return docs.filter((d) => {
    if (f.kind && d.kind !== f.kind) return false;
    if (f.status && d.status !== f.status) return false;
    if (f.stage && d.stage !== f.stage) return false;
    if (f.space && d.space_id !== f.space) return false;
    if (f.owner && d.created_by !== f.owner) return false;
    if (f.mine && d.created_by !== f.mine && d.decider !== f.mine) return false;
    if (since && !(d.updated_at && Date.parse(d.updated_at) >= since)) return false;
    if (q && !`${docLabel(d)} ${d.title} ${d.summary ?? ""}`.toLowerCase().includes(q)) return false;
    return true;
  });
}

export function sortDocs(docs: readonly ListDoc[], key: SortKey): ListDoc[] {
  const t = (d: ListDoc) => (d.updated_at ? Date.parse(d.updated_at) || 0 : 0);
  const by: Record<SortKey, (a: ListDoc, b: ListDoc) => number> = {
    updated: (a, b) => t(b) - t(a),
    number: (a, b) => KIND_ORDER.indexOf(a.kind) - KIND_ORDER.indexOf(b.kind) || (b.number ?? 0) - (a.number ?? 0),
    title: (a, b) => a.title.localeCompare(b.title),
    status: (a, b) => (a.status ?? "").localeCompare(b.status ?? "") || t(b) - t(a),
  };
  return [...docs].sort(by[key]);
}

/** How many documents have each status, most first. */
export function statusMix(docs: readonly ListDoc[]): { status: string; count: number }[] {
  const n = new Map<string, number>();
  for (const d of docs) n.set(d.status || "none", (n.get(d.status || "none") ?? 0) + 1);
  return [...n]
    .map(([status, count]) => ({ status, count }))
    .sort((a, b) => b.count - a.count || a.status.localeCompare(b.status));
}

/** How many documents are of each kind, in the kinds' order. */
export function kindMix(docs: readonly ListDoc[]): { kind: string; count: number }[] {
  return KIND_ORDER.map((kind) => ({ kind, count: docs.filter((d) => d.kind === kind).length })).filter(
    (k) => k.count > 0,
  );
}

/** Distinct non-empty values of a field, sorted. */
export const distinct = (docs: readonly ListDoc[], f: "status" | "stage" | "created_by"): string[] =>
  [...new Set(docs.map((d) => d[f]).filter((x): x is string => !!x))].sort();

// ---------------------------------------------------------------- command palette

export type Entry = { id: string; label: string; hint?: string; group: "View" | "Space" | "Document"; go: string };

/** Entries whose label has every word of `query` (any order); the label's start and whole words rank first. */
export function rankEntries(entries: readonly Entry[], query: string, max = 12): Entry[] {
  const words = query.trim().toLowerCase().split(/\s+/).filter(Boolean);
  if (words.length === 0) return entries.slice(0, max);
  const scored: [number, Entry][] = [];
  for (const e of entries) {
    const hay = `${e.label} ${e.hint ?? ""}`.toLowerCase();
    if (!words.every((w) => hay.includes(w))) continue;
    let s = 0;
    const label = e.label.toLowerCase();
    for (const w of words) {
      if (label.startsWith(w)) s += 3;
      else if (label.includes(` ${w}`)) s += 2;
      else s += 1;
    }
    scored.push([s, e]);
  }
  return scored
    .sort((a, b) => b[0] - a[0])
    .slice(0, max)
    .map(([, e]) => e);
}

/** Moves a selection by `by` within `n` rows, staying inside them. */
export const step = (at: number, by: number, n: number) => (n === 0 ? 0 : Math.max(0, Math.min(n - 1, at + by)));

// ---------------------------------------------------------------- the route

export type Route =
  | { view: "overview" }
  | { view: "waiting" }
  | { view: "mine" }
  | { view: "all"; kind?: string; status?: string; stage?: string }
  | { view: "queue" }
  | { view: "prs"; repo?: string }
  | { view: "diagrams"; id?: string; space?: string }
  | { view: "new"; space: string }
  | { view: "new-space" }
  | { view: "settings"; space: string }
  | { view: "timeline"; space: string }
  | { view: "space"; space: string }
  | { view: "doc"; space: string; doc: string };

/** The nav item after "decisions": "" overview, "~waiting" (and the old "~review"), "~mine", "~all",
 * "~diagrams[/space/id]", "~new/<space>", "~new-space", "<space>", "<space>/~timeline", "<space>/~settings", "<space>/<doc>". */
export function parseRoute(item: string | undefined): Route {
  const [a = "", b = "", c = ""] = (item ?? "").split("/");
  if (a === "") return { view: "overview" };
  if (a === "~waiting" || a === "~review") return { view: "waiting" };
  if (a === "~mine") return { view: "mine" };
  if (a === "~all")
    return {
      view: "all",
      ...Object.fromEntries(
        b
          .split(",")
          .filter((p) => /^(kind|status|stage):./.test(p))
          .map((p) => [p.slice(0, p.indexOf(":")), p.slice(p.indexOf(":") + 1)]),
      ),
    };
  if (a === "~queue") return { view: "queue" };
  if (a === "~prs") return b && c ? { view: "prs", repo: `${b}/${c}` } : { view: "prs" };
  if (a === "~diagrams") return b ? { view: "diagrams", space: b, id: c || undefined } : { view: "diagrams" };
  if (a === "~new-space") return { view: "new-space" };
  if (a === "~new") return b ? { view: "new", space: b } : { view: "all" };
  if (b === "~timeline") return { view: "timeline", space: a };
  if (b === "~settings") return { view: "settings", space: a };
  if (b) return { view: "doc", space: a, doc: b };
  return { view: "space", space: a };
}

// ---------------------------------------------------------------- the enterprise rail

/** The "All documents" route for a kind, status or stage ("~all/kind:rfc,status:draft"). */
export const allRoute = (f: { kind?: string; status?: string; stage?: string } = {}) => {
  const parts = (["kind", "status", "stage"] as const).filter((k) => f[k]).map((k) => `${k}:${f[k]}`);
  return parts.length ? `~all/${parts.join(",")}` : "~all";
};

/** The filters a route sets, on top of none. */
export const filtersOf = (r: { kind?: string; status?: string; stage?: string }): Filters => ({
  ...NO_FILTERS,
  kind: r.kind ?? "",
  status: r.status ?? "",
  stage: r.stage ?? "",
});

/** Documents by type in the order PRD, RFC, ADR, Study, then any other kind; kinds with none are kept (count 0). */
export function typeCounts(docs: readonly ListDoc[]): { kind: string; label: string; count: number }[] {
  const n = new Map<string, number>(KIND_ORDER.map((k) => [k, 0]));
  for (const d of docs) n.set(d.kind, (n.get(d.kind) ?? 0) + 1);
  return [...n].map(([kind, count]) => ({ kind, label: KIND_LABEL[kind] ?? kind.toUpperCase(), count }));
}

const LIFECYCLE = [
  "draft",
  "proposed",
  "in_review",
  "accepted",
  "running",
  "measured",
  "superseded",
  "rejected",
  "withdrawn",
  "abandoned",
];
const rankOf = (v: string) => (LIFECYCLE.includes(v) ? LIFECYCLE.indexOf(v) : LIFECYCLE.length);

/** The statuses, and separately the delivery stages, that exist in the documents, in lifecycle order, with counts. */
export function lifecycleCounts(
  docs: readonly ListDoc[],
  field: "status" | "stage",
): { value: string; count: number }[] {
  const n = new Map<string, number>();
  for (const d of docs) {
    const v = d[field];
    if (v) n.set(v, (n.get(v) ?? 0) + 1);
  }
  return [...n]
    .map(([value, count]) => ({ value, count }))
    .sort((a, b) => rankOf(a.value) - rankOf(b.value) || a.value.localeCompare(b.value));
}

/** The most recently changed documents, newest first. */
export const recentDocs = (docs: readonly ListDoc[], max = 6): ListDoc[] =>
  sortDocs(
    docs.filter((d) => d.updated_at),
    "updated",
  ).slice(0, max);

/** A pull request a document's comment names the way `link-pr` writes it ("owner/repo#12: merged"), with the word after the colon. */
export type PrLine = { repo: string; number: number; state: string };
export function prLines(comments: readonly { body: string; created_at?: string }[]): PrLine[] {
  const latest = new Map<string, { at: number; line: PrLine }>();
  comments.forEach((c, i) => {
    for (const m of c.body.matchAll(/([A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+)#(\d+)(?::\s*([A-Za-z-]+))?/g)) {
      const line = { repo: m[1]!, number: Number(m[2]), state: (m[3] ?? "").toLowerCase() };
      const key = `${line.repo}#${line.number}`;
      const at = (c.created_at ? Date.parse(c.created_at) : 0) || i;
      const had = latest.get(key);
      if (!had || (line.state && at >= had.at) || (!had.line.state && !line.state))
        latest.set(key, { at, line: line.state || !had ? line : { ...line, state: had.line.state } });
    }
  });
  return [...latest.values()].map((v) => v.line).sort((a, b) => b.number - a.number);
}
