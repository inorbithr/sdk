// Your tools. Two examples: one that is safe to run, one that changes something and so needs
// an answer first. Replace them; keep `needsApproval` on anything that writes, sends or deletes.

import type { Tool } from "./agent.ts";

export const TOOLS: Tool[] = [
  { name: "add", fn: (a) => Number(a["a"]) + Number(a["b"]) },
  // An example of a tool that changes something: it only pretends to here.
  { name: "delete_note", fn: (a) => `deleted ${String(a["id"])}`, needsApproval: true },
];

// Only these may run. A tool the model asks for that is not here is refused.
export const ALLOW = new Set(["add"]);
