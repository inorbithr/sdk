// The passes-on-init test: a scripted run within budget; a run that would pass a budget stops
// with a named reason; a tool outside the allow-list is refused and never runs; a tool that
// needs approval does not run without an answer.

import assert from "node:assert/strict";
import { test } from "node:test";

import { DEFAULT_BUDGET, runAgent } from "../src/agent.ts";
import type { Options, Tool, Turn } from "../src/agent.ts";
import { ScriptedModel, demo } from "../src/model.ts";
import { ALLOW, TOOLS } from "../src/tools.ts";

const ask = (turns: Turn[], o: Partial<Options> = {}) =>
  runAgent("task", { model: new ScriptedModel(turns), tools: TOOLS, allow: ALLOW, ...o });
const addCall = (tokens = 10): Turn => ({ call: { name: "add", args: { a: 1, b: 1 } }, tokens });

test("a scripted run finishes within the budget", () => {
  const run = runAgent("add 2 and 3", { model: demo(), tools: TOOLS, allow: ALLOW });
  assert.equal(run.stop, "done");
  assert.equal(run.text, "2 + 3 = 5");
  assert.equal(run.toolCalls, 1);
  assert.ok(run.steps <= DEFAULT_BUDGET.maxSteps);
  assert.deepEqual(run.history[1], { role: "tool", name: "add", result: 5 });
});

test("a run that would pass a budget stops and names it", () => {
  const forever = Array.from({ length: 50 }, () => addCall());
  assert.equal(ask(forever, { budget: { maxSteps: 8, maxToolCalls: 3, maxTokens: 4000 } }).stop, "budget_tool_calls");
  assert.equal(ask(forever, { budget: { maxSteps: 2, maxToolCalls: 9, maxTokens: 4000 } }).stop, "budget_steps");
  const heavy = Array.from({ length: 5 }, () => addCall(3000));
  const run = ask(heavy, { budget: { maxSteps: 8, maxToolCalls: 9, maxTokens: 4000 } });
  assert.equal(run.stop, "budget_tokens");
  assert.ok(run.tokens <= 4000);
});

test("a tool outside the allow-list is refused and never runs", () => {
  const ran: unknown[] = [];
  const spy: Tool = { name: "shell", fn: (a) => ran.push(a) };
  const run = ask([{ call: { name: "shell", args: { cmd: "rm -rf /" } } }, { text: "ok" }], {
    tools: [...TOOLS, spy],
  });
  assert.deepEqual(ran, []);
  assert.deepEqual(run.history[1], { role: "tool", name: "shell", error: "not_allowed" });
  assert.equal(run.stop, "done");
  assert.equal(run.toolCalls, 0);
});

test("a tool that needs approval does not run without an answer", () => {
  const ran: unknown[] = [];
  const tools: Tool[] = [{ name: "delete_note", fn: (a) => ran.push(a), needsApproval: true }];
  const call: Turn = { call: { name: "delete_note", args: { id: "n1" } } };
  const base = { tools, allow: new Set(["delete_note"]) };
  assert.equal(ask([call], base).stop, "awaiting_approval");
  assert.equal(ask([call], { ...base, approve: () => undefined }).stop, "awaiting_approval");
  const denied = ask([call, { text: "ok" }], { ...base, approve: () => false });
  assert.equal(denied.stop, "done");
  assert.deepEqual(denied.history[1], { role: "tool", name: "delete_note", error: "denied" });
  assert.equal(ran.length, 0);
  const yes = ask([call, { text: "ok" }], { ...base, approve: () => true });
  assert.equal(yes.stop, "done");
  assert.equal(ran.length, 1);
});

test("the log holds what happened and never a tool argument", () => {
  const lines: string[] = [];
  ask([{ call: { name: "add", args: { a: 424242, b: 1 } } }, { text: "secret-answer" }], {
    log: { write: (l: string) => lines.push(l) },
  });
  const text = lines.join("");
  assert.match(text, /"event":"tool"/);
  assert.ok(!text.includes("424242"));
  assert.ok(!text.includes("secret-answer"));
});
