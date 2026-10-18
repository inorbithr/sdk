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


unittest.main()
