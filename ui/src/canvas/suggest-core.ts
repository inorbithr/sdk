// Quick suggestions and "start work" on the canvas (PRD 0065), ported from the Mac app's
// canvas-suggest.ts and canvas-work.ts. Pure. The web has no asks.jsonl: a suggestion is a queued
// ask (POST /v1/coordinator/asks) and the work's state is read from the live picture.

import { type Rec, str } from "../lib/picture.ts";
import { type Canvas, itemOf } from "./schema.ts";

export type RefEl = { item_id: string; kind: string; label: string };
export type SelRef = { canvas_id: string; canvas_title: string; elements: RefEl[] };

/** What is selected, as a reference an ask can carry. */
export function buildSelRef(canvas: Canvas, ids: string[]): SelRef | null {
  const elements: RefEl[] = [];
  for (const id of new Set(ids)) {
    const it = itemOf(canvas, id);
    if (!it) continue;
    const label =
      ("title" in it && it.title) ||
      ("text" in it && typeof it.text === "string" && it.text.split("\n")[0].slice(0, 40)) ||
      "this part of the board";
    elements.push({ item_id: id, kind: it.type, label });
  }
  return elements.length ? { canvas_id: canvas.canvas_id, canvas_title: canvas.title, elements } : null;
}

/** "A", "A and B", "A, B and 2 more". */
export function labelsOf(ref: SelRef, max = 3): string {
  const l = ref.elements.map((e) => `“${e.label}”`);
  if (l.length <= 1) return l[0] ?? "";
  if (l.length <= max) return `${l.slice(0, -1).join(", ")} and ${l[l.length - 1]}`;
  return `${l.slice(0, max - 1).join(", ")} and ${l.length - (max - 1)} more`;
}

export type ChipId = "wrong" | "detail" | "split" | "rename" | "connect";
export const CHIPS: { id: ChipId; label: string; prefill?: string }[] = [
  { id: "wrong", label: "This is wrong" },
  { id: "detail", label: "Add detail" },
  { id: "split", label: "Split this" },
  { id: "rename", label: "Rename", prefill: "Rename to " },
  { id: "connect", label: "Connect to…", prefill: "Connect to " },
];
const SENTENCE: Record<ChipId, string> = {
  wrong: "This is wrong.",
  detail: "This needs more detail.",
  split: "This should be split into smaller parts.",
  rename: "This needs a better name.",
  connect: "This should be connected to something.",
};

/** The words typed win over the chip's sentence. */
export function suggestionText(chip: ChipId | null, typed: string): string {
  const t = typed.trim().replace(/\s+/g, " ");
  return t || (chip ? SENTENCE[chip] : "");
}

/** The ask body for the coordinator, with the canvas page marker the board reads back. */
export function suggestionAsk(
  ref: SelRef,
  text: string,
): { title: string; body: string; source: string; external_id: string } | null {
  if (!text.trim()) return null;
  const body = `${text} (on the canvas “${ref.canvas_title}”, about ${labelsOf(ref, 6)})`.slice(0, 1900);
  return { title: body.slice(0, 120), body, source: `canvas:${ref.canvas_id}`, external_id: "" };
}

/** The same suggestion on the same selection within `windowMs` is one: a double press files once. */
export function isRepeat(
  recent: Map<string, number>,
  ref: SelRef,
  text: string,
  now: number,
  windowMs = 4000,
): boolean {
  for (const [k, at] of recent) if (now - at > windowMs) recent.delete(k);
  const key = `${ref.canvas_id}|${ref.elements
    .map((e) => e.item_id)
    .sort()
    .join(",")}|${text.trim().toLowerCase()}`;
  if (recent.has(key)) return true;
  recent.set(key, now);
  return false;
}

export const WORK_KINDS = [
  { id: "design", label: "Design" },
  { id: "docs", label: "Docs" },
  { id: "ui", label: "UI" },
  { id: "api", label: "API" },
] as const;
export type WorkKind = (typeof WORK_KINDS)[number]["id"];

/** The kind of work from what is selected; the person can change it. */
export function suggestKind(ref: SelRef | null): WorkKind {
  const words = (ref?.elements ?? [])
    .map((e) => e.label)
    .join(" ")
    .toLowerCase();
  if (/\b(api|rpc|grpc|rest|endpoint|gateway|envoy|crate|service|broker|scheduler|runner|store|function)\b/.test(words))
    return "api";
  if (/\b(ui|page|screen|app|dashboard|desktop|iphone|browser|console|card|panel)\b/.test(words)) return "ui";
  if (ref?.elements.length && ref.elements.every((e) => e.kind === "document" || e.kind === "note")) return "docs";
  return "design";
}

export const workMarker = (now = Date.now()) => `cwk-${now.toString(36)}`;

/** The brief the work starts from. The work ends in a pull request for review; it never merges or rolls. */
export function buildBrief(a: {
  canvas: Canvas;
  ref: SelRef | null;
  marker: string;
  kind: WorkKind;
  note?: string;
  max?: number;
}): string {
  const out = [
    `Work ${a.marker}, approved on the canvas “${a.canvas.title}” (${a.canvas.canvas_id}). Kind: ${a.kind}.`,
  ];
  if (a.note?.trim()) out.push(`Owner's note: ${a.note.trim()}`);
  out.push("Approved:");
  if (a.ref) for (const e of a.ref.elements) out.push(`- ${e.kind}: ${e.label}`);
  else for (const it of a.canvas.items) out.push(`- ${it.type}: ${("title" in it && it.title) || it.item_id}`);
  out.push(
    "Rules: do the work on a branch and open a pull request for review. Never merge, roll or deploy. Say in the PR what changed and why.",
  );
  const text = out.join("\n");
  const max = a.max ?? 3800;
  return text.length > max ? `${text.slice(0, max - 40).trimEnd()}\n[cut: the canvas has the rest]` : text;
}

export type WorkState = "queued" | "running" | "pr" | "done" | "failed";

/** The state to show for a queued ask or job record from the live picture. */
export function stateOf(r: Rec): { state: WorkState; label: string } {
  const s = str(r, "state").toLowerCase();
  if (/fail|drop|cancel|error/.test(s)) return { state: "failed", label: "Didn't go through" };
  if (/done|merged|complete|resolved/.test(s)) return { state: "done", label: "Done" };
  if (str(r, "pr") || /pr/.test(s)) return { state: "pr", label: "PR open" };
  if (/run|active|claimed|working/.test(s)) return { state: "running", label: "Running" };
  return { state: "queued", label: "Queued" };
}

/** The asks in the live picture that this canvas filed: by the `canvas:<id>` source. */
export function asksFor(
  asks: Iterable<Rec>,
  canvasId: string,
): { id: string; text: string; state: WorkState; label: string }[] {
  const out: { id: string; text: string; state: WorkState; label: string }[] = [];
  for (const a of asks) {
    if (str(a, "source") !== `canvas:${canvasId}` && str(a, "page") !== `canvas:${canvasId}`) continue;
    out.push({ id: str(a, "id"), text: str(a, "title") || str(a, "ask"), ...stateOf(a) });
  }
  return out;
}
