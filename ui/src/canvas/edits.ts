// Edits the coordinator proposes (PRD 0065). When it answers a question asked from the canvas
// it may add a fenced block tagged `iohr-canvas-edit` holding JSON; the panel lists each
// proposal with what it would change, and nothing is applied until a person clicks Apply.
// Parsing and applying are pure, so they are tested on their own (tests/canvas-edits.test.ts).

import {
  type Canvas,
  type DiagramEdge,
  type DiagramItem,
  type DiagramNode,
  elementId,
  type Item,
  NODE_KINDS,
  type NodeKind,
  newId,
} from "./schema.ts";

export const FENCE = "iohr-canvas-edit";

export type Edit =
  | { op: "add_note"; text: string; near?: string; title?: string }
  | { op: "update_note"; item_id: string; text: string }
  /** The text of a note, a frame or a shape. */
  | { op: "update_text"; item_id: string; text: string }
  | { op: "rename"; item_id: string; title: string }
  | {
      op: "add_node";
      item_id: string;
      node: { id?: string; label: string; kind?: string; detail?: string; group?: string };
      edges?: { source: string; target: string; label?: string }[];
    }
  | { op: "update_node"; item_id: string; node_id: string; label?: string; detail?: string; kind?: string }
  | { op: "remove_node"; item_id: string; node_id: string }
  | { op: "add_edge"; item_id: string; source: string; target: string; label?: string }
  | { op: "remove_edge"; item_id: string; edge_id: string }
  | { op: "add_link"; from: string; to: string; kind?: string; label?: string }
  /** A new text for one section of a live document: saved to Decisions as a new version, on click. */
  | { op: "replace_section"; item_id: string; anchor: string; text: string };

/** `thread` and `reply` are set when the block answers a comment thread kept with the canvas. */
export type Proposal = { key: string; summary: string; edits: Edit[]; thread?: string; reply?: string };

type Obj = Record<string, unknown>;
const isObj = (v: unknown): v is Obj => !!v && typeof v === "object" && !Array.isArray(v);
const s = (v: unknown) => (typeof v === "string" ? v : "");

function edit(v: unknown): Edit | null {
  if (!isObj(v)) return null;
  switch (v.op) {
    case "add_note":
      return s(v.text)
        ? {
            op: "add_note",
            text: s(v.text),
            ...(s(v.near) ? { near: s(v.near) } : {}),
            ...(s(v.title) ? { title: s(v.title) } : {}),
          }
        : null;
    case "update_note":
      return s(v.item_id) && s(v.text) ? { op: "update_note", item_id: s(v.item_id), text: s(v.text) } : null;
    case "update_text":
      return s(v.item_id) && s(v.text) ? { op: "update_text", item_id: s(v.item_id), text: s(v.text) } : null;
    case "rename":
      return s(v.item_id) && s(v.title) ? { op: "rename", item_id: s(v.item_id), title: s(v.title) } : null;
    case "add_node": {
      const n = isObj(v.node) ? v.node : null;
      if (!s(v.item_id) || !n || !s(n.label)) return null;
      const edges = Array.isArray(v.edges)
        ? v.edges
            .filter(isObj)
            .map((e) => ({ source: s(e.source), target: s(e.target), ...(s(e.label) ? { label: s(e.label) } : {}) }))
            .filter((e) => e.source && e.target)
        : undefined;
      return {
        op: "add_node",
        item_id: s(v.item_id),
        node: {
          label: s(n.label),
          ...(s(n.id) ? { id: s(n.id) } : {}),
          ...(s(n.kind) ? { kind: s(n.kind) } : {}),
          ...(s(n.detail) ? { detail: s(n.detail) } : {}),
          ...(s(n.group) ? { group: s(n.group) } : {}),
        },
        ...(edges?.length ? { edges } : {}),
      };
    }
    case "update_node":
      return s(v.item_id) && s(v.node_id)
        ? {
            op: "update_node",
            item_id: s(v.item_id),
            node_id: s(v.node_id),
            ...(s(v.label) ? { label: s(v.label) } : {}),
            ...(typeof v.detail === "string" ? { detail: v.detail } : {}),
            ...(s(v.kind) ? { kind: s(v.kind) } : {}),
          }
        : null;
    case "remove_node":
      return s(v.item_id) && s(v.node_id) ? { op: "remove_node", item_id: s(v.item_id), node_id: s(v.node_id) } : null;
    case "add_edge":
      return s(v.item_id) && s(v.source) && s(v.target)
        ? {
            op: "add_edge",
            item_id: s(v.item_id),
            source: s(v.source),
            target: s(v.target),
            ...(s(v.label) ? { label: s(v.label) } : {}),
          }
        : null;
    case "remove_edge":
      return s(v.item_id) && s(v.edge_id) ? { op: "remove_edge", item_id: s(v.item_id), edge_id: s(v.edge_id) } : null;
    case "add_link":
      return s(v.from) && s(v.to)
        ? {
            op: "add_link",
            from: s(v.from),
            to: s(v.to),
            ...(s(v.kind) ? { kind: s(v.kind) } : {}),
            ...(s(v.label) ? { label: s(v.label) } : {}),
          }
        : null;
    case "replace_section":
      return s(v.item_id) && s(v.anchor) && typeof v.text === "string"
        ? { op: "replace_section", item_id: s(v.item_id), anchor: s(v.anchor), text: v.text }
        : null;
    default:
      return null;
  }
}

/**
 * Every `iohr-canvas-edit` block in a reply. A block holds `{"summary": "...", "edits": [...]}`
 * or a bare list of edits. Blocks that don't parse are skipped; an edit it doesn't know is dropped.
 */
export function proposalsIn(text: string, keyPrefix = ""): Proposal[] {
  const out: Proposal[] = [];
  const re = new RegExp("```\\s*" + FENCE + "[^\\n]*\\n([\\s\\S]*?)```", "g");
  let n = 0;
  for (const m of text.matchAll(re)) {
    let v: unknown;
    try {
      v = JSON.parse(m[1]);
    } catch {
      continue;
    }
    const list = Array.isArray(v) ? v : isObj(v) && Array.isArray(v.edits) ? v.edits : isObj(v) && v.op ? [v] : [];
    const edits = list.map(edit).filter((e): e is Edit => !!e);
    const thread = isObj(v) ? s(v.thread) : "";
    const reply = isObj(v) ? s(v.reply) : "";
    // An answer to a thread may carry only the reply.
    if (!edits.length && !(thread && reply)) continue;
    const summary = isObj(v) && s(v.summary) ? s(v.summary) : edits.length ? edits.map(describe).join("; ") : "A reply";
    out.push({
      key: `${keyPrefix}${n++}:${hash(m[1])}`,
      summary,
      edits,
      ...(thread ? { thread } : {}),
      ...(reply ? { reply } : {}),
    });
  }
  return out;
}

function hash(t: string): string {
  let h = 2166136261;
  for (let i = 0; i < t.length; i++) h = Math.imul(h ^ t.charCodeAt(i), 16777619);
  return (h >>> 0).toString(36);
}

/** One line a person can read before clicking Apply. */
export function describe(e: Edit): string {
  switch (e.op) {
    case "add_note":
      return `Add a note: “${e.text.slice(0, 60)}${e.text.length > 60 ? "…" : ""}”`;
    case "update_note":
      return `Change note ${e.item_id}`;
    case "update_text":
      return `Rewrite the text of ${e.item_id}`;
    case "rename":
      return `Rename ${e.item_id} to “${e.title}”`;
    case "add_node":
      return `Add “${e.node.label}” to diagram ${e.item_id}${e.edges?.length ? ` with ${e.edges.length} connection${e.edges.length === 1 ? "" : "s"}` : ""}`;
    case "update_node":
      return `Change node ${e.node_id}${e.label ? ` to “${e.label}”` : ""}`;
    case "remove_node":
      return `Remove node ${e.node_id}`;
    case "add_edge":
      return `Connect ${e.source} to ${e.target}${e.label ? ` (“${e.label}”)` : ""}`;
    case "remove_edge":
      return `Remove connection ${e.edge_id}`;
    case "add_link":
      return `Link ${e.from} to ${e.to}`;
    case "replace_section":
      return `Rewrite section #${e.anchor} of the document (saved to Decisions as a new version)`;
  }
}

export type Applied = {
  canvas: Canvas;
  done: string[];
  skipped: string[] /** Section rewrites go to Decisions; the caller saves them. */;
  sections: Extract<Edit, { op: "replace_section" }>[];
};

const kindOf = (k?: string): NodeKind =>
  (NODE_KINDS as readonly string[]).includes(k ?? "") ? (k as NodeKind) : "service";

function onDiagram(c: Canvas, id: string, f: (d: DiagramItem) => DiagramItem | string): Canvas | string {
  const it = c.items.find((i) => i.item_id === id);
  if (!it || it.type !== "diagram") return `no diagram ${id} on this canvas`;
  const next = f(it);
  if (typeof next === "string") return next;
  return { ...c, items: c.items.map((i) => (i.item_id === id ? next : i)) };
}

/** Applies the canvas edits in order; each that can't apply is skipped with the reason. */
export function applyEdits(c: Canvas, edits: Edit[], now = new Date().toISOString()): Applied {
  void now;
  let cur = c;
  const done: string[] = [];
  const skipped: string[] = [];
  const sections: Applied["sections"] = [];
  for (const e of edits) {
    let next: Canvas | string;
    switch (e.op) {
      case "add_note": {
        const near = e.near ? cur.items.find((i) => i.item_id === e.near) : undefined;
        const x = near ? near.x + near.w + 40 : Math.max(0, ...cur.items.map((i) => i.x + i.w)) + 60;
        const y = near ? near.y : 0;
        const note: Item = {
          item_id: newId("itm"),
          type: "note",
          title: e.title ?? "Note from the coordinator",
          text: e.text,
          tone: "amber",
          x,
          y,
          w: 300,
          h: 180,
        };
        next = { ...cur, items: [...cur.items, note] };
        break;
      }
      case "update_note": {
        const it = cur.items.find((i) => i.item_id === e.item_id);
        next =
          it?.type === "note"
            ? { ...cur, items: cur.items.map((i) => (i.item_id === e.item_id ? { ...it, text: e.text } : i)) }
            : `no note ${e.item_id}`;
        break;
      }
      case "update_text": {
        const it = cur.items.find((i) => i.item_id === e.item_id);
        next =
          it?.type === "note" || it?.type === "frame" || it?.type === "shape"
            ? { ...cur, items: cur.items.map((i) => (i.item_id === e.item_id ? { ...it, text: e.text } : i)) }
            : `no note, frame or shape ${e.item_id}`;
        break;
      }
      case "rename": {
        const has = cur.items.some((i) => i.item_id === e.item_id);
        next = has
          ? { ...cur, items: cur.items.map((i) => (i.item_id === e.item_id ? { ...i, title: e.title } : i)) }
          : `no item ${e.item_id}`;
        break;
      }
      case "add_node":
        next = onDiagram(cur, e.item_id, (d) => {
          const taken = new Set(d.model.nodes.map((n) => n.id));
          const id =
            e.node.id && !taken.has(e.node.id) && /^[a-z0-9][a-z0-9-]*$/.test(e.node.id)
              ? e.node.id
              : elementId(e.node.label, taken);
          const node: DiagramNode = {
            id,
            label: e.node.label,
            kind: kindOf(e.node.kind),
            ...(e.node.detail ? { detail: e.node.detail } : {}),
            ...(e.node.group && d.model.groups?.some((g) => g.id === e.node.group) ? { group: e.node.group } : {}),
          };
          const ids = new Set([...taken, id]);
          const edges: DiagramEdge[] = (e.edges ?? [])
            .filter((x) => ids.has(x.source) && ids.has(x.target))
            .map((x) => ({
              id: `${x.source}--${x.target}`,
              source: x.source,
              target: x.target,
              ...(x.label ? { label: x.label } : {}),
            }));
          // Positions are cleared for the new node only; the frame lays it out next to the others.
          return {
            ...d,
            model: { ...d.model, nodes: [...d.model.nodes, node], edges: dedupeEdges([...d.model.edges, ...edges]) },
          };
        });
        break;
      case "update_node":
        next = onDiagram(cur, e.item_id, (d) =>
          d.model.nodes.some((n) => n.id === e.node_id)
            ? {
                ...d,
                model: {
                  ...d.model,
                  nodes: d.model.nodes.map((n) =>
                    n.id === e.node_id
                      ? {
                          ...n,
                          ...(e.label ? { label: e.label } : {}),
                          ...(e.detail !== undefined ? { detail: e.detail } : {}),
                          ...(e.kind ? { kind: kindOf(e.kind) } : {}),
                        }
                      : n,
                  ),
                },
              }
            : `no node ${e.node_id}`,
        );
        break;
      case "remove_node":
        next = onDiagram(cur, e.item_id, (d) =>
          d.model.nodes.some((n) => n.id === e.node_id)
            ? {
                ...d,
                model: {
                  ...d.model,
                  nodes: d.model.nodes.filter((n) => n.id !== e.node_id),
                  edges: d.model.edges.filter((x) => x.source !== e.node_id && x.target !== e.node_id),
                },
              }
            : `no node ${e.node_id}`,
        );
        break;
      case "add_edge":
        next = onDiagram(cur, e.item_id, (d) => {
          const ids = new Set(d.model.nodes.map((n) => n.id));
          if (!ids.has(e.source) || !ids.has(e.target)) return `no node ${ids.has(e.source) ? e.target : e.source}`;
          return {
            ...d,
            model: {
              ...d.model,
              edges: dedupeEdges([
                ...d.model.edges,
                {
                  id: `${e.source}--${e.target}`,
                  source: e.source,
                  target: e.target,
                  ...(e.label ? { label: e.label } : {}),
                },
              ]),
            },
          };
        });
        break;
      case "remove_edge":
        next = onDiagram(cur, e.item_id, (d) =>
          d.model.edges.some((x) => x.id === e.edge_id)
            ? { ...d, model: { ...d.model, edges: d.model.edges.filter((x) => x.id !== e.edge_id) } }
            : `no connection ${e.edge_id}`,
        );
        break;
      case "add_link": {
        const ok = cur.items.some((i) => i.item_id === e.from) && cur.items.some((i) => i.item_id === e.to);
        next = ok
          ? {
              ...cur,
              links: [
                ...cur.links,
                {
                  link_id: newId("lnk"),
                  from: { item_id: e.from },
                  to: { item_id: e.to },
                  kind: e.kind ?? "context",
                  ...(e.label ? { label: e.label } : {}),
                },
              ],
            }
          : `no item ${e.from} or ${e.to}`;
        break;
      }
      case "replace_section": {
        const it = cur.items.find((i) => i.item_id === e.item_id);
        if (it?.type === "document") {
          sections.push(e);
          done.push(describe(e));
        } else skipped.push(`${describe(e)}: no document ${e.item_id}`);
        continue;
      }
    }
    if (typeof next === "string") skipped.push(`${describe(e)}: ${next}`);
    else {
      cur = next;
      done.push(describe(e));
    }
  }
  return { canvas: cur, done, skipped, sections };
}

function dedupeEdges(edges: DiagramEdge[]): DiagramEdge[] {
  const seen = new Set<string>();
  const out: DiagramEdge[] = [];
  for (const e of edges) {
    let id = e.id;
    for (let k = 2; seen.has(id); k++) id = `${e.id}-${k}`;
    seen.add(id);
    out.push(id === e.id ? e : { ...e, id });
  }
  return out;
}

/** What "Ask about this" sends: who is asking about what, the part's text, and how to propose edits. */
export function askPrompt(a: {
  canvas: string;
  canvasId: string;
  what: string;
  itemId: string;
  element?: string;
  documentId?: string;
  title: string;
  text: string;
  question: string;
}): string {
  const where = [
    `item ${a.itemId}`,
    a.element ? `element ${a.element}` : "",
    a.documentId ? `document ${a.documentId}` : "",
  ]
    .filter(Boolean)
    .join(", ");
  const body = a.text.length > 2400 ? `${a.text.slice(0, 2400)}\n[cut at 2,400 characters]` : a.text;
  return [
    `From the canvas “${a.canvas}” (${a.canvasId}), about the ${a.what} “${a.title}” (${where}):`,
    "",
    body.trim() ? body.trim() : "(no text)",
    "",
    `Question: ${a.question.trim() || "What do you think of this part, and what would you change?"}`,
    "",
    `If you propose a change to the canvas, add a fenced block tagged ${FENCE} with JSON {"summary": "...", "edits": [...]}. Ops: add_note {text, near}, update_note {item_id, text}, update_text {item_id, text} (a note, frame or shape), rename {item_id, title}, add_node {item_id, node: {label, kind, detail, group}, edges: [{source, target, label}]}, update_node {item_id, node_id, label, detail, kind}, remove_node, add_edge {item_id, source, target, label}, remove_edge {item_id, edge_id}, add_link {from, to, label}, replace_section {item_id, anchor, text}. I apply it only when I click Apply.`,
  ].join("\n");
}

/** The comment that carries a suggested change until suggested versions ship (RFC 0124). */
export function suggestionBody(anchor: string, base: number, text: string): string {
  return `**Suggested change to #${anchor}** (on version ${base}, from the canvas; waiting for the system and then a person to approve)\n\n${text.trim()}`;
}

/** What a person's comment on a canvas part sends, when the coordinator answers comments: the part, the thread, and how to answer. */
export function commentPrompt(a: {
  canvas: string;
  canvasId: string;
  threadId: string;
  what: string;
  itemId: string;
  element?: string;
  title: string;
  text: string;
  quote?: string;
  conversation: { author: string; body: string }[];
}): string {
  const where = [`item ${a.itemId}`, a.element ? `element ${a.element}` : ""].filter(Boolean).join(", ");
  const body = a.text.length > 4000 ? `${a.text.slice(0, 4000)}\n[cut at 4,000 characters]` : a.text;
  return [
    `A comment on the canvas “${a.canvas}” (${a.canvasId}), thread ${a.threadId}, on the ${a.what} “${a.title}” (${where}).`,
    "",
    "The part as it is now:",
    body.trim() ? body.trim() : "(no text)",
    ...(a.quote ? ["", `The passage they picked: “${a.quote}”`] : []),
    "",
    "The thread:",
    ...a.conversation.map((c) => `- ${c.author}: ${c.body}`),
    "",
    `Answer in that thread and redo the part as the comment asks. Reply with one fenced block tagged ${FENCE} holding JSON {"thread": "${a.threadId}", "reply": "what you changed and why, or your answer, in a sentence or two", "summary": "...", "edits": [...]}. Change only the part commented on unless the comment asks for more. Ops: update_text {item_id, text} for a note, frame or shape (the whole new text), rename {item_id, title}, update_node {item_id, node_id, label, detail, kind}, add_node, remove_node, add_edge, remove_edge, add_note {text, near}, add_link {from, to, label}. Leave edits empty when the comment is a question. The app applies the edits as soon as your answer arrives and posts your reply in the thread as the coordinator (AI).`,
  ].join("\n");
}
