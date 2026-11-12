"""The passes-on-init test: golden output in text and JSON, exit codes, and a version that
carries the commit."""

import io
import os
import subprocess
import sys
import unittest
from pathlib import Path

from @@MODULE@@.cli import FAILURE, OK, USAGE, run

ROOT = Path(__file__).resolve().parent.parent
COMMIT = "0123456789ab"


def call(*args, commit=COMMIT):
    out, err = io.StringIO(), io.StringIO()
    code = run(list(args), out, err, commit)
    return out.getvalue(), err.getvalue(), code


def golden(name: str) -> str:
    return (ROOT / "golden" / name).read_text()


class Cli(unittest.TestCase):
    def test_golden_text(self):
        for name, args in [
            ("help.txt", ["--help"]),
            ("greet.txt", ["greet", "Ada"]),
            ("version.txt", ["version"]),
        ]:
            out, _, code = call(*args)
            self.assertEqual((code, out), (OK, golden(name)), name)

    def test_golden_json(self):
        for name, args in [
            ("greet.json", ["--json", "greet", "Ada"]),
            ("version.json", ["version", "--json"]),
        ]:
            out, _, code = call(*args)
            self.assertEqual((code, out), (OK, golden(name)), name)

    def test_exit_codes(self):
        cases = [
            ([], USAGE),
            (["greet"], OK),
            (["nope"], USAGE),
            (["greet", "a", "b"], USAGE),
            (["greet", "   "], FAILURE),
            (["greet", "x" * 65], FAILURE),
        ]
        for args, want in cases:
            self.assertEqual(call(*args)[2], want, args)
        self.assertEqual(call("nope")[0], "", "a usage error writes to stderr, not stdout")

    def test_the_real_process_exits_with_the_same_codes(self):
        env = {**os.environ, "PYTHONPATH": str(ROOT / "src"), "PYTHONDONTWRITEBYTECODE": "1"}
        def proc(*args):
            return subprocess.run([sys.executable, "-m", "@@MODULE@@", *args],
                                  capture_output=True, text=True, env=env, timeout=30)
        self.assertEqual(proc("greet", "Ada").stdout, golden("greet.txt"))
        self.assertEqual(proc("nope").returncode, USAGE)
        self.assertEqual(proc("greet", " ").returncode, FAILURE)
        # No fixed commit here: whatever the build says, the field is there.
        line = proc("version").stdout.strip()
        self.assertRegex(line, r"\(commit ([0-9a-f]{7,40}|unknown)\)$")

    def test_an_error_does_not_echo_a_hostile_argument(self):
        _, err, _ = call("\x1b[31m" + "x" * 100)
        self.assertNotIn("\x1b", err)
        self.assertLess(len(err), 200)


if __name__ == "__main__":
    unittest.main()
