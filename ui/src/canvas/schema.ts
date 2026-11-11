// A canvas file (PRD 0065): one board of typed items, the links between them and the threads on
// them, as JSON. Versioned, with stable ids, and shaped like an API resource (snake_case, ids
// with a prefix, RFC 3339 times, a revision for optimistic saves) so it can move from
// ~/.inorbit/canvases/<canvas_id>.json to a core API later without a rewrite. Pure, so it is
// tested on its own (tests/canvas-schema.test.ts). The full description is src/canvas/README.md.

export const SCHEMA = "iohr.canvas";
export const VERSION = 3;

export type ItemType =
  | "document"
  | "diagram"
  | "note"
  | "frame"
  | "shape"
  | "image"
  | "ask"
  | "option"
  | "pr"
  | "decision";
export type Tone = "plain" | "amber" | "green" | "blue" | "violet" | "red";
export const TONES: readonly Tone[] = ["plain", "amber", "green", "blue", "violet", "red"];

/** Where an item sits on the board, in board units (1 unit = 1 CSS pixel at 100%). */
export type Box = { x: number; y: number; w: number; h: number };

type ItemBase = Box & {
  /** "itm_" and a time-ordered suffix; never reused, never changed. */
  item_id: string;
  type: ItemType;
  title: string;
  /** Collapsed items show their header only. */
  collapsed?: boolean;
  /** The section frame it sits in (version 2): moving the frame moves it. */
  frame_id?: string;
};

/** A live Decisions document: PRD, RFC, ADR or study. The text is never copied in; it is read. */
export type DocumentItem = ItemBase & {
  type: "document";
  document: { space_id: string; document_id: string; kind?: string; display_number?: string };
  /** Heading anchors (Decisions heading ids) folded on this board. */
  folded?: string[];
  /** Pull requests that implement it, as URLs; PRs found in the text are added when shown. */
  prs?: string[];
};

export type NodeKind =
  | "person"
  | "client"
  | "edge"
  | "service"
  | "part"
  | "crate"
  | "rpc"
  | "function"
  | "store"
  | "runner"
  | "ai"
  | "git"
  | "state"
  | "personality"
  | "external"
  | "gate"
  | "placeholder"
  /** A Personality drawn as a card (a rounded box with room for its role and live line), where a diamond would not hold it. */
  | "persona";
export const NODE_KINDS: readonly NodeKind[] = [
  "person",
  "client",
  "edge",
  "service",
  "part",
  "crate",
  "rpc",
  "function",
  "store",
  "runner",
  "ai",
  "git",
  "state",
  "personality",
  "external",
  "gate",
  "placeholder",
  "persona",
];

export type DiagramNode = {
  /** Stable within its diagram: lowercase letters, digits and "-" (it doubles as a comment anchor). */
  id: string;
  label: string;
  kind: NodeKind;
  /** One or two short lines under the label (a crate path, a signature, a state's gate). */
  detail?: string;
  /** A group node's id: drawn as a box around its members (the service around its parts). */
  group?: string;
  /** Position inside the frame, top-left, set by the layout or by a drag. */
  x?: number;
  y?: number;
  /** Moved by hand: kept where it is until "Lay out again". */
  pinned?: boolean;
  /** Its size, when someone resized it; otherwise it is sized from its label and detail. */
  w?: number;
  h?: number;
  /** Where it is written down: a Decisions document id and, optionally, its heading anchor. */
  ref?: { document_id: string; anchor?: string };
  /**
   * Where the part stands, for boards that track a build (the extensions board): "built" (on main),
   * "building" (a branch or a PR is open) or "planned". Drawn on the box: planned is dashed, being
   * built has an amber edge.
   */
  status?: BuildStatus;
  /**
   * The trust zone it sits in (version 3), which sets its colour: InOrbit's service, the user's own
   * machines (where keys and model traffic live), or outside. Left out, it follows the kind.
   */
  zone?: Zone;
  /** The level frame (an item id) a click on it zooms into (version 3): the box has children. */
  opens?: string;
  /**
   * A live number shown on the box, read from the app each time the board is shown (live.ts): a key
   * like "jobs7d:legal", "lessons", "memory", "governor", "queued". Never stored as a value.
   */
  live?: string;
};

export type BuildStatus = "built" | "building" | "planned";
export const BUILD_STATUSES: readonly BuildStatus[] = ["built", "building", "planned"];

export type Zone = "inorbit" | "user" | "outside";
export const ZONES: readonly Zone[] = ["inorbit", "user", "outside"];

/** The edge styles: "call" (a request), "event" (an event or a stream), "lease" (a runner's dial-out lease), and the older ones. */
export type EdgeStyle = "flow" | "call" | "data" | "fail" | "learn" | "event" | "lease";
export const EDGE_STYLES: readonly EdgeStyle[] = ["flow", "call", "data", "fail", "learn", "event", "lease"];

export type DiagramEdge = {
  id: string;
  source: string;
  target: string;
  label?: string;
  /** "flow" (default), "call" (RPC), "data", "fail" (to Held, Retry, Stopped), "learn"; version 3 adds "event" (an event or a stream) and "lease" (a dial-out lease). */
  style?: EdgeStyle;
  /** The bends between its ends, relative to the diagram (set by the layout or by hand). */
  points?: [number, number][];
};

/**
 * A group of nodes. A collapsed group is drawn as one box (its members hidden, their edges bundled
 * into one per neighbour); `x`, `y`, `w`, `h` are that box's place when collapsed.
 */
export type DiagramGroup = {
  id: string;
  label: string;
  kind?: NodeKind;
  collapsed?: boolean;
  x?: number;
  y?: number;
  w?: number;
  h?: number;
};

/** The model saved as a Decisions diagram: `version`, `nodes`, `edges` are what the API checks; the rest it keeps. */
export type DiagramModel = {
  version: number;
  direction: "TB" | "LR";
  nodes: DiagramNode[];
  edges: DiagramEdge[];
  groups?: DiagramGroup[];
  /** "layered" (the default) or "pack": groups laid out on their own, then placed together. */
  layout?: "layered" | "pack";
  /** The bends of bundled edges (to or from a collapsed group), by `source>target`, set by the layout. */
  routes?: Record<string, [number, number][]>;
};

export type DiagramItem = ItemBase & {
  type: "diagram";
  model: DiagramModel;
  /** Set once it is saved to Decisions (decisions_save_diagram); `version` is the one last read or saved. */
  diagram?: { space_id: string; diagram_id: string; version: number };
  /** Says it is a placeholder another worker fills (README, "Filling a placeholder"). */
  placeholder?: { owner: string; source: string; note: string };
};

export type NoteItem = ItemBase & { type: "note"; text: string; tone?: Tone };

/** A section frame: a titled area behind other items, with an optional line of text. */
export type FrameItem = ItemBase & { type: "frame"; text?: string; tone?: Tone };

export type ShapeKind = "rectangle" | "ellipse" | "diamond" | "arrow" | "line" | "text" | "freedraw";
export const SHAPE_KINDS: readonly ShapeKind[] = [
  "rectangle",
  "ellipse",
  "diamond",
  "arrow",
  "line",
  "text",
  "freedraw",
];

/**
 * A free shape drawn on the board (version 2): a box, an ellipse, a diamond, an arrow, a line,
 * a text or a pen stroke. Colours are CSS colours; points are relative to the item's x and y.
 * Arrows say what they are bound to (`start`, `end`), so they follow it when it moves.
 */
export type ShapeItem = ItemBase & {
  type: "shape";
  shape: ShapeKind;
  text?: string;
  /** An architecture kind from the library (a service, an RPC, a gate). */
  kind?: NodeKind;
  detail?: string;
  angle?: number;
  points?: [number, number][];
  start?: Target;
  end?: Target;
  arrowheads?: { start?: string; end?: string };
  elbow?: boolean;
  style?: {
    stroke?: string;
    fill?: string;
    stroke_width?: number;
    stroke_style?: "solid" | "dashed" | "dotted";
    fill_style?: string;
    rounded?: boolean;
    opacity?: number;
    font_size?: number;
    align?: "left" | "center" | "right";
  };
  /** Shapes grouped together move together. */
  groups?: string[];
};

/** What an image's `src` may be: a PNG, WebP or JPEG as a base64 data URL, so the canvas file holds the picture itself. */
export const IMAGE_SRC_RE = /^data:image\/(png|webp|jpeg);base64,[A-Za-z0-9+/]+={0,2}$/;
/** The longest `src` kept, in characters (about 4 MB of text): one screenshot, not a library. */
export const MAX_IMAGE_SRC = 4_000_000;

/** A picture on the board, pasted or dropped. It lives in the file (no side store) and moves, resizes and sits in frames like any item. */
export type ImageItem = ItemBase & { type: "image"; src: string; alt?: string; natural_w?: number; natural_h?: number };

/** The question of an ask (PRD 0071): the centre of a canvas made from an ask. Its state is read live from the ledger. */
export type AskItem = ItemBase & { type: "ask"; ask_id: string; text: string; asked_at?: string; by?: string };

/** One answer of an ask, as a card: what it does. Approve or Decline. */
export type OptionItem = ItemBase & { type: "option"; ask_id: string; option: "approve" | "deny"; means: string };

/** A pull request the ask is about: its checks and diff are read live from GitHub. */
export type PrItem = ItemBase & { type: "pr"; repo: string; number: number };

/** A decision (PRD, RFC, ADR or study) named by its number; the card finds it in Decisions when shown. */
export type DecisionItem = ItemBase & { type: "decision"; kind: "prd" | "rfc" | "adr" | "study"; number: string };

/** The cards of an ask canvas: drawn by the board's embeddables like a document. */
export type AskCardItem = AskItem | OptionItem | PrItem | DecisionItem;
export const ASK_CARDS: readonly string[] = ["ask", "option", "pr", "decision"];

export type Item =
  | DocumentItem
  | DiagramItem
  | NoteItem
  | FrameItem
  | ShapeItem
  | ImageItem
  | AskItem
  | OptionItem
  | PrItem
  | DecisionItem;

/** What a link or a thread points at: an item, and optionally one part of it. */
export type Target = {
  item_id: string;
  /** A diagram node or edge id, or a document heading anchor. */
  element?: string;
  /** A passage inside a document section (the Decisions quote shape, RFC 0062). */
  quote?: Quote;
};

export type Quote = { exact: string; prefix?: string; suffix?: string; start?: number };

export type Link = {
  /** "lnk_…" */
  link_id: string;
  from: Target;
  to: Target;
  /** "implements", "decides", "measures", "context", "pr" or anything short. */
  kind: string;
  label?: string;
};

/**
 * A thread kept with the canvas, for parts Decisions can't hold comments on yet (diagram nodes
 * and edges, notes, frames). Threads on a document live in Decisions and are never copied here.
 * The shape is the Decisions comment's, so they can move to an API unchanged.
 */
export type Comment = {
  comment_id: string;
  parent_id?: string;
  author: string;
  body: string;
  created_at: string;
  resolved?: boolean;
  resolved_at?: string;
};
/** The coordinator answering a thread: asked (not sent yet), waiting for its answer, answered, or failed. */
export type ThreadAi = { status: "asking" | "waiting" | "done" | "failed"; asked_at?: string; note?: string };
export type Thread = { thread_id: string; target: Target; comments: Comment[]; ai?: ThreadAi };

/** A walk-through step: it lights boxes and lines of a diagram frame, or whole items (notes) of a plain frame (`items`). */
export type StoryStep = {
  title: string;
  caption: string;
  frame: string;
  nodes: string[];
  edges: string[];
  items?: string[];
};
/** A walk-through a canvas file carries for itself. `close`: a step on a level too big to read whole frames the boxes it lights. */
export type CanvasStory = { id: string; title: string; steps: StoryStep[]; close?: boolean };

export type Canvas = {
  schema: typeof SCHEMA;
  version: typeof VERSION;
  /** "cnv_…" */
  canvas_id: string;
  title: string;
  description?: string;
  /** Where the canvas switcher nests it ("Coordinator/Older layouts"); when absent the nest is derived (nest.ts). */
  group?: string;
  /** The account and Decisions space its documents come from. */
  account_id?: string;
  space_id?: string;
  created_at: string;
  updated_at: string;
  /** Goes up by one on every save; a save made from an older revision is a conflict. */
  revision: number;
  /** "clean" (the default: straight, architect lines) or "sketch" (softly hand-drawn). */
  style?: "clean" | "sketch";
  /** A canvas that follows a plan (the Planner, PRD 0111): the plan, and what the board last took from it, one signature per card and link. */
  follows?: { plan_id: string; seen: Record<string, string> };
  items: Item[];
  links: Link[];
  threads: Thread[];
  /** Its own walk-throughs, shown beside the built-in ones when every frame, box and line they light is on it. */
  stories?: CanvasStory[];
};

// ---------------------------------------------------------------- ids

const ALPHA = "0123456789abcdefghjkmnpqrstvwxyz"; // Crockford, lowercase
let last = 0;
let seq = 0;

/** A time-ordered id with a prefix: `itm_`, `lnk_`, `thr_`, `cmt_`, `cnv_`. Unique within a process. */
export function newId(prefix: string, now = Date.now()): string {
  if (now === last) seq++;
  else {
    last = now;
    seq = 0;
  }
  let t = now;
  let time = "";
  for (let i = 0; i < 9; i++) {
    time = ALPHA[t % 32] + time;
    t = Math.floor(t / 32);
  }
  let rand = "";
  for (const b of crypto.getRandomValues(new Uint8Array(6))) rand += ALPHA[b % 32];
  return `${prefix}_${time}${seq.toString(32).padStart(2, "0")}${rand}`;
}

export const ID_RE = /^[a-z]{3}_[0-9a-z_-]{1,80}$/;
export const ELEMENT_RE = /^[a-z0-9][a-z0-9-]{0,199}$/;

/** A diagram element id from a label: what a heading anchor would be (Decisions' slugify), at most 60. */
export function elementId(label: string, taken: Set<string> = new Set()): string {
  let base = "";
  for (const c of label.toLowerCase()) {
    if (/[a-z0-9]/.test(c)) base += c;
    else if (base && !base.endsWith("-")) base += "-";
    if (base.length >= 60) break;
  }
  base = base.replace(/-+$/, "") || "node";
  let id = base;
  for (let n = 2; taken.has(id); n++) id = `${base}-${n}`;
  return id;
}

// ---------------------------------------------------------------- parsing

/** `from` is the version the file was written in (0 for the first drafts). */
export type Parsed =
  | { ok: true; canvas: Canvas; migrated: boolean; dropped: string[]; from: number }
  | { ok: false; error: string };

type Obj = Record<string, unknown>;
const isObj = (v: unknown): v is Obj => !!v && typeof v === "object" && !Array.isArray(v);
const str = (v: unknown, d = ""): string => (typeof v === "string" ? v : d);
const num = (v: unknown, d = 0): number => (typeof v === "number" && Number.isFinite(v) ? v : d);
const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));
const tone = (v: unknown): Tone | undefined =>
  (TONES as readonly string[]).includes(str(v)) ? (v as Tone) : undefined;

function box(o: Obj): Box {
  return {
    x: clamp(num(o.x), -1e6, 1e6),
    y: clamp(num(o.y), -1e6, 1e6),
    w: clamp(num(o.w, 320), 40, 20_000),
    h: clamp(num(o.h, 200), 24, 20_000),
  };
}

function target(v: unknown): Target | null {
  if (!isObj(v) || !str(v.item_id)) return null;
  const t: Target = { item_id: str(v.item_id) };
  if (str(v.element)) t.element = str(v.element);
  if (isObj(v.quote) && str(v.quote.exact)) {
    const q = v.quote;
    t.quote = { exact: str(q.exact).slice(0, 1000) };
    if (str(q.prefix)) t.quote.prefix = str(q.prefix).slice(-64);
    if (str(q.suffix)) t.quote.suffix = str(q.suffix).slice(0, 64);
    if (typeof q.start === "number") t.quote.start = Math.max(0, Math.floor(q.start));
  }
  return t;
}

function points(v: unknown): [number, number][] | undefined {
  if (!Array.isArray(v)) return undefined;
  const out: [number, number][] = [];
  for (const p of v.slice(0, 2000))
    if (Array.isArray(p) && Number.isFinite(p[0]) && Number.isFinite(p[1]))
      out.push([clamp(p[0], -1e6, 1e6), clamp(p[1], -1e6, 1e6)]);
  return out.length ? out : undefined;
}

const CSS_COLOR = /^(#[0-9a-f]{3,8}|transparent|(rgb|rgba|hsl|hsla|oklch)\([0-9.,%\s/-]+\))$/i;
function shapeStyle(v: unknown): ShapeItem["style"] | undefined {
  if (!isObj(v)) return undefined;
  const o: NonNullable<ShapeItem["style"]> = {};
  if (CSS_COLOR.test(str(v.stroke))) o.stroke = str(v.stroke);
  if (CSS_COLOR.test(str(v.fill))) o.fill = str(v.fill);
  if (typeof v.stroke_width === "number" && v.stroke_width > 0) o.stroke_width = clamp(v.stroke_width, 0.5, 16);
  if (["solid", "dashed", "dotted"].includes(str(v.stroke_style))) o.stroke_style = v.stroke_style as "solid";
  if (["solid", "hachure", "cross-hatch", "zigzag"].includes(str(v.fill_style))) o.fill_style = str(v.fill_style);
  if (typeof v.rounded === "boolean") o.rounded = v.rounded;
  if (typeof v.opacity === "number") o.opacity = clamp(v.opacity, 0, 100);
  if (typeof v.font_size === "number") o.font_size = clamp(v.font_size, 6, 400);
  if (["left", "center", "right"].includes(str(v.align))) o.align = v.align as "left";
  return Object.keys(o).length ? o : undefined;
}

/** A live key: a name and, after a colon, what it is about ("jobs7d:legal"). */
export const LIVE_RE = /^[a-z][a-z0-9]*(?::[a-z0-9-]{1,48})?$/;

export function parseModel(v: unknown): DiagramModel {
  const o = isObj(v) ? v : {};
  const nodes: DiagramNode[] = [];
  const seen = new Set<string>();
  for (const n of Array.isArray(o.nodes) ? o.nodes : []) {
    if (!isObj(n) || !str(n.id) || seen.has(str(n.id))) continue;
    seen.add(str(n.id));
    const kind = (NODE_KINDS as readonly string[]).includes(str(n.kind)) ? (n.kind as NodeKind) : "service";
    const node: DiagramNode = { id: str(n.id), label: str(n.label, str(n.id)), kind };
    if (str(n.detail)) node.detail = str(n.detail);
    if (str(n.group)) node.group = str(n.group);
    if (typeof n.x === "number" && Number.isFinite(n.x)) node.x = n.x;
    if (typeof n.y === "number" && Number.isFinite(n.y)) node.y = n.y;
    if (n.pinned === true) node.pinned = true;
    if (typeof n.w === "number" && Number.isFinite(n.w) && n.w >= 24) node.w = Math.min(4000, n.w);
    if (typeof n.h === "number" && Number.isFinite(n.h) && n.h >= 16) node.h = Math.min(4000, n.h);
    if (isObj(n.ref) && str(n.ref.document_id))
      node.ref = { document_id: str(n.ref.document_id), ...(str(n.ref.anchor) ? { anchor: str(n.ref.anchor) } : {}) };
    if ((BUILD_STATUSES as readonly string[]).includes(str(n.status))) node.status = n.status as BuildStatus;
    if ((ZONES as readonly string[]).includes(str(n.zone))) node.zone = n.zone as Zone;
    if (ID_RE.test(str(n.opens))) node.opens = str(n.opens);
    if (LIVE_RE.test(str(n.live))) node.live = str(n.live);
    nodes.push(node);
  }
  const groups: DiagramGroup[] = [];
  for (const g of Array.isArray(o.groups) ? o.groups : []) {
    if (!isObj(g) || !str(g.id) || seen.has(str(g.id))) continue;
    seen.add(str(g.id));
    const grp: DiagramGroup = {
      id: str(g.id),
      label: str(g.label, str(g.id)),
      ...((NODE_KINDS as readonly string[]).includes(str(g.kind)) ? { kind: g.kind as NodeKind } : {}),
    };
    if (g.collapsed === true) grp.collapsed = true;
    for (const k of ["x", "y", "w", "h"] as const)
      if (typeof g[k] === "number" && Number.isFinite(g[k])) grp[k] = g[k] as number;
    groups.push(grp);
  }
  const ids = new Set(nodes.map((n) => n.id));
  const gids = new Set(groups.map((g) => g.id));
  for (const n of nodes) if (n.group && !gids.has(n.group)) delete n.group;
  const edges: DiagramEdge[] = [];
  const eids = new Set<string>();
  for (const e of Array.isArray(o.edges) ? o.edges : []) {
    if (!isObj(e) || !ids.has(str(e.source)) || !ids.has(str(e.target))) continue;
    let id = str(e.id) || `${str(e.source)}--${str(e.target)}`;
    for (let k = 2; eids.has(id); k++) id = `${str(e.id) || `${str(e.source)}--${str(e.target)}`}-${k}`;
    eids.add(id);
    const edge: DiagramEdge = { id, source: str(e.source), target: str(e.target) };
    if (str(e.label)) edge.label = str(e.label);
    if ((EDGE_STYLES as readonly string[]).includes(str(e.style))) edge.style = e.style as EdgeStyle;
    const pts = points(e.points);
    if (pts) edge.points = pts;
    edges.push(edge);
  }
  const routes: Record<string, [number, number][]> = {};
  if (isObj(o.routes))
    for (const [k, v] of Object.entries(o.routes)) {
      const pts = points(v);
      if (pts && k.length <= 420) routes[k] = pts;
    }
  return {
    version: Math.max(1, Math.floor(num(o.version, 1))),
    direction: o.direction === "LR" ? "LR" : "TB",
    nodes,
    edges,
    ...(groups.length ? { groups } : {}),
    ...(o.layout === "pack" ? { layout: "pack" as const } : {}),
    ...(Object.keys(routes).length ? { routes } : {}),
  };
}

function item(v: unknown): Item | null {
  if (!isObj(v) || !ID_RE.test(str(v.item_id))) return null;
  const base = {
    item_id: str(v.item_id),
    title: str(v.title).slice(0, 300),
    ...box(v),
    ...(v.collapsed === true ? { collapsed: true } : {}),
    ...(ID_RE.test(str(v.frame_id)) ? { frame_id: str(v.frame_id) } : {}),
  };
  switch (v.type) {
    case "document": {
      const d = isObj(v.document) ? v.document : {};
      if (!str(d.document_id)) return null;
      const doc: DocumentItem = {
        ...base,
        type: "document",
        document: { space_id: str(d.space_id), document_id: str(d.document_id) },
      };
      if (str(d.kind)) doc.document.kind = str(d.kind);
      if (str(d.display_number)) doc.document.display_number = str(d.display_number);
      if (Array.isArray(v.folded)) doc.folded = v.folded.filter((a): a is string => typeof a === "string");
      if (Array.isArray(v.prs))
        doc.prs = v.prs.filter((a): a is string => typeof a === "string" && /^https:\/\//.test(a));
      return doc;
    }
    case "diagram": {
      const d: DiagramItem = { ...base, type: "diagram", model: parseModel(v.model) };
      if (isObj(v.diagram) && str(v.diagram.diagram_id))
        d.diagram = {
          space_id: str(v.diagram.space_id),
          diagram_id: str(v.diagram.diagram_id),
          version: Math.max(0, Math.floor(num(v.diagram.version))),
        };
      if (isObj(v.placeholder))
        d.placeholder = {
          owner: str(v.placeholder.owner),
          source: str(v.placeholder.source),
          note: str(v.placeholder.note),
        };
      return d;
    }
    case "note":
      return {
        ...base,
        type: "note",
        text: str(v.text).slice(0, 20_000),
        ...(tone(v.tone) ? { tone: tone(v.tone) } : {}),
      };
    case "frame":
      return {
        ...base,
        type: "frame",
        ...(str(v.text) ? { text: str(v.text).slice(0, 2000) } : {}),
        ...(tone(v.tone) ? { tone: tone(v.tone) } : {}),
      };
    case "shape": {
      if (!(SHAPE_KINDS as readonly string[]).includes(str(v.shape))) return null;
      const sh: ShapeItem = { ...base, type: "shape", shape: v.shape as ShapeKind };
      if (str(v.text)) sh.text = str(v.text).slice(0, 20_000);
      if ((NODE_KINDS as readonly string[]).includes(str(v.kind))) sh.kind = v.kind as NodeKind;
      if (str(v.detail)) sh.detail = str(v.detail).slice(0, 2000);
      if (typeof v.angle === "number" && Number.isFinite(v.angle) && v.angle !== 0) sh.angle = v.angle;
      const pts = points(v.points);
      if (pts) sh.points = pts;
      const st = target(v.start);
      const en = target(v.end);
      if (st) sh.start = st;
      if (en) sh.end = en;
      if (isObj(v.arrowheads)) {
        const a: { start?: string; end?: string } = {};
        if (str(v.arrowheads.start)) a.start = str(v.arrowheads.start).slice(0, 20);
        if (str(v.arrowheads.end)) a.end = str(v.arrowheads.end).slice(0, 20);
        sh.arrowheads = a;
      }
      if (v.elbow === true) sh.elbow = true;
      const style = shapeStyle(v.style);
      if (style) sh.style = style;
      if (Array.isArray(v.groups)) {
        const g = v.groups.filter((x): x is string => typeof x === "string" && x.length <= 80).slice(0, 20);
        if (g.length) sh.groups = g;
      }
      return sh;
    }
    case "image": {
      const src = str(v.src);
      if (src.length > MAX_IMAGE_SRC || !IMAGE_SRC_RE.test(src)) return null;
      const img: ImageItem = { ...base, type: "image", src };
      if (str(v.alt)) img.alt = str(v.alt).slice(0, 500);
      if (typeof v.natural_w === "number" && v.natural_w >= 1 && v.natural_w <= 20_000)
        img.natural_w = Math.round(v.natural_w);
      if (typeof v.natural_h === "number" && v.natural_h >= 1 && v.natural_h <= 20_000)
        img.natural_h = Math.round(v.natural_h);
      return img;
    }
    case "ask":
      if (!str(v.ask_id)) return null;
      return {
        ...base,
        type: "ask",
        ask_id: str(v.ask_id),
        text: str(v.text).slice(0, 20_000),
        ...(str(v.asked_at) ? { asked_at: str(v.asked_at) } : {}),
        ...(str(v.by) ? { by: str(v.by) } : {}),
      };
    case "option":
      if (!str(v.ask_id) || (v.option !== "approve" && v.option !== "deny")) return null;
      return { ...base, type: "option", ask_id: str(v.ask_id), option: v.option, means: str(v.means).slice(0, 2000) };
    case "pr":
      if (!str(v.repo) || typeof v.number !== "number" || !Number.isInteger(v.number) || v.number < 1) return null;
      return { ...base, type: "pr", repo: str(v.repo), number: v.number };
    case "decision":
      if (!["prd", "rfc", "adr", "study"].includes(str(v.kind)) || !/^\d{1,5}(\.\d{1,2})?$/.test(str(v.number)))
        return null;
      return { ...base, type: "decision", kind: v.kind as DecisionItem["kind"], number: str(v.number) };
    default:
      return null;
  }
}

/** Version 0 (the first drafts): `id` instead of `item_id`, `kind` instead of `type`, no threads. */
function migrate0(o: Obj): Obj {
  const items = (Array.isArray(o.items) ? o.items : []).map((i) =>
    isObj(i) ? { ...i, item_id: i.item_id ?? i.id, type: i.type ?? i.kind } : i,
  );
  return {
    ...o,
    schema: SCHEMA,
    version: 1,
    canvas_id: o.canvas_id ?? o.id,
    items,
    links: o.links ?? [],
    threads: o.threads ?? [],
  };
}

/**
 * Version 1 to 2: items that sit wholly inside a section frame are put in it (`frame_id`), so
 * moving the frame moves them. Nothing else changes; every field of version 1 means the same.
 */
export function migrate1(items: Item[]): Item[] {
  const frames = items.filter((i): i is FrameItem => i.type === "frame");
  const inside = (a: Box, f: Box) => a.x >= f.x && a.y >= f.y && a.x + a.w <= f.x + f.w && a.y + a.h <= f.y + f.h;
  return items.map((i) => {
    if (i.type === "frame" || i.type === "diagram" || i.frame_id) return i;
    // The smallest frame that holds it.
    const f = frames.filter((f) => inside(i, f)).sort((a, b) => a.w * a.h - b.w * b.h)[0];
    return f ? { ...i, frame_id: f.item_id } : i;
  });
}

function follows(v: unknown): Pick<Canvas, "follows"> {
  if (!isObj(v) || !str(v.plan_id) || !isObj(v.seen)) return {};
  const seen: Record<string, string> = {};
  for (const [k, s] of Object.entries(v.seen).slice(0, 10_000)) if (typeof s === "string") seen[k] = s.slice(0, 40);
  return { follows: { plan_id: str(v.plan_id).slice(0, 120), seen } };
}

const STORY_ID_RE = /^[a-z0-9][a-z0-9_-]{0,63}$/;
const ids = (v: unknown, max: number): string[] =>
  (Array.isArray(v) ? v : [])
    .filter((x): x is string => typeof x === "string" && x.length > 0 && x.length <= 200)
    .slice(0, max);

/** A file's walk-throughs, bounded; a story without a usable id, title or step is dropped and named. */
export function parseStories(v: unknown, dropped: string[]): CanvasStory[] {
  const out: CanvasStory[] = [];
  for (const s of (Array.isArray(v) ? v : []).slice(0, 32)) {
    const id = isObj(s) ? str(s.id) : "";
    const steps: StoryStep[] = (isObj(s) && Array.isArray(s.steps) ? s.steps : [])
      .filter(isObj)
      .slice(0, 64)
      .map((st) => ({
        title: str(st.title).slice(0, 120),
        caption: str(st.caption).slice(0, 2_000),
        frame: str(st.frame),
        nodes: ids(st.nodes, 64),
        edges: ids(st.edges, 64),
        ...(Array.isArray(st.items) ? { items: ids(st.items, 64) } : {}),
      }))
      .filter((st) => st.title && st.caption && st.frame);
    if (!isObj(s) || !STORY_ID_RE.test(id) || !str(s.title) || !steps.length || out.some((x) => x.id === id)) {
      dropped.push(`story ${id || "without an id"}`);
      continue;
    }
    out.push({ id, title: str(s.title).slice(0, 200), steps, ...(s.close === true ? { close: true } : {}) });
  }
  return out;
}

/** Reads a canvas file. Never throws: a file it can't read says why; unknown items are dropped and named. */
export function parseCanvas(text: string): Parsed {
  let raw: unknown;
  try {
    raw = JSON.parse(text);
  } catch (e) {
    return { ok: false, error: `It isn't JSON: ${(e as Error).message}` };
  }
  if (!isObj(raw)) return { ok: false, error: "It isn't a canvas: the file holds no object." };
  let o = raw;
  let migrated = false;
  const version = num(o.version, 0);
  if (o.schema !== undefined && o.schema !== SCHEMA)
    return { ok: false, error: `It isn't a canvas: its schema is ${str(o.schema) || "unknown"}.` };
  if (version > VERSION)
    return {
      ok: false,
      error: `It was written by a newer InOrbit (canvas version ${version}); update the app to open it.`,
    };
  if (version < 1) {
    o = migrate0(o);
    migrated = true;
  }
  const id = str(o.canvas_id);
  if (!ID_RE.test(id)) return { ok: false, error: "It has no usable canvas_id." };
  const dropped: string[] = [];
  const items: Item[] = [];
  const ids = new Set<string>();
  for (const v of Array.isArray(o.items) ? o.items : []) {
    const it = item(v);
    if (!it || ids.has(it.item_id)) {
      dropped.push(isObj(v) ? str(v.item_id, "an item") : "an item");
      continue;
    }
    ids.add(it.item_id);
    items.push(it);
  }
  if (version < 2) {
    items.splice(0, items.length, ...migrate1(items));
    migrated = true;
  }
  for (const it of items)
    if (it.frame_id && !items.some((f) => f.item_id === it.frame_id && (f.type === "frame" || f.type === "diagram")))
      delete it.frame_id;
  const links: Link[] = [];
  for (const v of Array.isArray(o.links) ? o.links : []) {
    if (!isObj(v)) continue;
    const from = target(v.from);
    const to = target(v.to);
    if (!from || !to || !ids.has(from.item_id) || !ids.has(to.item_id)) continue;
    links.push({
      link_id: str(v.link_id) || newId("lnk"),
      from,
      to,
      kind: str(v.kind, "context").slice(0, 40),
      ...(str(v.label) ? { label: str(v.label).slice(0, 120) } : {}),
    });
  }
  const threads: Thread[] = [];
  for (const v of Array.isArray(o.threads) ? o.threads : []) {
    if (!isObj(v)) continue;
    const t = target(v.target);
    if (!t) continue;
    const comments: Comment[] = (Array.isArray(v.comments) ? v.comments : []).filter(isObj).map((c) => ({
      comment_id: str(c.comment_id) || newId("cmt"),
      ...(str(c.parent_id) ? { parent_id: str(c.parent_id) } : {}),
      author: str(c.author, "someone"),
      body: str(c.body).slice(0, 10_240),
      created_at: str(c.created_at),
      ...(c.resolved === true ? { resolved: true, resolved_at: str(c.resolved_at) } : {}),
    }));
    const ai =
      isObj(v.ai) && ["asking", "waiting", "done", "failed"].includes(str(v.ai.status))
        ? {
            status: str(v.ai.status) as ThreadAi["status"],
            ...(str(v.ai.asked_at) ? { asked_at: str(v.ai.asked_at) } : {}),
            ...(str(v.ai.note) ? { note: str(v.ai.note).slice(0, 1000) } : {}),
          }
        : undefined;
    if (comments.length)
      threads.push({ thread_id: str(v.thread_id) || newId("thr"), target: t, comments, ...(ai ? { ai } : {}) });
  }
  const canvas: Canvas = {
    schema: SCHEMA,
    version: VERSION,
    canvas_id: id,
    title: str(o.title, "Untitled canvas").slice(0, 200),
    ...(str(o.description) ? { description: str(o.description) } : {}),
    ...(str(o.group) ? { group: str(o.group).slice(0, 200) } : {}),
    ...(str(o.account_id) ? { account_id: str(o.account_id) } : {}),
    ...(str(o.space_id) ? { space_id: str(o.space_id) } : {}),
    created_at: str(o.created_at),
    updated_at: str(o.updated_at),
    revision: Math.max(0, Math.floor(num(o.revision))),
    ...(o.style === "sketch" ? { style: "sketch" as const } : {}),
    ...follows(o.follows),
    items,
    links,
    threads,
  };
  const stories = parseStories(o.stories, dropped);
  if (stories.length) canvas.stories = stories;
  // Version 2 to 3 adds fields (zones, levels, two edge styles) and changes none: nothing to move.
  if (version < 3) migrated = true;
  return { ok: true, canvas, migrated, dropped, from: version };
}

/** The file's text: stable key order, two-space indent, so diffs of a canvas read well. */
export const serialize = (c: Canvas): string => JSON.stringify(c, null, 2) + "\n";

export function emptyCanvas(title: string, now = new Date().toISOString()): Canvas {
  return {
    schema: SCHEMA,
    version: VERSION,
    canvas_id: newId("cnv"),
    title,
    created_at: now,
    updated_at: now,
    revision: 0,
    items: [],
    links: [],
    threads: [],
  };
}

// ---------------------------------------------------------------- small edits, all pure

export const itemOf = (c: Canvas, id: string): Item | undefined => c.items.find((i) => i.item_id === id);

export function updateItem(c: Canvas, id: string, patch: Partial<Item>): Canvas {
  return { ...c, items: c.items.map((i) => (i.item_id === id ? ({ ...i, ...patch } as Item) : i)) };
}

/** Removes items and every link and canvas thread that pointed at them. */
export function removeItems(c: Canvas, ids: string[]): Canvas {
  const gone = new Set(ids);
  return {
    ...c,
    items: c.items.filter((i) => !gone.has(i.item_id)),
    links: c.links.filter((l) => !gone.has(l.from.item_id) && !gone.has(l.to.item_id)),
    threads: c.threads.filter((t) => !gone.has(t.target.item_id)),
  };
}

/** The box around some items (or all), or null when there are none. */
export function bounds(items: Box[]): Box | null {
  if (!items.length) return null;
  let x0 = Infinity,
    y0 = Infinity,
    x1 = -Infinity,
    y1 = -Infinity;
  for (const b of items) {
    x0 = Math.min(x0, b.x);
    y0 = Math.min(y0, b.y);
    x1 = Math.max(x1, b.x + b.w);
    y1 = Math.max(y1, b.y + b.h);
  }
  return { x: x0, y: y0, w: x1 - x0, h: y1 - y0 };
}

/** Frames first (they sit behind), then everything else in file order. */
export const paintOrder = (items: Item[]): Item[] => [
  ...items.filter((i) => i.type === "frame"),
  ...items.filter((i) => i.type !== "frame"),
];
