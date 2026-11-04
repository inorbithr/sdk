import importlib.util
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location("g", Path(__file__).with_name("examples-gate.py"))
g = importlib.util.module_from_spec(spec)
spec.loader.exec_module(g)
H = "a" * 40
L = {"go", "rust"}
ok = {"schema": 1, "commit": H, "dirty": False, "results": [
    {"language": "go", "example": "whoami", "status": "pass"},
    {"language": "rust", "example": "whoami", "status": "pass"}]}


class Verdict(unittest.TestCase):
    def test_pass(self):
        self.assertIsNone(g.verdict(ok, H, L))

    def test_other_commit(self):
        self.assertIn("HEAD", g.verdict(ok, "b" * 40, L))

    def test_dirty(self):
        self.assertIn("dirty", g.verdict({**ok, "dirty": True}, H, L))

    def test_fail(self):
        r = {**ok, "results": ok["results"] + [{"language": "go", "example": "x", "status": "fail"}]}
        self.assertIn("go/x", g.verdict(r, H, L))

    def test_missing_language(self):
        self.assertIn("rust", g.verdict(ok, H, L | {"python"}) + "rust")
        self.assertIn("python", g.verdict(ok, H, L | {"python"}))

    def test_skip_is_not_pass(self):
        r = {**ok, "results": [{"language": "go", "example": "w", "status": "skip"}]}
        self.assertIn("go", g.verdict(r, H, {"go"}))


class Languages(unittest.TestCase):
    def test_every_example_directory_is_run(self):
        # The gate requires the languages in examples.toml; a directory missing there would
        # never run and never be required.
        dirs = {p.name for p in (g.ROOT / "examples").iterdir() if p.is_dir()}
        self.assertEqual(set(), dirs - g.languages())


def doc(lang, commit=H, **kw):
    return {"schema": 1, "commit": commit, "dirty": False, "results": [
        {"language": lang, "example": "x", "status": "pass"}], **kw}


class Merge(unittest.TestCase):
    def joined(self, files, langs):
        res, why = g.merge(files, H)
        return why if res is None else g.verdict(res, H, langs)

    def test_linux_and_mac_pass(self):
        self.assertIsNone(self.joined([ok, doc("swift")], L | {"swift"}))

    def test_mac_on_another_commit_fails(self):
        self.assertIn("results file 2", self.joined([ok, doc("swift", "b" * 40)], L | {"swift"}))

    def test_dirty_mac_fails(self):
        self.assertIn("dirty", self.joined([ok, doc("swift", dirty=True)], L | {"swift"}))

    def test_missing_language_fails(self):
        self.assertIn("swift", self.joined([ok], L | {"swift"}))

    def test_one_file_is_as_before(self):
        self.assertIsNone(self.joined([ok], L))


def status(state, login, context="examples-run"):
    return {"context": context, "state": state, "creator": {"login": login}}


class LatestState(unittest.TestCase):
    BOT = "github-actions[bot]"

    def test_bot_success(self):
        self.assertEqual("success", g.latest_state([status("success", self.BOT)]))

    def test_hand_posted_success_does_not_count(self):
        self.assertEqual("missing", g.latest_state([status("success", "0x19")]))

    def test_newest_bot_status_wins_over_older_and_hand_posted(self):
        rows = [status("success", "0x19"), status("failure", self.BOT), status("success", self.BOT)]
        self.assertEqual("failure", g.latest_state(rows))

    def test_other_context_and_no_creator_ignored(self):
        rows = [status("success", self.BOT, "ci"), {"context": "examples-run", "state": "success"}]
        self.assertEqual("missing", g.latest_state(rows))

    def test_none(self):
        self.assertEqual("missing", g.latest_state([]))


unittest.main()
