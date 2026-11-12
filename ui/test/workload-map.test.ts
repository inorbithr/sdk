import assert from "node:assert/strict";
import { test } from "node:test";
import { applyFrame, emptyPicture } from "../src/lib/picture.ts";
import { actionsFor } from "../src/lib/workload-actions.ts";
import {
  agentsOf,
  blocked,
  buildMap,
  G_AGENTS,
  G_SERVICE,
  layerCounts,
  MOON_CAP,
  moonNodes,
  orbitPlanets,
  pathTo,
  roomTiles,
  treeRows,
} from "../src/lib/workload-map.ts";

const snap = {
  result: {
    snapshot: {
      seq: "7",
      tier: "full",
      jobs: [
        {
          id: "job_a",
          title: "Fix login",
          stage: "build",
          repository: "https://github.com/inorbithr/core",
          runner_id: "r1",
        },
        {
          id: "job_b",
          title: "Docs",
          stage: "intake",
          mode: "awaiting",
          await_kind: "approval",
          repository: "inorbithr/core",
        },
        { id: "job_c", title: "Broken", stage: "failed", repository: "inorbithr/sdk" },
        { id: "job_d", title: "Old", stage: "done" },
      ],
      asks: [
        { id: "ask_1", title: "Login bug", state: "building", job_ids: ["job_a"] },
        { id: "ask_2", title: "Gone", state: "withdrawn" },
      ],
      runners: [{ id: "r1", name: "nevio-server", status: "online", slots_used: 1, capacity: 4 }],
      open_items: [{ id: "o1", kind: "pr_open", title: "PR 12", next_action: "Review", stub: true }],
    },
  },
};
const agents = agentsOf({
  agents: [
    { agent_id: "agt_1", name: "edge", status: "online", version: "1.2" },
    { agent_id: "agt_2", name: "old", status: "revoked" },
  ],
});
const pic = () => applyFrame(emptyPicture(), snap);
const map = () => buildMap(pic(), agents);

test("a snapshot replaces the picture and the gateway wrapper is unwrapped", () => {
  const p = pic();
  assert.equal(p.seq, 7);
  assert.equal(p.tier, "full");
  assert.equal(p.jobs.size, 4);
  assert.equal(p.open.size, 1);
});

test("a delta upserts or removes one entity and advances seq; a heartbeat only seq", () => {
  let p = pic();
  p = applyFrame(p, { delta: { seq: 8, entity: "job", id: "job_e", job: { id: "job_e", stage: "build" } } });
  assert.equal(p.jobs.size, 5);
  assert.equal(p.seq, 8);
  p = applyFrame(p, { delta: { seq: 9, entity: "job", id: "job_e", op: "remove" } });
  assert.equal(p.jobs.size, 4);
  p = applyFrame(p, { heartbeat: { seq: 12 } });
  assert.equal(p.seq, 12);
  assert.equal(p.jobs.size, 4);
});

test("a job sits under its ask, else under the service; withdrawn asks are left out", () => {
  const m = map();
  assert.equal(m.nodes.get("job_a")?.parent, "ask_1");
  assert.equal(m.nodes.get("job_b")?.parent, G_SERVICE);
  assert.equal(m.nodes.has("ask_2"), false);
  assert.deepEqual(pathTo(m, "job_a"), [G_SERVICE, "ask_1", "job_a"]);
  assert.deepEqual(pathTo(m, "nope"), []);
});

test("states: working, needs, failed, done; agent revoked fails", () => {
  const m = map();
  assert.equal(m.nodes.get("job_a")?.state, "working");
  assert.equal(m.nodes.get("job_b")?.state, "needs");
  assert.equal(m.nodes.get("job_c")?.state, "failed");
  assert.equal(m.nodes.get("agt_2")?.state, "failed");
  assert.equal(m.nodes.get("r1")?.state, "working");
  assert.equal(m.nodes.get("job_a")?.group, "core");
});

test("a planet takes the worst state under it and layer counts skip groups", () => {
  const m = map();
  assert.equal(m.nodes.get(G_SERVICE)?.state, "failed");
  assert.equal(m.nodes.get(G_AGENTS)?.state, "failed");
  const c = layerCounts(m);
  assert.equal(c.failed, 2);
  assert.equal(c.needs, 2);
  assert.equal(
    Object.values(c).reduce((a, b) => a + b, 0),
    [...m.nodes.values()].filter((n) => n.kind !== "group").length,
  );
});

test("blocked lists failed and needs-you first with who unblocks", () => {
  const b = blocked(map());
  assert.equal(b[0]?.state, "failed");
  assert.ok(b.find((x) => x.id === "open:o1")?.who === "a person");
});

test("orbit planets: worst first, a stub item is marked, moons capped with a +N more moon", () => {
  const m = map();
  const planets = orbitPlanets(m);
  assert.equal(planets.length, 3);
  assert.equal(planets[0]?.tone, "failed");
  assert.equal(m.nodes.get("open:o1")?.stub, true);
  const big = emptyPicture();
  for (let i = 0; i < 30; i++) big.jobs.set(`j${i}`, { id: `j${i}`, stage: "build" });
  const bp = orbitPlanets(buildMap(big, [])).find((p) => p.id === G_SERVICE);
  assert.equal(bp?.moons.length, MOON_CAP);
  assert.match(bp?.moons.at(-1)?.name ?? "", /^\+11 more$/);
});

test("moons come in map order and finished jobs beyond the newest 40 are dropped", () => {
  const big = emptyPicture();
  for (let i = 0; i < 50; i++)
    big.jobs.set(`j${i}`, {
      id: `j${i}`,
      stage: "done",
      updated_at: `2026-11-${String(10 + (i % 10)).padStart(2, "0")}`,
    });
  assert.equal(moonNodes(buildMap(big, []), G_SERVICE).length, 40);
});

test("room tiles group jobs by repository and put blocked first", () => {
  const t = roomTiles(map());
  const core = t.find((x) => x.id === "jg:core");
  assert.equal(core?.members.length, 2);
  assert.ok(t[0]?.blocked);
});

test("tree rows fold a collapsed node and filter by state", () => {
  const m = map();
  const all = treeRows(m, new Set());
  assert.ok(all.find((r) => r.id === "job_a" && r.depth === 2));
  const folded = treeRows(m, new Set(["ask_1"]));
  assert.equal(
    folded.find((r) => r.id === "job_a"),
    undefined,
  );
  const failedOnly = treeRows(m, new Set(), new Set(["failed"]));
  assert.ok(failedOnly.every((r) => r.node.state === "failed" || r.node.children.length > 0));
  assert.equal(
    failedOnly.find((r) => r.id === "job_a"),
    undefined,
  );
});

test("actions: a live job offers release and cancel, an ask withdraw, a finished or agent node none", () => {
  const m = map();
  assert.deepEqual(
    actionsFor(m.nodes.get("job_a")!).map((a) => a.id),
    ["release", "cancel"],
  );
  assert.equal(actionsFor(m.nodes.get("job_a")!)[1]?.path, "/v1/coordinator/jobs/job_a/cancel");
  assert.deepEqual(
    actionsFor(m.nodes.get("ask_1")!).map((a) => a.id),
    ["withdraw"],
  );
  assert.deepEqual(actionsFor(m.nodes.get("agt_1")!), []);
  assert.deepEqual(actionsFor({ ...m.nodes.get("job_a")!, state: "done" }), []);
});
