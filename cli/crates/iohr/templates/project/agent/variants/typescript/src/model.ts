// A scripted model: it answers from a list, so a run is repeatable and costs nothing. It is a
// stub, labelled as one everywhere it prints. To use a real model, write an object with
// `next(history)` that calls your provider and returns its text or tool call and the tokens it
// used; the loop does not change.

import type { Message, Model, Turn } from "./agent.ts";

export const STUB = "stub: scripted model, no real model was called";

export class ScriptedModel implements Model {
  #turns: Turn[];
  constructor(turns: Turn[]) {
    this.#turns = [...turns];
  }
  next(_history: Message[]): Turn {
    return this.#turns.shift() ?? { text: "(the script has ended)" };
  }
}

export const demo = (): ScriptedModel =>
  new ScriptedModel([
    { call: { name: "add", args: { a: 2, b: 3 } }, tokens: 40 },
    { text: "2 + 3 = 5", tokens: 20 },
  ]);
