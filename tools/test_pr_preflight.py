import importlib.util
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location("p", Path(__file__).with_name("pr-preflight.py"))
p = importlib.util.module_from_spec(spec)
spec.loader.exec_module(p)
RULES = p.title_rules((p.ROOT / p.PR_TITLE_YML).read_text())
GOOD = "Implements: RFC 0153\nPRD: 0221\nADR: 0193\nMonitors: none (a CI check; nothing to watch)\n"


class Title(unittest.TestCase):
    def test_rules_come_from_pr_title_yml(self):
        types, scopes, require = RULES
        self.assertIn("feat", types)
        self.assertIn("revert", types)
        self.assertIn("deps", scopes)
        self.assertFalse(require)

    def test_good(self):
        for t in ("ci: add a preflight", "feat(go,rust)!: break", "fix(deps): bump"):
            self.assertEqual(p.check_title(t, RULES), [], t)

    def test_bad(self):
        self.assertIn("type `Feat`", p.check_title("Feat: x", RULES)[0])
        self.assertIn("scope `security`", p.check_title("ci(security): x", RULES)[0])
        self.assertIn("empty scope", p.check_title("ci(): x", RULES)[0])
        self.assertIn("is not `type(scope): subject`", p.check_title("add a thing", RULES)[0])

    def test_scope_required(self):
        self.assertIn("needs a scope", p.check_title("ci: x", (["ci"], ["go"], True))[0])

    def test_yaml_block_stops_at_next_key(self):
        text = "x:\n  types: |\n    a\n    b # c\n  scopes: |\n    go\n  requireScope: true\n"
        self.assertEqual(p.yaml_block(text, "types"), ["a", "b"])
        self.assertEqual(p.title_rules(text), (["a", "b"], ["go"], True))


class Body(unittest.TestCase):
    def test_good(self):
        self.assertEqual(p.check_body(GOOD, "ci: x"), [])

    def test_kind_named_twice(self):
        got = p.check_body(GOOD.replace("PRD: 0221", "PRD: PRD 0182"), "ci: x")
        self.assertEqual(len(got), 1)
        self.assertIn("names the kind twice", got[0])

    def test_prose_does_not_count(self):
        got = p.check_body("This follows RFC 0171 and PRD 0221.\nADR: 1\nMonitors: m1\n", "ci: x")
        self.assertEqual([g.split("`")[1] for g in got], ["PRD: <n>", "Implements: RFC <n>"])

    def test_core_spellings(self):
        body = "Part of: rfc-0158.1\nPRD #39\nADR_142\nMonitors: mon_1\n"
        self.assertEqual(p.check_body(body, "ci: x"), [])

    def test_fix_needs_study(self):
        self.assertIn("Study: <n>", p.check_body(GOOD, "fix(go): x")[0])
        self.assertEqual(p.check_body(GOOD + "Study: 0094.87\n", "fix(go): x"), [])
        self.assertIn("Study", p.check_body(GOOD, "make it work", "fix/thing")[0])
        self.assertEqual(p.check_body(GOOD, "ci: x", "fix/thing"), [])

    def test_monitors_as_core_reads_them(self):
        m = p.monitors_problem
        self.assertIsNone(m("Monitors: none.rs"))
        self.assertIsNone(m("Monitors: None - a rename in a test"))
        self.assertIsNone(m("* Monitors: mon_1, mon_2"))
        self.assertIn("without a reason", m("Monitors: none ()"))
        self.assertIn("without a reason", m("Monitors: none (<reason>)"))
        self.assertIn("without a reason", m("Monitors: none"))
        self.assertIn("lacks", m("Watched by monitors: mon_1"))
        self.assertIn("lacks", m("Monitors: <ids>"))
        self.assertIsNone(m("Monitors:\nMonitors: none (later line counts)"))


class Report(unittest.TestCase):
    def test_marker_and_counts(self):
        findings, md = p.report("bad", "", "main", run_legs=False)
        self.assertTrue(md.startswith(p.MARKER))
        self.assertIn(f"will fail {len(findings)} check(s)", md)
        self.assertEqual(p.report("ci: x", GOOD, "main", run_legs=False)[0], [])

    def test_annotation_escape(self):
        self.assertEqual(p.escape("a%b\nc"), "a%25b%0Ac")


if __name__ == "__main__":
    unittest.main()
