// The scorers: each takes the case and the solver's answer and returns 0 or 1.
//
// A scorer is a pure function. Keep it deterministic: the same case and answer give the same
// score, with no clock, no randomness and no network, so a change in the score is a change in
// the solver. (A model that grades another model's answer is a solver of its own: label it a
// stub in tests and give it a bar of its own.)

import type { Case } from "./dataset.ts";
import { LABELS } from "./solver.ts";

export type Scorer = (c: Case, answer: string) => 0 | 1;

/** The answer is the expected label, ignoring case and surrounding space. */
export const exact: Scorer = (c, answer) =>
  answer.trim().toLowerCase() === c.expected.trim().toLowerCase() ? 1 : 0;

/**
 * The answer is one bare label from the list and nothing else (no sentence, no full stop). A
 * model that is right but chatty breaks the program that reads its answer, so it fails here.
 */
export const format: Scorer = (_c, answer) => ((LABELS as readonly string[]).includes(answer) ? 1 : 0);

export const SCORERS: Readonly<Record<string, Scorer>> = { exact, format };
