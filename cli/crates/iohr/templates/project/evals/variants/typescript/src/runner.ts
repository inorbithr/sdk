// Run a solver over the cases, score each answer and check the result against the bar.

import fs from "node:fs";
import path from "node:path";

import { type Case, SuiteError, reason } from "./dataset.ts";
import { SCORERS, type Scorer } from "./scorers.ts";
import type { Solver } from "./solver.ts";

export interface Bar {
  readonly minPassRate: number;
  readonly maxErrors: number;
  readonly scorers: Readonly<Record<string, number>>;
}

export interface Result {
  readonly id: string;
  readonly answer: string | null; // null when the solver threw
  readonly scores: Record<string, number>;
  readonly error: string | null;
}

export interface Report {
  readonly solver: string;
  readonly stub: boolean;
  readonly results: Result[];
}

export const passed = (r: Result): boolean => r.error === null && Object.values(r.scores).every((s) => s === 1);
export const passes = (rep: Report): number => rep.results.filter(passed).length;
export const passRate = (rep: Report): number => passes(rep) / rep.results.length;
export const errors = (rep: Report): number => rep.results.filter((r) => r.error !== null).length;
export const mean = (rep: Report, scorer: string): number =>
  rep.results.reduce((sum, r) => sum + (r.scores[scorer] ?? 0), 0) / rep.results.length;

const TABLE = /^\[([A-Za-z0-9_.]+)\]$/;
const KEY = /^([A-Za-z0-9_]+)\s*=\s*([+-]?[0-9]+(\.[0-9]+)?)$/;

/**
 * The small part of TOML the bar uses: `[table]` lines, `key = number` lines and `#` comments.
 * Node has no TOML reader and the bar needs nothing more, so anything else is refused, not guessed at.
 */
function parseTables(name: string, text: string): Map<string, Map<string, string>> {
  const tables = new Map<string, Map<string, string>>();
  let current: Map<string, string> | null = null;
  text.split("\n").forEach((raw, i) => {
    const line = raw.split("#")[0]!.trim();
    if (line === "") return;
    const t = TABLE.exec(line);
    if (t) {
      if (tables.has(t[1]!)) throw new SuiteError(`${name}: not valid TOML (line ${i + 1}: table [${t[1]}] twice)`);
      current = new Map();
      tables.set(t[1]!, current);
      return;
    }
    const k = KEY.exec(line);
    if (!k || current === null) {
      throw new SuiteError(`${name}: not valid TOML (line ${i + 1}: only \`[table]\` and \`key = number\` are read here)`);
    }
    if (current.has(k[1]!)) throw new SuiteError(`${name}: not valid TOML (line ${i + 1}: key ${k[1]} twice)`);
    current.set(k[1]!, k[2]!);
  });
  return tables;
}

export function loadBar(file: string, scorers: Readonly<Record<string, Scorer>> = SCORERS): Bar {
  const name = path.basename(file);
  let text: string;
  try {
    text = fs.readFileSync(file, "utf8");
  } catch (e) {
    throw new SuiteError(`cannot read the bar ${file}: ${reason(e)}`);
  }
  const tables = parseTables(name, text);
  const bar = tables.get("bar");
  if (!bar) throw new SuiteError(`${name}: the [bar] table is missing`);
  const per = tables.get("bar.scorers");
  const rate = Number(bar.get("min_pass_rate"));
  const max = Number(bar.get("max_errors"));
  if (!per || !bar.has("min_pass_rate") || !bar.has("max_errors") || !Number.isInteger(max)) {
    throw new SuiteError(`${name}: [bar] needs min_pass_rate, max_errors and [bar.scorers]`);
  }
  const rates = Object.fromEntries([...per].map(([k, v]) => [k, Number(v)]));
  if (![rate, ...Object.values(rates)].every((v) => v >= 0 && v <= 1)) {
    throw new SuiteError(`${name}: every rate is a number from 0 to 1`);
  }
  if (max < 0) throw new SuiteError(`${name}: max_errors cannot be negative`);
  const unguarded = Object.keys(scorers).filter((s) => !(s in rates)).sort();
  if (unguarded.length > 0) {
    throw new SuiteError(`${name}: no bar for the scorer(s) ${unguarded.join(", ")}; a scorer with no bar guards nothing`);
  }
  const unknown = Object.keys(rates).filter((s) => !(s in scorers)).sort();
  if (unknown.length > 0) throw new SuiteError(`${name}: a bar for ${unknown.join(", ")}, which is not a scorer`);
  return { minPassRate: rate, maxErrors: max, scorers: rates };
}

export async function evaluate(
  cases: readonly Case[],
  solver: Solver,
  scorers: Readonly<Record<string, Scorer>> = SCORERS,
): Promise<Report> {
  const results: Result[] = [];
  for (const c of cases) {
    try {
      const answer = await solver.solve(c);
      const scores = Object.fromEntries(Object.entries(scorers).map(([n, s]) => [n, s(c, answer)]));
      results.push({ id: c.id, answer, scores, error: null });
    } catch (e) {
      // a broken solver is an error case, never a pass
      const scores = Object.fromEntries(Object.keys(scorers).map((n) => [n, 0]));
      results.push({ id: c.id, answer: null, scores, error: (e as Error)?.constructor?.name ?? "Error" });
    }
  }
  return { solver: solver.name, stub: solver.stub, results };
}

/** Every line of the bar the report does not reach; empty means the bar is met. */
export function missed(rep: Report, bar: Bar): string[] {
  const out: string[] = [];
  if (passRate(rep) < bar.minPassRate) {
    out.push(`pass rate ${passRate(rep).toFixed(3)} is below ${bar.minPassRate.toFixed(3)}`);
  }
  if (errors(rep) > bar.maxErrors) out.push(`${errors(rep)} error(s), at most ${bar.maxErrors} allowed`);
  for (const [name, minimum] of Object.entries(bar.scorers)) {
    if (mean(rep, name) < minimum) {
      out.push(`scorer ${name}: mean ${mean(rep, name).toFixed(3)} is below ${minimum.toFixed(3)}`);
    }
  }
  return out;
}
