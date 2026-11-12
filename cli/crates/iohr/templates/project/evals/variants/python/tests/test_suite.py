"""The passes-on-init test: the fixture suite meets its bar offline, the bar bites when an
answer is wrong, and the harness refuses a broken dataset or bar instead of grading less."""

import contextlib
import io
import json
import socket
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from @@MODULE@@.__main__ import main
from @@MODULE@@.dataset import DEFAULT_BAR, DEFAULT_DATASET, SuiteError, load_cases
from @@MODULE@@.runner import evaluate, load_bar, missed
from @@MODULE@@.scorers import SCORERS, exact, format_ok
from @@MODULE@@.solver import LABELS, StubSolver


def run(argv):
    out, err = io.StringIO(), io.StringIO()
    with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
        code = main(argv)
    return code, out.getvalue(), err.getvalue()


def no_network():
    def refuse(*a, **k):
        raise AssertionError("the suite opened a network connection")

    return mock.patch.multiple(socket.socket, connect=refuse, connect_ex=refuse)


class Fixture(unittest.TestCase):
    def test_the_fixture_suite_meets_its_bar_with_no_network(self):
        with no_network():
            code, out, _ = run([])
        self.assertEqual(code, 0, out)
        self.assertIn("BAR MET", out)

    def test_the_dataset_is_about_a_dozen_labelled_cases_covering_every_label(self):
        cases = load_cases(DEFAULT_DATASET)
        self.assertEqual(len(cases), 12)
        self.assertEqual({c.expected for c in cases}, set(LABELS))

    def test_the_report_says_the_solver_is_a_stub(self):
        code, out, _ = run(["--json"])
        report = json.loads(out)
        self.assertEqual(code, 0)
        self.assertIs(report["stub"], True)
        self.assertTrue(report["solver"].startswith("stub-"))
        self.assertTrue(report["bar_met"])
        self.assertEqual(report["pass_rate"], 1.0)

    def test_the_report_holds_ids_and_scores_never_inputs_or_answers(self):
        _, out, _ = run(["--json"])
        for case in load_cases(DEFAULT_DATASET):
            self.assertNotIn(case.input, out)

    def test_two_runs_give_the_same_report(self):
        self.assertEqual(run(["--json"]), run(["--json"]))


class TheBarBites(unittest.TestCase):
    def with_dataset(self, change):
        """A copy of the dataset with one case changed by `change(row)`; returns its path."""
        rows = [json.loads(line) for line in DEFAULT_DATASET.read_text().splitlines() if line.strip()]
        change(rows[0])
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        path = Path(tmp.name) / "cases.jsonl"
        path.write_text("\n".join(json.dumps(r) for r in rows) + "\n")
        return path, rows[0]["id"]

    def test_lowering_one_expected_answer_fails_the_suite(self):
        path, case_id = self.with_dataset(lambda row: row.update(expected="other"))
        code, out, _ = run(["--dataset", str(path)])
        self.assertEqual(code, 1, out)
        self.assertIn("BAR MISSED", out)
        self.assertIn(f"FAILED {case_id}", out)

    def test_a_chatty_answer_fails_the_format_scorer_even_when_it_is_right(self):
        class Chatty(StubSolver):
            name = "stub-chatty"

            def solve(self, case):
                return f"The label is {super().solve(case)}."

        report = evaluate(load_cases(DEFAULT_DATASET), Chatty())
        self.assertEqual(report.mean("format"), 0)
        self.assertTrue(missed(report, load_bar(DEFAULT_BAR)))

    def test_a_solver_that_raises_is_an_error_and_never_a_pass(self):
        class Broken(StubSolver):
            def solve(self, case):
                raise RuntimeError("provider down")

        report = evaluate(load_cases(DEFAULT_DATASET), Broken())
        self.assertEqual(report.errors, 12)
        self.assertEqual(report.pass_rate, 0)
        self.assertTrue(any("error" in m for m in missed(report, load_bar(DEFAULT_BAR))))


class TheHarnessRefusesABrokenSuite(unittest.TestCase):
    def write(self, name, text):
        d = tempfile.TemporaryDirectory()
        self.addCleanup(d.cleanup)
        path = Path(d.name) / name
        path.write_text(text)
        return path

    def test_a_duplicate_id_an_empty_field_and_an_empty_dataset_stop_the_run(self):
        row = '{"id": "a", "input": "i", "expected": "bug"}\n'
        for text, why in [(row + row, "twice"), ('{"id": "a", "input": "", "expected": "bug"}\n', "input"), ("\n", "no cases"), ("not json\n", "not JSON")]:
            code, _, err = run(["--dataset", str(self.write("c.jsonl", text))])
            self.assertEqual(code, 2, why)
            self.assertIn(why, err)

    def test_a_scorer_with_no_bar_stops_the_run(self):
        bar = self.write("t.toml", "[bar]\nmin_pass_rate = 0.5\nmax_errors = 0\n[bar.scorers]\nexact = 0.5\n")
        code, _, err = run(["--bar", str(bar)])
        self.assertEqual(code, 2)
        self.assertIn("no bar for the scorer(s) format", err)

    def test_a_missing_bar_or_dataset_stops_the_run(self):
        self.assertEqual(run(["--bar", "/nonexistent/t.toml"])[0], 2)
        self.assertEqual(run(["--dataset", "/nonexistent/c.jsonl"])[0], 2)
        with self.assertRaises(SuiteError):
            load_bar(Path("/nonexistent/t.toml"))


class Scorers(unittest.TestCase):
    def test_exact_ignores_case_and_space_only(self):
        case = load_cases(DEFAULT_DATASET)[0]  # billing
        self.assertEqual(exact(case, " Billing\n"), 1)
        self.assertEqual(exact(case, "bug"), 0)

    def test_format_wants_one_bare_label(self):
        case = load_cases(DEFAULT_DATASET)[0]
        self.assertEqual(format_ok(case, "billing"), 1)
        for bad in ("Billing", "billing.", "billing, bug", ""):
            self.assertEqual(format_ok(case, bad), 0, bad)

    def test_every_scorer_has_a_bar(self):
        self.assertEqual(set(load_bar(DEFAULT_BAR).scorers), set(SCORERS))


if __name__ == "__main__":
    unittest.main()
