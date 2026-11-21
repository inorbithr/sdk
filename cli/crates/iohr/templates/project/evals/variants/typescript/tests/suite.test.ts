// The passes-on-init test: the fixture suite meets its bar offline, the bar bites when an answer
// is wrong, and the harness refuses a broken dataset or bar instead of grading less.

import assert from "node:assert/strict";
import fs from "node:fs";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import { describe, mock, test } from "node:test";

import { DEFAULT_BAR, DEFAULT_DATASET, SuiteError, loadCases } from "../src/dataset.ts";
import { errors, evaluate, loadBar, mean, missed, passRate } from "../src/runner.ts";
import { SCORERS, exact, format } from "../src/scorers.ts";
import { LABELS, StubSolver } from "../src/solver.ts";
import { main } from "../src/suite.ts";

async function run(...argv: string[]): Promise<{ code: number; out: string; err: string }> {
  let out = "";
  let err = "";
  const code = await main(argv, { out: (t) => (out += t + "\n"), err: (t) => (err += t + "\n") });
  return { code, out, err };
}

function tmpFile(name: string, text: string): string {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "evals-"));
  const file = path.join(dir, name);
  fs.writeFileSync(file, text);
  return file;
}

describe("the fixture", () => {
  test("meets its bar with no network", async () => {
    const connect = mock.method(net.Socket.prototype, "connect", () => {
      throw new Error("the suite opened a network connection");
    });
    const fetch = mock.method(globalThis, "fetch", () => {
      throw new Error("the suite opened a network connection");
    });
    const { code, out } = await run();
    assert.equal(code, 0, out);
    assert.match(out, /BAR MET/);
    assert.equal(connect.mock.callCount() + fetch.mock.callCount(), 0);
    mock.restoreAll();
  });

  test("the dataset is a dozen labelled cases covering every label", () => {
    const cases = loadCases(DEFAULT_DATASET);
    assert.equal(cases.length, 12);
    assert.deepEqual(new Set(cases.map((c) => c.expected)), new Set(LABELS));
  });

  test("the report says the solver is a stub and holds ids and scores, never inputs", async () => {
    const { code, out } = await run("--json");
    const report = JSON.parse(out);
    assert.equal(code, 0);
    assert.equal(report.stub, true);
    assert.ok(report.solver.startsWith("stub-"));
    assert.equal(report.bar_met, true);
    assert.equal(report.pass_rate, 1);
    for (const c of loadCases(DEFAULT_DATASET)) assert.ok(!out.includes(c.input), c.input);
  });

  test("two runs give the same report", async () => {
    assert.deepEqual(await run("--json"), await run("--json"));
  });
});

describe("the bar bites", () => {
  test("lowering one expected answer fails the suite and names the case", async () => {
    const rows = fs.readFileSync(DEFAULT_DATASET, "utf8").split("\n").filter((l) => l.trim()).map((l) => JSON.parse(l));
    rows[0].expected = "other";
    const file = tmpFile("cases.jsonl", rows.map((r) => JSON.stringify(r)).join("\n") + "\n");
    const { code, out } = await run("--dataset", file);
    assert.equal(code, 1, out);
    assert.match(out, /BAR MISSED/);
    assert.ok(out.includes(`FAILED ${rows[0].id}`));
  });

  test("a chatty answer fails the format scorer even when it is right", async () => {
    class Chatty extends StubSolver {
      override readonly name: string = "stub-chatty";
      override solve(c: Parameters<StubSolver["solve"]>[0]): string {
        return `The label is ${super.solve(c)}.`;
      }
    }
    const report = await evaluate(loadCases(DEFAULT_DATASET), new Chatty());
    assert.equal(mean(report, "format"), 0);
    assert.ok(missed(report, loadBar(DEFAULT_BAR)).length > 0);
  });

  test("a solver that throws is an error and never a pass", async () => {
    const broken = {
      name: "stub-broken",
      stub: true,
      solve(): string {
        throw new Error("provider down");
      },
    };
    const report = await evaluate(loadCases(DEFAULT_DATASET), broken);
    assert.equal(errors(report), 12);
    assert.equal(passRate(report), 0);
    assert.ok(missed(report, loadBar(DEFAULT_BAR)).some((m) => m.includes("error")));
  });
});

describe("the harness refuses a broken suite", () => {
  test("a duplicate id, an empty field and an empty dataset stop the run", async () => {
    const row = '{"id": "a", "input": "i", "expected": "bug"}\n';
    const broken: Array<[string, string]> = [
      [row + row, "twice"],
      ['{"id": "a", "input": "", "expected": "bug"}\n', "input"],
      ["\n", "no cases"],
      ["not json\n", "not JSON"],
    ];
    for (const [text, why] of broken) {
      const { code, err } = await run("--dataset", tmpFile("c.jsonl", text));
      assert.equal(code, 2, why);
      assert.ok(err.includes(why), `${why}: ${err}`);
    }
  });

  test("a scorer with no bar stops the run", async () => {
    const bar = tmpFile("t.toml", "[bar]\nmin_pass_rate = 0.5\nmax_errors = 0\n[bar.scorers]\nexact = 0.5\n");
    const { code, err } = await run("--bar", bar);
    assert.equal(code, 2);
    assert.ok(err.includes("no bar for the scorer(s) format"), err);
  });

  test("a bar that is not understood stops the run", async () => {
    for (const text of [
      "[bar]\nmin_pass_rate = high\n",
      "min_pass_rate = 1\n",
      "[bar]\nmin_pass_rate = 2\nmax_errors = 0\n[bar.scorers]\nexact = 1\nformat = 1\n",
    ]) {
      assert.equal((await run("--bar", tmpFile("t.toml", text))).code, 2, text);
    }
  });

  test("a missing bar or dataset stops the run", async () => {
    assert.equal((await run("--bar", "/nonexistent/t.toml")).code, 2);
    assert.equal((await run("--dataset", "/nonexistent/c.jsonl")).code, 2);
    assert.throws(() => loadBar("/nonexistent/t.toml"), SuiteError);
  });
});

describe("the scorers", () => {
  const billing = loadCases(DEFAULT_DATASET)[0]!;

  test("exact ignores case and space only", () => {
    assert.equal(exact(billing, " Billing\n"), 1);
    assert.equal(exact(billing, "bug"), 0);
  });

  test("format wants one bare label", () => {
    assert.equal(format(billing, "billing"), 1);
    for (const bad of ["Billing", "billing.", "billing, bug", ""]) assert.equal(format(billing, bad), 0, bad);
  });

  test("every scorer has a bar", () => {
    assert.deepEqual(Object.keys(loadBar(DEFAULT_BAR).scorers).sort(), Object.keys(SCORERS).sort());
  });
});
