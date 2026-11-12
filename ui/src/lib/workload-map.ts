// The workload map: everything that works for the account (agents, coordinator jobs, asks, runners and
// the items waiting on a person) as one tree of nodes, read from the API's picture and agent list. Pure.
// A port of the Mac app's workload-map.ts to what the API tells: the Mac's local session registry and
// process stats have no web source, so the map has no session, terminal or process nodes.
import { num, type Picture, type Rec, str, strs } from "./picture.ts";

export type NodeState = "working" | "needs" | "idle" | "stuck" | "failed" | "done";
export type NodeKind = "group" | "agent" | "runner" | "job" | "ask" | "open" | "more";

export type WNode = {
  id: string;
  kind: NodeKind;
  label: string;
  /** One short line: the stage, the slots used, the version. */
  sub: string;
  state: NodeState;
  /** What the node says it is doing or waiting for; empty when nothing says. */
  latest: string;
  parent: string | null;
  children: string[];
  /** The repository a job names, for grouping. */
  group: string;
  /** Who holds it (a runner, a user); empty when nothing says. */
  owner: string;
  /** True for a node whose data is a stub fixture (shown as stub). */
  stub: boolean;
};

export type WorkloadMap = { nodes: Map<string, WNode>; roots: string[] };

export const STATE_WORD: Record<NodeState, string> = {
  working: "working",
  needs: "needs you",
  idle: "idle",
  stuck: "stuck",
  failed: "failed",
  done: "done",
};
export const STATE_COLOR: Record<NodeState, string> = {
  working: "#10b981",
  needs: "#f59e0b",
  idle: "#94a3b8",
  stuck: "#f97316",
  failed: "#ef4444",
  done: "#64748b",
};
/** The CSS token each state reads in the page's dots, with STATE_COLOR as the fallback. */
export const STATE_TOKEN: Record<NodeState, string> = {
  working: "--success",
  needs: "--warning",
  idle: "--muted-foreground",
  stuck: "--warning",
  failed: "--destructive",
  done: "--muted-foreground",
};
/** The states to filter by, worst first. */
export const STATES: NodeState[] = ["failed", "needs", "stuck", "working", "idle", "done"];
const RANK: Record<NodeState, number> = { failed: 0, needs: 1, stuck: 2, working: 3, idle: 4, done: 5 };
/** The worse of two states (failed before needs before stuck before working; idle and done are quiet). */
export const worseState = (a: NodeState, b: NodeState): NodeState => (RANK[a] <= RANK[b] ? a : b);

const human = (w: string) => (w ? w.charAt(0).toUpperCase() + w.slice(1).replace(/[_-]+/g, " ") : "");
const short = (id: string) => id.replace(/^[a-z]+_/i, "").slice(-6);
/** A repository's last path part from a github.com link or `owner/repo`. */
export const repoName = (r: string): string =>
  r
    .replace(/\.git$/, "")
    .split("/")
    .filter(Boolean)
    .pop() ?? "";

export const G_AGENTS = "g:agents";
export const G_RUNNERS = "g:runners";
export const G_SERVICE = "g:service";

export function jobState(j: Rec): NodeState {
  const stage = str(j, "stage");
  const mode = str(j, "mode");
  if (/fail|error/i.test(stage)) return "failed";
  if (stage === "done" || stage === "stopped") return "done";
  if (mode === "held") return "stuck";
  if (mode === "awaiting" || mode === "retry") return "needs";
  return "working";
}

export function askState(a: Rec): NodeState {
  const st = str(a, "state") || "queued";
  if (st === "building") return "working";
  if (st === "stuck") return "stuck";
  if (st === "done" || st === "closed" || st === "withdrawn") return "done";
  return str(a, "wait_code") ? "needs" : "idle";
}

export function runnerState(r: Rec): NodeState {
  const st = str(r, "status").toLowerCase();
  if (/offline|lost|revoked|dead/.test(st)) return "stuck";
  return num(r, "slots_used") > 0 ? "working" : "idle";
}

export function agentState(a: Rec): NodeState {
  const st = str(a, "status").toLowerCase();
  if (st === "revoked") return "failed";
  if (st === "offline") return "done";
  return "idle";
}

/** The agents the list answer holds (`{agents: [...]}` or a bare array), as plain records. */
export const agentsOf = (v: unknown): Rec[] => {
  const raw = Array.isArray(v) ? v : (v as { agents?: unknown } | null)?.agents;
  return Array.isArray(raw) ? raw.filter((x) => x && typeof x === "object").map((x) => x as Rec) : [];
};

/** Of the finished jobs only the newest ones stay on the map. */
const DONE_KEEP = 40;

/**
 * Build the map. Three planets: the coordinator service (asks, with their jobs under them, and the
 * items waiting on a person), the runners, and the agents. A job sits under the ask that lists it, else
 * under the service. Pure: the same picture and agents give the same map.
 */
export function buildMap(p: Picture, agents: readonly Rec[]): WorkloadMap {
  const nodes = new Map<string, WNode>();
  const add = (n: Omit<WNode, "children" | "group" | "owner" | "stub" | "latest"> & Partial<WNode>) => {
    nodes.set(n.id, { children: [], group: "", owner: "", stub: false, latest: "", ...n });
    if (n.parent) nodes.get(n.parent)?.children.push(n.id);
  };
  const root = (id: string, label: string) => add({ id, kind: "group", label, sub: "", state: "idle", parent: null });
  root(G_SERVICE, "Coordinator");
  root(G_RUNNERS, "Runners");
  root(G_AGENTS, "Agents");

  const runnerName = (id: string) => (id ? str(p.runners.get(id) ?? {}, "name") || id : "");
  const doneJobs = new Set(
    [...p.jobs.values()]
      .filter((j) => jobState(j) === "done")
      .sort((a, b) =>
        (str(b, "updated_at") || str(b, "created_at")).localeCompare(str(a, "updated_at") || str(a, "created_at")),
      )
      .slice(DONE_KEEP)
      .map((j) => str(j, "id") || str(j, "job_id")),
  );
  const jobAsk = new Map<string, string>();
  for (const a of p.asks.values()) for (const j of strs(a, "job_ids")) jobAsk.set(j, str(a, "id"));

  for (const a of p.asks.values()) {
    const id = str(a, "id");
    const st = askState(a);
    if (!id || str(a, "state") === "withdrawn") continue; // a withdrawn ask is gone from the map
    add({
      id,
      kind: "ask",
      label: str(a, "title") || `${human(str(a, "kind")) || "New work"} ask ${short(id)}`,
      sub: human(str(a, "state") || "queued"),
      state: st,
      latest: str(a, "reason") || human(str(a, "wait_code")),
      parent: G_SERVICE,
      owner: str(a, "created_by"),
    });
  }
  for (const j of p.jobs.values()) {
    const id = str(j, "id") || str(j, "job_id");
    if (!id || doneJobs.has(id)) continue;
    const mode = str(j, "mode");
    const parent = nodes.has(jobAsk.get(id) ?? "") ? (jobAsk.get(id) as string) : G_SERVICE;
    const stage = str(j, "stage") || "intake";
    add({
      id,
      kind: "job",
      label: str(j, "title") || `${human(str(j, "kind")) || "Job"} job ${short(id)}`,
      sub: human(mode && mode !== "running" ? mode : stage),
      state: jobState(j),
      latest: mode === "held" ? str(j, "hold_reason") : mode === "awaiting" ? human(str(j, "await_kind")) : "",
      parent,
      group: repoName(str(j, "repository")),
      owner: runnerName(str(j, "runner_id")) || str(j, "user") || str(j, "created_by"),
    });
  }
  for (const o of p.open.values()) {
    const id = `open:${str(o, "id")}`;
    const kind = str(o, "kind");
    add({
      id,
      kind: "open",
      label: str(o, "title") || `${human(kind) || "Open item"} ${short(str(o, "id"))}`,
      sub: human(kind),
      state: "needs",
      latest: str(o, "next_action"),
      parent: G_SERVICE,
      owner: str(o, "user"),
      stub: o.stub === true,
    });
  }
  for (const r of p.runners.values()) {
    const id = str(r, "id");
    if (!id) continue;
    add({
      id,
      kind: "runner",
      label: str(r, "name") || `Runner ${short(id)}`,
      sub: `${num(r, "slots_used")} of ${num(r, "capacity")} slots`,
      state: runnerState(r),
      latest: str(r, "status"),
      parent: G_RUNNERS,
    });
  }
  for (const a of agents) {
    const id = str(a, "agent_id") || str(a, "id");
    if (!id) continue;
    add({
      id,
      kind: "agent",
      label: str(a, "name") || `Agent ${short(id)}`,
      sub: [str(a, "environment"), str(a, "version")].filter(Boolean).join(" · "),
      state: agentState(a),
      latest: str(a, "status"),
      parent: G_AGENTS,
    });
  }
  for (const g of [G_SERVICE, G_RUNNERS, G_AGENTS]) {
    const n = nodes.get(g) as WNode;
    const under = moonNodes({ nodes, roots: [] }, g);
    n.state = under.reduce<NodeState>((w, c) => worseState(w, c.state), under.length ? "done" : "idle");
    n.sub = under.length ? `${under.length} in orbit` : "nothing here";
  }
  return { nodes, roots: [G_SERVICE, G_RUNNERS, G_AGENTS] };
}

/** Every node under a planet, through asks to their jobs, in map order. Groups are never moons. */
export function moonNodes(m: WorkloadMap, id: string): WNode[] {
  const out: WNode[] = [];
  const walk = (cid: string) => {
    const n = m.nodes.get(cid);
    if (!n) return;
    if (n.kind !== "group") out.push(n);
    n.children.forEach(walk);
  };
  m.nodes.get(id)?.children.forEach(walk);
  return out;
}

/** The path from a planet down to a node, as ids; empty for an unknown id. */
export function pathTo(m: WorkloadMap, id: string): string[] {
  const out: string[] = [];
  for (let cur = m.nodes.get(id); cur; cur = cur.parent ? m.nodes.get(cur.parent) : undefined) out.unshift(cur.id);
  return out;
}

/** How many nodes sit in each state, groups left out. */
export function layerCounts(m: WorkloadMap): Record<NodeState, number> {
  const c: Record<NodeState, number> = { working: 0, needs: 0, idle: 0, stuck: 0, failed: 0, done: 0 };
  for (const n of m.nodes.values()) if (n.kind !== "group") c[n.state]++;
  return c;
}

export type BlockedItem = { id: string; label: string; state: NodeState; why: string; who: string };

/** What is held up: failed, stuck and needs-you nodes, with why and who unblocks it, worst first. */
export function blocked(m: WorkloadMap): BlockedItem[] {
  const out: BlockedItem[] = [];
  for (const n of m.nodes.values()) {
    if (n.kind === "group" || n.kind === "more") continue;
    if (n.state !== "failed" && n.state !== "needs" && n.state !== "stuck") continue;
    const who =
      n.kind === "open" || n.state === "needs" ? "a person" : n.kind === "runner" ? "an operator" : "the coordinator";
    out.push({ id: n.id, label: n.label, state: n.state, why: n.latest || n.sub || STATE_WORD[n.state], who });
  }
  return out.sort((a, b) => RANK[a.state] - RANK[b.state] || a.label.localeCompare(b.label));
}

/** A moon's orbit speed in radians a second: working moves, needs you and stuck crawl, idle and done stand still. */
const MOON_SPEED: Record<NodeState, number> = {
  working: 0.7,
  needs: 0.25,
  stuck: 0.15,
  failed: 0,
  idle: 0.08,
  done: 0,
};
/** At most this many moons on a planet; the rest fold into one "+N more" moon, worst first so the quiet ones fold. */
export const MOON_CAP = 20;

export type WorkMoon = { id: string; name: string; tone: NodeState; speed: number; say: string };
export type WorkPlanet = { id: string; name: string; tone: NodeState; moons: WorkMoon[]; badge: string; say: string };

/** The orbit view's planets, worst first, each with its moons (worst first, capped). Pure. */
export function orbitPlanets(m: WorkloadMap): WorkPlanet[] {
  const out: WorkPlanet[] = [];
  for (const id of m.roots) {
    const g = m.nodes.get(id);
    if (!g) continue;
    const under = moonNodes(m, id);
    const moons: WorkMoon[] = under
      .map((c) => ({
        id: c.id,
        name: c.label,
        tone: c.state,
        speed: MOON_SPEED[c.state],
        say: [STATE_WORD[c.state], c.sub, c.latest].filter(Boolean).join(", "),
      }))
      .sort((a, b) => RANK[a.tone] - RANK[b.tone] || a.name.localeCompare(b.name));
    const kept =
      moons.length > MOON_CAP
        ? [
            ...moons.slice(0, MOON_CAP - 1),
            {
              id: `${id}:more`,
              name: `+${moons.length - MOON_CAP + 1} more`,
              tone: moons.slice(MOON_CAP - 1).reduce<NodeState>((w, c) => worseState(w, c.tone), "done"),
              speed: 0,
              say: "folded; open the rows view for each",
            },
          ]
        : moons;
    out.push({
      id,
      name: g.label,
      tone: g.state,
      moons: kept,
      badge: g.sub,
      say: `${g.label}: ${STATE_WORD[g.state]}, ${under.length} in orbit.`,
    });
  }
  return out.sort((a, b) => RANK[a.tone] - RANK[b.tone] || a.name.localeCompare(b.name));
}

export type RoomTile = {
  id: string;
  name: string;
  kind: NodeKind | "jobs";
  sub: string;
  state: NodeState;
  owner: string;
  latest: string;
  members: string[];
  blocked?: BlockedItem;
};

/**
 * The room view's tiles: one per agent and runner, one per job group (jobs by repository). Blocked
 * tiles first, then worst state, then name. Pure.
 */
export function roomTiles(m: WorkloadMap): RoomTile[] {
  const held = new Map(blocked(m).map((b) => [b.id, b]));
  const out: RoomTile[] = [];
  const groups = new Map<string, WNode[]>();
  for (const n of m.nodes.values()) {
    if (n.kind === "job") groups.set(n.group || "jobs", [...(groups.get(n.group || "jobs") ?? []), n]);
    else if (n.kind === "agent" || n.kind === "runner")
      out.push({
        id: n.id,
        name: n.label,
        kind: n.kind,
        sub: n.sub,
        state: n.state,
        owner: n.owner,
        latest: n.latest,
        members: [n.id],
        blocked: held.get(n.id),
      });
  }
  for (const [g, jobs] of groups) {
    const worst = jobs.reduce<NodeState>((w, j) => worseState(w, j.state), "done");
    const first = [...jobs].sort((a, b) => RANK[a.state] - RANK[b.state])[0] as WNode;
    const stuck = jobs.map((j) => held.get(j.id)).find((b) => b);
    out.push({
      id: `jg:${g}`,
      name: g,
      kind: "jobs",
      sub: `${jobs.length} job${jobs.length === 1 ? "" : "s"} · ${jobs.filter((j) => j.state === "working").length} working`,
      state: worst,
      owner: first.owner,
      latest: `${first.label}: ${first.latest || first.sub}`,
      members: jobs.map((j) => j.id),
      blocked: stuck,
    });
  }
  return out.sort(
    (a, b) =>
      Number(!!b.blocked) - Number(!!a.blocked) || RANK[a.state] - RANK[b.state] || a.name.localeCompare(b.name),
  );
}

export type TreeRow = { id: string; depth: number; node: WNode; open: boolean; hasChildren: boolean };

/** The tree view's visible rows: depth first, a collapsed node hides what is under it. `collapsed` holds ids; `only` limits to a state set when given. */
export function treeRows(m: WorkloadMap, collapsed: ReadonlySet<string>, only?: ReadonlySet<NodeState>): TreeRow[] {
  const out: TreeRow[] = [];
  const keeps = (id: string): boolean => {
    const n = m.nodes.get(id);
    return !!n && (!only || only.has(n.state) || n.children.some(keeps));
  };
  const walk = (id: string, depth: number) => {
    const n = m.nodes.get(id);
    if (!n || !keeps(id)) return;
    const open = !collapsed.has(id);
    out.push({ id, depth, node: n, open, hasChildren: n.children.length > 0 });
    if (open)
      [...n.children]
        .sort((a, b) => RANK[(m.nodes.get(a) as WNode).state] - RANK[(m.nodes.get(b) as WNode).state])
        .forEach((c) => {
          walk(c, depth + 1);
        });
  };
  for (const r of m.roots) walk(r, 0);
  return out;
}
