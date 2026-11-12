// The layout button's menu and the Arrange menu (src/frame/arrange-menu.ts): the Mac's rows, in its order and words.
import assert from "node:assert/strict";
import { test } from "node:test";
import { applyLayout, initial } from "../src/frame/arrange.ts";
import { arrangeMenu, layoutMenu } from "../src/frame/arrange-menu.ts";
import { flatten, isAction, isSep, type MenuEntry } from "../src/frame/context-menu-core.ts";

const noop = () => {};
const labels = (rows: MenuEntry[]) => rows.map((r) => (isSep(r) ? "-" : isAction(r) ? r.label : "?"));

test("the layout menu lists the Mac's rows in its order, with its keys", () => {
  const rows = layoutMenu(initial(), { layout: noop, grid: noop, back: noop });
  assert.deepEqual(labels(rows), [
    "Side by side",
    "Two thirds and one third",
    "One third and two thirds",
    "-",
    "Left half",
    "Right half",
    "Top half",
    "Bottom half",
    "Full",
    "-",
    "Grid…",
    "-",
    "Back to before",
  ]);
  const key = (label: string) => flatten(rows).find((f) => f.action.label === label)?.action.keys;
  assert.equal(key("Left half"), "⌃⌥←");
  assert.equal(key("Right half"), "⌃⌥→");
  assert.equal(key("Top half"), "⌃⌥↑");
  assert.equal(key("Bottom half"), "⌃⌥↓");
  assert.equal(key("Full"), "⌃⌥↩");
  assert.equal(key("Grid…"), "⌃⌥G");
  assert.equal(key("Back to before"), "⌃⌥Z");
});

test("Back to before is off until a layout was applied, and every row runs its action", () => {
  const ran: string[] = [];
  const act = { layout: (id: string) => ran.push(id), grid: () => ran.push("grid"), back: () => ran.push("back") };
  const find = (rows: MenuEntry[], label: string) => flatten(rows).find((f) => f.action.label === label)?.action;
  const fresh = layoutMenu(initial(), act);
  assert.ok(find(fresh, "Back to before")?.disabled);
  const used = layoutMenu(applyLayout(initial(), "left"), act);
  assert.equal(find(used, "Back to before")?.disabled, false);
  for (const f of flatten(used)) f.action.run?.();
  assert.deepEqual(ran, [
    "side-by-side",
    "two-thirds",
    "one-third",
    "left",
    "right",
    "top",
    "bottom",
    "full",
    "grid",
    "back",
  ]);
});

test("the Arrange menu has the Mac's rows, and a box at an end cannot move further", () => {
  const act = { move: noop, reset: noop };
  const rows = arrangeMenu(initial(), "page", act);
  assert.deepEqual(labels(rows), [
    "Move left",
    "Move right",
    "Move to the start",
    "Move to the end",
    "-",
    "Reset the arrangement",
  ]);
  const off = (rows: MenuEntry[]) =>
    flatten(rows)
      .filter((f) => f.action.disabled)
      .map((f) => f.action.label);
  assert.deepEqual(off(rows), ["Move left", "Move to the start"]);
  assert.deepEqual(off(arrangeMenu(initial(), "dock", act)), ["Move right", "Move to the end"]);
});
