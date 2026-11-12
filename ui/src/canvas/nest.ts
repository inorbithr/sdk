// The canvas switcher's tree: which nest each canvas belongs to. Pure, so a new kind of canvas
// lands in the right place without touching the picker (tests/canvas-nest.test.ts).
//
// The nest comes from what the canvas is (its id, then its title). A canvas can say it itself with
// `group` ("Coordinator/Older layouts"), which wins over every rule here. Nests nest to any depth.

export type Entry = { canvas_id: string; title: string; group?: string };
export type Leaf = { kind: "leaf"; id: string; label: string; path: string[] };
export type Group = { kind: "group"; key: string; label: string; children: Node[] };
export type Node = Leaf | Group;
export type Row = { node: Node; depth: number; path: string[] };

export const NEST = {
  coordinator: "Coordinator",
  understanding: "Understanding versions",
  older: "Older layouts, kept",
  decisions: "Decisions",
  plans: "Plans",
  asks: "Asks & approvals",
  jobs: "Jobs",
  guides: "Guides & demos",
  other: "Other",
} as const;
const TOP_ORDER: string[] = [
  NEST.coordinator,
  NEST.decisions,
  NEST.plans,
  NEST.asks,
  NEST.jobs,
  NEST.guides,
  NEST.other,
];

/** A layout kept when a board was redone (`<id>_v1`). */
export const isKept = (id: string) => /_v\d+(-\d+)?$/.test(id);

const KEPT_NOTE = /\s*\((?:((?:first|v[\d.]+) layout), )?(?:kept|old layout)\)/gi;

/** The label a canvas shows inside its nest: the title, without the words its nest already says (a kept layout keeps which one it is). */
export function labelOf(e: Entry): string {
  const t = e.title.trim() || "Untitled canvas";
  if (!isKept(e.canvas_id)) return t;
  let which = "";
  const base =
    t
      .replace(KEPT_NOTE, (_m, w?: string) => {
        if (w && !which) which = w;
        return "";
      })
      .trim() || t;
  const n = /_v(\d+(?:-\d+)?)$/.exec(e.canvas_id)?.[1];
  return `${base}, ${which || (n ? `v${n}` : "older")}`;
}

/** The nest of one canvas, outermost first; [] is the top level, no nest. */
export function groupPath(e: Entry): string[] {
  if (e.group && e.group.trim())
    return e.group
      .split("/")
      .map((s) => s.trim())
      .filter(Boolean);
  const id = e.canvas_id;
  const title = e.title.trim();
  if (id.startsWith("cnv_coordinator-understanding")) return [NEST.coordinator, NEST.understanding];
  if (id.startsWith("cnv_coordinator")) return isKept(id) ? [NEST.coordinator, NEST.older] : [NEST.coordinator];
  if (/^coordinator, as it really is/i.test(title)) return [NEST.coordinator, NEST.understanding];
  if (/^(distributed and programmable )?coordinator as a service/i.test(title))
    return isKept(id) ? [NEST.coordinator, NEST.older] : [NEST.coordinator];
  if (id.startsWith("cnv_plan-") || id.startsWith("cnv_plnh-")) return [NEST.plans];
  if (id.startsWith("cnv_ask-")) return [NEST.asks];
  if (id.startsWith("cnv_job-")) return [NEST.jobs];
  if (/^prd\b/i.test(title)) return [NEST.decisions, "PRDs"];
  if (/^rfc\b/i.test(title)) return [NEST.decisions, "RFCs"];
  if (/^adr\b/i.test(title)) return [NEST.decisions, "ADRs"];
  if (/\bstudy \d+/i.test(title) || /^study\b/i.test(title)) return [NEST.decisions, "Studies"];
  if (id.startsWith("cnv_dec-")) return [NEST.decisions];
  if (/^(testflight|run the |roll |deploy )/i.test(title)) return [NEST.jobs];
  if (
    /^how /i.test(title) ||
    /\b(demo|guide)\b/i.test(title) ||
    /dev loop/i.test(title) ||
    id.startsWith("cnv_extensions-") ||
    id.startsWith("cnv_build-your-own")
  )
    return [NEST.guides];
  return [NEST.other];
}

type Mut = { kind: "group"; key: string; label: string; children: Node[] };

/** The tree of a list. A nest with one child shows that child inline, in its place. */
export function buildTree(list: Entry[]): Node[] {
  const root: Mut = { kind: "group", key: "", label: "", children: [] };
  const seen = new Set<string>();
  for (const e of list) {
    if (seen.has(e.canvas_id)) continue;
    seen.add(e.canvas_id);
    const path = groupPath(e);
    let at = root;
    for (const seg of path) {
      const key = at.key ? `${at.key}/${seg}` : seg;
      let g = at.children.find((c): c is Mut => c.kind === "group" && c.key === key);
      if (!g) {
        g = { kind: "group", key, label: seg, children: [] };
        at.children.push(g);
      }
      at = g;
    }
    at.children.push({ kind: "leaf", id: e.canvas_id, label: labelOf(e), path });
  }
  return flatten(sort(root.children, true));
}

/** Replaces a nest that holds one thing with that thing. */
function flatten(nodes: Node[]): Node[] {
  return nodes.map((n) => {
    if (n.kind === "leaf") return n;
    const kids = flatten(n.children);
    return kids.length === 1 ? kids[0] : { ...n, children: kids };
  });
}

/** Inside a nest: canvases first, then nests, "Older layouts" last. At the top: the suggested order. */
function sort(nodes: Node[], top = false): Node[] {
  const rank = (n: Node) => {
    if (n.kind === "leaf") return 0;
    if (top) {
      const i = TOP_ORDER.indexOf(n.label);
      return 1 + (i < 0 ? TOP_ORDER.length - 1 : i);
    }
    return n.label === NEST.older ? 99 : 1;
  };
  return nodes
    .map((n, i) => ({ n: n.kind === "group" ? { ...n, children: sort(n.children) } : n, i }))
    .sort((a, b) => rank(a.n) - rank(b.n) || a.i - b.i)
    .map((x) => x.n);
}

/** The nests that hold a canvas, as keys (the ones to open so it shows). */
export function ancestorsOf(tree: Node[], id: string): string[] {
  const walk = (nodes: Node[], acc: string[]): string[] | null => {
    for (const n of nodes) {
      if (n.kind === "leaf") {
        if (n.id === id) return acc;
      } else {
        const r = walk(n.children, [...acc, n.key]);
        if (r) return r;
      }
    }
    return null;
  };
  return walk(tree, []) ?? [];
}

/** The rows on screen: every nest, and the children of the open ones. */
export function visibleRows(tree: Node[], open: ReadonlySet<string>): Row[] {
  const out: Row[] = [];
  const walk = (nodes: Node[], depth: number, path: string[]) => {
    for (const n of nodes) {
      out.push({ node: n, depth, path });
      if (n.kind === "group" && open.has(n.key)) walk(n.children, depth + 1, [...path, n.label]);
    }
  };
  walk(tree, 0, []);
  return out;
}

/** A search: the canvases whose title or nest holds every word, each with its path. */
export function searchRows(tree: Node[], query: string): Leaf[] {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (!words.length) return [];
  const out: Leaf[] = [];
  const walk = (nodes: Node[]) => {
    for (const n of nodes) {
      if (n.kind === "group") walk(n.children);
      else {
        const hay = `${n.path.join(" ")} ${n.label} ${n.id}`.toLowerCase();
        if (words.every((w) => hay.includes(w))) out.push(n);
      }
    }
  };
  walk(tree);
  return out;
}
