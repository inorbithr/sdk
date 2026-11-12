// The Orbits and Maintenance menus' rows (src/lib/page-menus.ts).
import assert from "node:assert/strict";
import { test } from "node:test";
import { flatten } from "../src/frame/context-menu-core.ts";
import { putInMaintenance } from "../src/lib/features-core.ts";
import { add, defaultOrbits } from "../src/lib/orbits-core.ts";
import { maintenanceMenu, orbitsMenu } from "../src/lib/page-menus.ts";

const pages = [
  { key: "home", title: "Home" },
  { key: "ext:a", title: "Alpha" },
  { key: "ext:b", title: "Beta" },
];
const rows = (m: ReturnType<typeof orbitsMenu>) => flatten(m).map((f) => f.action);
const noop = () => {};
const act = { go: noop, add: noop, remove: noop };

test("the orbits menu lists the picked pages that still exist, checks the current one, and steps by the list", () => {
  const o = add(add(defaultOrbits(), "ext:a"), "ext:gone");
  const r = rows(orbitsMenu(o, "ext:a", pages, act));
  assert.deepEqual(
    r.filter((x) => x.id.startsWith("orbit:ext") || x.id === "orbit:home").map((x) => [x.label, x.checked]),
    [
      ["Home", false],
      ["Alpha", true],
    ],
  );
  const by = (id: string) => r.find((x) => x.id === id);
  assert.equal(by("orbit:add")?.disabled, "Already an orbit");
  assert.equal(by("orbit:remove")?.disabled, false);
  assert.equal(by("orbit:prev")?.disabled, false);
});

test("Home cannot be removed, a page that is not an orbit can be added, and the ends have no neighbour", () => {
  const r = rows(orbitsMenu(defaultOrbits(), "home", pages, act));
  const by = (id: string) => r.find((x) => x.id === id);
  assert.equal(by("orbit:remove")?.disabled, "Home is always an orbit");
  assert.ok(by("orbit:prev")?.disabled);
  assert.ok(by("orbit:next")?.disabled);
  const other = rows(orbitsMenu(defaultOrbits(), "ext:b", pages, act));
  assert.equal(other.find((x) => x.id === "orbit:add")?.disabled, false);
  assert.ok(other.find((x) => x.id === "orbit:remove")?.disabled);
});

test("the orbit rows run the caller's actions with the page", () => {
  const ran: string[] = [];
  const m = orbitsMenu(add(defaultOrbits(), "ext:a"), "home", pages, {
    go: (p) => ran.push(`go ${p}`),
    add: (p) => ran.push(`add ${p}`),
    remove: (p) => ran.push(`remove ${p}`),
  });
  for (const a of rows(m)) if (!a.disabled) a.run?.();
  assert.deepEqual(ran, ["go home", "go ext:a", "go ext:a"]);
});

test("the maintenance menu shows each page's state, offers Bring everything online only when needed, and Undo when live", () => {
  const mact = { edit: noop, back: noop, allOnline: noop, undo: null, undoLabel: "" };
  const calm = rows(maintenanceMenu({}, mact));
  assert.deepEqual(
    calm.filter((x) => x.id.startsWith("maintenance:") && !["all", "undo"].includes(x.id.slice(12))).map((x) => x.hint),
    ["online", "online", "online"],
  );
  assert.ok(calm.find((x) => x.id === "maintenance:all")?.disabled);
  assert.equal(
    calm.find((x) => x.id === "maintenance:undo"),
    undefined,
  );

  const down = putInMaintenance({}, "home", { reason: "Moving the data", eta: "" });
  const ran: string[] = [];
  const r = rows(
    maintenanceMenu(down, {
      edit: (id) => ran.push(`edit ${id}`),
      back: (id) => ran.push(`back ${id}`),
      allOnline: () => ran.push("all"),
      undo: () => ran.push("undo"),
      undoLabel: "Put Home in maintenance",
    }),
  );
  assert.equal(r.find((x) => x.id === "maintenance:home")?.hint, "Moving the data");
  assert.equal(r.find((x) => x.id === "maintenance:home")?.checked, true);
  for (const a of r) a.run?.();
  assert.deepEqual(ran, ["back home", "edit planner", "edit extensions", "all", "undo"]);
  assert.equal(r.find((x) => x.id === "maintenance:undo")?.label, "Undo: Put Home in maintenance");
});
