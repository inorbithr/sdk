// The solver: the thing under test. It turns a case's input into an answer.
//
// The one here is a STUB: a fixed list of keyword rules, labelled as one everywhere it shows up
// (`stub: true` in the report, a `stub-` name). It exists so the suite runs offline, is free and
// gives the same answer every time. To grade a real model, write an object with the same two
// properties and a `solve(case)` method that calls your provider, sign in the way your
// provider's subscription allows and keep the credential in your secret store, never in this
// folder. A solver that throws is counted as an error, never as a pass.

import type { Case } from "./dataset.ts";

export const LABELS = ["billing", "bug", "feature_request", "account", "other"] as const;

export interface Solver {
  readonly name: string;
  readonly stub: boolean;
  solve(c: Case): string | Promise<string>;
}

// First match wins, in this order.
const RULES: ReadonlyArray<readonly [string, readonly string[]]> = [
  ["billing", ["charged", "refund", "invoice", "payment", "card"]],
  ["bug", ["crash", "error", "blank screen"]],
  ["feature_request", ["would be great", "add an option", "feature"]],
  ["account", ["password", "account"]],
];

/** Keyword rules. Not a model: nothing is learned, nothing is called. */
export class StubSolver implements Solver {
  readonly name: string = "stub-keyword-rules-v1";
  readonly stub: boolean = true;

  solve(c: Case): string {
    const text = c.input.toLowerCase();
    for (const [label, words] of RULES) {
      if (words.some((w) => text.includes(w))) return label;
    }
    return "other";
  }
}
