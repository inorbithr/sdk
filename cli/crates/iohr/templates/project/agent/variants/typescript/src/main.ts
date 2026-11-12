// `node src/main.ts "your task"`: runs the loop with the scripted model and prints the
// outcome, the stop reason and what it used.

import { DEFAULT_BUDGET, runAgent } from "./agent.ts";
import { STUB, demo } from "./model.ts";
import { ALLOW, TOOLS } from "./tools.ts";

function main(): number {
  const task = process.argv.slice(2).join(" ") || "add 2 and 3";
  const run = runAgent(task, { model: demo(), tools: TOOLS, allow: ALLOW, budget: DEFAULT_BUDGET, log: process.stderr });
  console.log(`[${STUB}]`);
  console.log(run.text ?? `(stopped: ${run.stop})`);
  console.log(`stop=${run.stop} steps=${run.steps} tool_calls=${run.toolCalls} tokens=${run.tokens}`);
  return run.stop === "done" ? 0 : 1;
}

// Run as a program, not when a test imports this file.
if (import.meta.url === new URL(process.argv[1] ?? "", "file://").href) process.exit(main());
