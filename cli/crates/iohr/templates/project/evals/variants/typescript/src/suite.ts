// Grade the solver on the dataset and check the pass bar.
//
// Exit 0 when the bar is met, 1 when it is missed, 2 when the dataset or the bar is wrong. The
// report says which solver ran and whether it is a stub. It holds case ids and scores only; the
// inputs and answers stay out of it (they can be customer data), unless you ask for `--show`,
// which prints the failed cases to your terminal and nowhere else.

import { parseArgs } from "node:util";

import { DEFAULT_BAR, DEFAULT_DATASET, SuiteError, loadCases } from "./dataset.ts";
import { type Report, errors, evaluate, loadBar, mean, missed, passRate, passed, passes } from "./runner.ts";
import { SCORERS } from "./scorers.ts";
import { type Solver, StubSolver } from "./solver.ts";

const round4 = (x: number): number => Math.round(x * 1e4) / 1e4;

export function toJson(rep: Report, barMissed: string[]): object {
  return {
    solver: rep.solver,
    stub: rep.stub,
    cases: rep.results.length,
    pass_rate: round4(passRate(rep)),
    errors: errors(rep),
    scorers: Object.fromEntries(Object.keys(SCORERS).map((n) => [n, round4(mean(rep, n))])),
    bar_met: barMissed.length === 0,
    missed: barMissed,
    results: rep.results.map((r) => ({ id: r.id, passed: passed(r), scores: r.scores, error: r.error })),
  };
}

export interface Io {
  out(text: string): void;
  err(text: string): void;
}

const real: Io = {
  out: (t) => process.stdout.write(t + "\n"),
  err: (t) => process.stderr.write(t + "\n"),
};

export async function main(argv: string[], io: Io = real, solver: Solver = new StubSolver()): Promise<number> {
  let values;
  try {
    ({ values } = parseArgs({
      args: argv,
      options: {
        dataset: { type: "string", default: DEFAULT_DATASET },
        bar: { type: "string", default: DEFAULT_BAR },
        json: { type: "boolean", default: false },
        show: { type: "boolean", default: false },
      },
    }));
  } catch (e) {
    io.err(`usage: main.ts [--dataset FILE] [--bar FILE] [--json] [--show] (${(e as Error).message})`);
    return 2;
  }
  let cases, bar;
  try {
    cases = loadCases(values.dataset);
    bar = loadBar(values.bar);
  } catch (e) {
    if (!(e instanceof SuiteError)) throw e;
    io.err(`suite error: ${e.message}`);
    return 2;
  }
  const report = await evaluate(cases, solver);
  const barMissed = missed(report, bar);
  if (values.json) {
    io.out(JSON.stringify(toJson(report, barMissed), null, 2));
  } else {
    const label = report.stub ? "STUB solver, not a model: " : "";
    io.out(`${label}${report.solver}: ${passes(report)}/${report.results.length} cases pass`);
    for (const name of Object.keys(SCORERS)) {
      io.out(`  ${name}: mean ${mean(report, name).toFixed(3)} (bar ${bar.scorers[name]!.toFixed(3)})`);
    }
    io.out(
      `  pass rate ${passRate(report).toFixed(3)} (bar ${bar.minPassRate.toFixed(3)}), errors ${errors(report)} (at most ${bar.maxErrors})`,
    );
    for (const r of report.results) {
      if (!passed(r)) {
        const why = r.error ?? Object.entries(r.scores).filter(([, s]) => s === 0).map(([n]) => n).join(", ");
        io.out(`  FAILED ${r.id}: ${why}`);
      }
    }
    io.out(barMissed.length === 0 ? "BAR MET" : "BAR MISSED: " + barMissed.join("; "));
  }
  if (values.show) {
    const byId = new Map(cases.map((c) => [c.id, c]));
    for (const r of report.results) {
      if (!passed(r)) {
        const c = byId.get(r.id)!;
        io.err(`  ${r.id}: input=${JSON.stringify(c.input)} expected=${JSON.stringify(c.expected)} answer=${JSON.stringify(r.answer)}`);
      }
    }
  }
  return barMissed.length === 0 ? 0 : 1;
}
