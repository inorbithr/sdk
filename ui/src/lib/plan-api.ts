// The plans API as a source of the Planner's plan (RFC 0171 slice 3, RFC 0166 section 3): the
// coordinator's plan and its items (core #1107, `GET /v1/coordinator/plans/{plan_id}`) are turned
// into the same event lines the plan file holds and folded by the same `applyLines`, so there is
// one store and one fold; the local jsonl is the cache the server's lines land on (its space ids
// and canvas diagrams are kept where the API names a document or diagram by id alone). The plan
// stream (`/v1/coordinator/plan/events`, core #1108) says when a plan changed; planner-store.ts
// then reads the plan again. Pure, tested in tests/plan-api.test.ts.

import { applyLines, type Plan } from "./planner.ts";

/** `Plan` of the plans API, the fields this page reads. */
export type ApiPlan = {
  id: string;
  title?: string;
  external_id?: string;
  state?: string;
  version?: number;
  updated_at?: string;
  item_count?: number;
};
/** `PlanItem` of the plans API; `gate` is RFC 0171 slice 1's word and is read when the item carries it. */
export type ApiItem = {
  item_id: string;
  kind?: string;
  parent?: string;
  title?: string;
  status?: string;
  body?: string;
  doc?: string;
  diagram?: string;
  preview?: string;
  ask_id?: string;
  prs?: string[];
  position?: number;
  version?: number;
  updated_by?: string;
  updated_at?: string;
  gate?: unknown;
  blocked?: unknown;
};
export type ApiNote = { id?: string; item_id?: string; author?: string; body?: string; created_at?: string };
export type ApiPlanResponse = { plan?: ApiPlan; items?: ApiItem[]; notes?: ApiNote[] };

const isObj = (v: unknown): v is Record<string, unknown> => !!v && typeof v === "object" && !Array.isArray(v);
const snakeKey = (k: string) => k.replace(/[A-Z]/g, (c) => `_${c.toLowerCase()}`);

/** The gateway may spell keys camelCase or snake_case (proto JSON); the fold reads snake_case. */
export function snake(v: unknown): unknown {
  if (Array.isArray(v)) return v.map(snake);
  if (!isObj(v)) return v;
  return Object.fromEntries(Object.entries(v).map(([k, x]) => [snakeKey(k), snake(x)]));
}

/**
 * The plan the API holds for a local plan id: the server plan whose `external_id` is the local id
 * (the plugin files plans under the file's id), else the one whose id is the local id itself.
 */
export function serverPlanFor(localId: string, plans: ApiPlan[]): ApiPlan | null {
  return plans.find((p) => p.external_id === localId) ?? plans.find((p) => p.id === localId) ?? null;
}

/** One line per fact of the server's plan, in the plan file's own event shape. */
export function serverLines(r: ApiPlanResponse): string {
  const o = snake(r) as ApiPlanResponse;
  const lines: Record<string, unknown>[] = [];
  const plan = o.plan;
  if (plan)
    lines.push({
      t: "plan",
      ...(plan.title ? { title: plan.title } : {}),
      at: plan.updated_at ?? "",
      by: "coordinator",
    });
  const items = [...(o.items ?? [])].sort((a, b) => (a.position ?? 0) - (b.position ?? 0));
  for (const i of items) {
    if (!i.item_id) continue;
    const l: Record<string, unknown> = {
      t: "item",
      id: i.item_id,
      at: i.updated_at ?? "",
      by: i.updated_by || "coordinator",
    };
    if (i.kind) l.kind = i.kind;
    if (i.title) l.title = i.title;
    if (i.status) l.status = i.status;
    if (typeof i.body === "string") l.body = i.body;
    if (i.parent !== undefined) l.parent = i.parent;
    if (i.doc) l.doc = i.doc;
    if (i.diagram) l.diagram = i.diagram;
    if (i.prs && i.prs.length) l.prs = i.prs;
    if (i.gate !== undefined) l.gate = i.gate;
    else if (i.blocked !== undefined) l.gate = i.blocked;
    lines.push(l);
  }
  for (const n of o.notes ?? []) {
    if (!n.body) continue;
    lines.push({
      t: "activity",
      item: n.item_id ?? "",
      agent: n.author || "coordinator",
      text: n.body,
      at: n.created_at ?? "",
    });
  }
  return lines.map((l) => JSON.stringify(l)).join("\n") + "\n";
}

/** The server's plan folded over the cache: the server's facts win where it has them; the cache keeps the rest. A note the cache already holds as activity (same text, same time) is not added twice. */
export function foldServer(cache: Plan, r: ApiPlanResponse): Plan {
  const o = snake(r) as ApiPlanResponse;
  const had = new Set(cache.activity.map((a) => `${a.at}|${a.text}`));
  const notes = (o.notes ?? []).filter((n) => !had.has(`${n.created_at ?? ""}|${n.body ?? ""}`));
  return applyLines(cache, serverLines({ ...o, notes }));
}

/** The fold of the plan stream, keeping only what the store needs: the newest `seq` and `updated_at` per plan, so a change re-reads that plan. */
export type StreamState = {
  seq: number;
  ready: boolean;
  plans: Map<string, { seq: number; updated_at: string; external_id?: string }>;
};
export const emptyStream = (): StreamState => ({ seq: 0, ready: false, plans: new Map() });

const num = (v: unknown) => (typeof v === "number" ? v : typeof v === "string" && /^-?\d+$/.test(v) ? Number(v) : 0);
const str = (v: unknown) => (typeof v === "string" ? v : "");

export function foldStream(s: StreamState, raw: unknown): StreamState {
  const f = snake(raw);
  if (!isObj(f)) return s;
  const put = (plans: StreamState["plans"], p: unknown, seq: number) => {
    if (!isObj(p) || !str(p.plan_id)) return;
    plans.set(str(p.plan_id), {
      seq: num(p.seq) || seq,
      updated_at: str(p.updated_at),
      ...(str(p.external_id) ? { external_id: str(p.external_id) } : {}),
    });
  };
  if (isObj(f.snapshot)) {
    const plans: StreamState["plans"] = new Map();
    const seq = num(f.snapshot.seq);
    for (const p of Array.isArray(f.snapshot.plans) ? f.snapshot.plans : []) put(plans, p, seq);
    return { seq, ready: true, plans };
  }
  if (isObj(f.delta)) {
    const plans = new Map(s.plans);
    const seq = num(f.delta.seq) || s.seq;
    if (f.delta.op === "remove" && isObj(f.delta.plan)) plans.delete(str(f.delta.plan.plan_id));
    else put(plans, f.delta.plan, seq);
    return { ...s, seq, plans };
  }
  if (isObj(f.ended)) return { ...s, ready: false };
  return s;
}
