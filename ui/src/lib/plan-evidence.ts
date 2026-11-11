// The evidence gate on the Mac (RFC 0171, slice 3): what an item carries as evidence (its PRD, RFC
// and ADR with their status, its pull requests with their state, its diagram and how old it is) and
// the gate's refusal in the gate's own words ("no ADR", "diagram older than head"). One rule, one
// place: the coordinator's `item_evidenced` check is the rule; when the server has spoken (the item
// carries its `gate`), its words are shown as they are. Until it has, the same facts the Mac already
// reads (the plan, the pull request poll) say what the gate would say, so a card never claims
// "ready" with nothing behind it. Pure, tested in tests/plan-evidence.test.ts.

import { NO_STAGES, prsOfStage, type StageMap } from "./plan-stage-core.ts";
import type { DocRef, Item, Plan } from "./planner.ts";
import type { PrLive } from "./planner-shared.ts";

export const DOC_KINDS = ["prd", "rfc", "adr"] as const;
export type DocKind = (typeof DOC_KINDS)[number];

/** A document's status as Decisions says it, when it was read; absent until then. */
export type DocStatus = ReadonlyMap<string, string>;

export type DocEvidence = {
  kind: DocKind;
  /** The Decisions document; null when the plan holds the document as an item that is not linked to Decisions yet. */
  ref: DocRef | null;
  /** The plan item that names it: the step itself, or a document item of the plan. */
  item: string;
  title: string;
  /** draft, proposed, accepted, superseded... when read; "" until then. */
  status: string;
};

export type PrEvidence = {
  repo: string;
  number: number;
  ref: string;
  /** open, draft, merged, closed, or "not read" when GitHub was not asked yet. */
  state: string;
  live?: PrLive;
};

export type DiagramEvidence = {
  present: boolean;
  /** When the diagram was last saved, when known. */
  updatedAt?: string;
  /** The newest pull request head the diagram is measured against, when known. */
  headAt?: string;
  /** Diagram saved before the newest head: the gate's "diagram older than head". */
  stale: boolean;
};

export type Evidence = {
  docs: Record<DocKind, DocEvidence | null>;
  prs: PrEvidence[];
  diagram: DiagramEvidence;
  /** What is missing, in the gate's words, in the order the gate names them. */
  missing: string[];
  /** The item claims ready or done: the gate applies and a miss is a refusal. */
  claimed: boolean;
  /** The refusal, "blocked: no ADR" (+N more), or null when nothing blocks. */
  blocked: string | null;
  /** The server's gate spoke (the item carries `gate`); the words are its, not derived here. */
  fromServer: boolean;
};

export const CLAIMS = new Set(["ready", "done"]);
const prKey = (p: { repo: string; number: number }) => `${p.repo}#${p.number}`;

/** The chain an item's documents are looked for in: itself, its parents, then the whole plan. */
function chain(plan: Plan, item: Item): Item[] {
  const byId = new Map(plan.items.map((i) => [i.id, i]));
  const out: Item[] = [item];
  const seen = new Set([item.id]);
  let p = item.parent;
  while (p && !seen.has(p)) {
    const it = byId.get(p);
    if (!it) break;
    out.push(it);
    seen.add(p);
    p = it.parent;
  }
  // The documents linked to the step ("implements", or any link), then the plan's own.
  for (const l of plan.links) {
    const other = l.from === item.id ? l.to : l.to === item.id ? l.from : null;
    const it = other ? byId.get(other) : undefined;
    if (it && !seen.has(it.id)) {
      out.push(it);
      seen.add(it.id);
    }
  }
  for (const it of plan.items) {
    if (!seen.has(it.id) && it.status !== "dropped") {
      out.push(it);
      seen.add(it.id);
    }
  }
  return out;
}

const kindOf = (i: Item): DocKind | null => {
  const k = (i.doc?.kind ?? i.kind).toLowerCase();
  return (DOC_KINDS as readonly string[]).includes(k) ? (k as DocKind) : null;
};

/**
 * The item's PRD, RFC and ADR: its own document first, then its parents', then the plan's. A linked
 * document (a `doc` into Decisions) beats a plan item of that kind that is not linked yet; the
 * latter is still named, as "not linked", so the card says which fact is missing.
 */
export function documentsOf(
  plan: Plan,
  item: Item,
  status: DocStatus = new Map(),
): Record<DocKind, DocEvidence | null> {
  const docs: Record<DocKind, DocEvidence | null> = { prd: null, rfc: null, adr: null };
  for (const it of chain(plan, item)) {
    const k = kindOf(it);
    if (!k || it.status === "dropped") continue;
    const linked = !!it.doc?.document_id;
    if (docs[k] && (docs[k]!.ref || !linked)) continue;
    docs[k] = {
      kind: k,
      ref: linked ? it.doc! : null,
      item: it.id,
      title: it.title,
      status: linked ? (status.get(it.doc!.document_id) ?? "") : "",
    };
  }
  return docs;
}

const prState = (l?: PrLive): string =>
  !l ? "not read" : l.state === "OPEN" ? (l.draft ? "draft" : "open") : l.state.toLowerCase();

/** The newest head among the item's pull requests: a merge time when merged, else when it was opened. */
export function newestHead(prs: PrEvidence[]): string | undefined {
  let best: string | undefined;
  for (const p of prs) {
    const t = p.live?.merged_at || p.live?.created_at;
    if (t && (!best || t > best)) best = t;
  }
  return best;
}

export function diagramOf(item: Item, prs: PrEvidence[]): DiagramEvidence {
  const present = !!item.diagram || !!item.diagramRef;
  const updatedAt = item.diagramAt;
  const headAt = newestHead(prs);
  return {
    present,
    ...(updatedAt ? { updatedAt } : {}),
    ...(headAt ? { headAt } : {}),
    stale: !!(present && updatedAt && headAt && updatedAt < headAt),
  };
}

/** The gate's words for what is missing, from the facts on the Mac. */
export function missingOf(
  docs: Record<DocKind, DocEvidence | null>,
  prs: PrEvidence[],
  diagram: DiagramEvidence,
): string[] {
  const out: string[] = [];
  for (const k of DOC_KINDS) {
    if (!docs[k]) out.push(`no ${k.toUpperCase()}`);
    else if (!docs[k]!.ref) out.push(`${k.toUpperCase()} not linked`);
  }
  for (const p of prs) if (p.live?.state === "CLOSED") out.push(`PR #${p.number} closed without merging`);
  if (!diagram.present) out.push("no diagram");
  else if (diagram.stale) out.push("diagram older than head");
  return out;
}

/** "blocked: no ADR", "blocked: no ADR +2": the first miss and how many more. */
export const blockedLine = (missing: string[]): string | null =>
  missing.length ? `blocked: ${missing[0]}${missing.length > 1 ? ` +${missing.length - 1}` : ""}` : null;

export function evidenceOf(
  plan: Plan,
  item: Item,
  prNow: Map<string, PrLive> = new Map(),
  status: DocStatus = new Map(),
  stages: StageMap = NO_STAGES,
): Evidence {
  const docs = documentsOf(plan, item, status);
  // The pull requests an item is about are the plan machine's one parse (plan-stage-core.ts), not a second reading here.
  const prs = prsOfStage(stages.get(item.id)).map((p) => {
    const live = prNow.get(prKey(p));
    return { ...p, ref: prKey(p), state: prState(live), ...(live ? { live } : {}) };
  });
  const diagram = diagramOf(item, prs);
  const claimed = CLAIMS.has(item.status);
  // The server's word stands as it is; the Mac's reading fills in only while the server has not spoken.
  const fromServer = !!item.gate;
  const missing = fromServer ? (item.gate!.ok ? [] : item.gate!.reasons.slice()) : missingOf(docs, prs, diagram);
  return { docs, prs, diagram, missing, claimed, blocked: claimed ? blockedLine(missing) : null, fromServer };
}

/** Every item of a plan the gate refuses now, in plan order. */
export function blockedItems(
  plan: Plan,
  prNow: Map<string, PrLive> = new Map(),
  status: DocStatus = new Map(),
  stages: StageMap = NO_STAGES,
): { item: Item; blocked: string }[] {
  const out: { item: Item; blocked: string }[] = [];
  for (const item of plan.items) {
    if (item.status === "dropped" || item.kind === "goal" || item.kind === "note") continue;
    const e = evidenceOf(plan, item, prNow, status, stages);
    if (e.blocked) out.push({ item, blocked: e.blocked });
  }
  return out;
}

/** "3 h ago", "2 d ago": how old a diagram is, against now. */
export function ageOf(iso: string | undefined, now = Date.now()): string {
  const t = iso ? Date.parse(iso) : NaN;
  if (!Number.isFinite(t)) return "age unknown";
  const m = Math.max(0, Math.round((now - t) / 60_000));
  if (m < 1) return "just now";
  if (m < 60) return `${m} min old`;
  const h = Math.round(m / 60);
  if (h < 48) return `${h} h old`;
  return `${Math.round(h / 24)} d old`;
}
