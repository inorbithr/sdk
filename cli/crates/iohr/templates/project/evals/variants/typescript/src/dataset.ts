// The dataset: one JSON object per line, each with an `id`, an `input` and the `expected` answer.
// Loading is strict, so a broken dataset fails loudly instead of quietly scoring fewer cases.

import fs from "node:fs";
import path from "node:path";

export const ROOT = path.join(import.meta.dirname, "..");
export const DEFAULT_DATASET = path.join(ROOT, "datasets", "tickets.jsonl");
export const DEFAULT_BAR = path.join(ROOT, "thresholds.toml");

/** The dataset or the bar is wrong. The run stops and exits 2: nothing was graded. */
export class SuiteError extends Error {}

export interface Case {
  readonly id: string;
  readonly input: string;
  readonly expected: string;
}

export function reason(e: unknown): string {
  return (e as NodeJS.ErrnoException).code ?? String(e);
}

export function loadCases(file: string): Case[] {
  let text: string;
  try {
    text = fs.readFileSync(file, "utf8");
  } catch (e) {
    throw new SuiteError(`cannot read the dataset ${file}: ${reason(e)}`);
  }
  const name = path.basename(file);
  const cases: Case[] = [];
  const seen = new Set<string>();
  text.split("\n").forEach((line, i) => {
    const number = i + 1;
    if (line.trim() === "") return;
    let row: unknown;
    try {
      row = JSON.parse(line);
    } catch (e) {
      throw new SuiteError(`${name} line ${number}: not JSON (${(e as Error).message})`);
    }
    if (row === null || typeof row !== "object" || Array.isArray(row)) {
      throw new SuiteError(`${name} line ${number}: expected an object`);
    }
    const fields: Record<string, string> = {};
    for (const key of ["id", "input", "expected"]) {
      const v = (row as Record<string, unknown>)[key];
      if (typeof v !== "string" || v.trim() === "") {
        throw new SuiteError(`${name} line ${number}: \`${key}\` must be a non-empty string`);
      }
      fields[key] = v;
    }
    if (seen.has(fields.id!)) {
      throw new SuiteError(`${name} line ${number}: the id \`${fields.id}\` is used twice`);
    }
    seen.add(fields.id!);
    cases.push({ id: fields.id!, input: fields.input!, expected: fields.expected! });
  });
  if (cases.length === 0) {
    throw new SuiteError(`${name} has no cases: an empty suite passes nothing`);
  }
  return cases;
}
