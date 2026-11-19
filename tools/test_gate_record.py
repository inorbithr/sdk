#!/usr/bin/env python3
"""tools/ci-changed.sh leaves core's gate record (devops/ci/gates.py) on a clean pass, and
nothing on a failure, a dry run or a dirty checkout. Runs the real script in a temporary
repository with a stub `mise` on PATH and XDG_STATE_HOME in a temporary directory."""

import hashlib
import json
import os
import pathlib
import shutil
import subprocess
import tempfile
import unittest

TOOLS = pathlib.Path(__file__).resolve().parent
# The key as core's gates.key() builds it with CI unset: schema, $CI, then the command.
KEY = hashlib.sha256("\0".join(["1", "", "mise", "run", "ci:changed"]).encode()).hexdigest()[:32]


class GateRecord(unittest.TestCase):
    def setUp(self):
        self.tmp = pathlib.Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.tmp)
        self.repo = self.tmp / "repo"
        self.state = self.tmp / "state"
        bin_dir = self.tmp / "bin"
        bin_dir.mkdir()
        # The stub runs every task: MISE_FAIL fails it, MISE_TOUCH leaves a file behind.
        stub = bin_dir / "mise"
        stub.write_text(
            "#!/usr/bin/env bash\n"
            '[ -n "${MISE_TOUCH:-}" ] && touch "$MISE_TOUCH"\n'
            '[ -n "${MISE_FAIL:-}" ] && exit 1\n'
            "exit 0\n"
        )
        stub.chmod(0o755)
        (self.repo / "tools").mkdir(parents=True)
        for name in ("ci-changed.sh", "gate-record.py"):
            shutil.copy(TOOLS / name, self.repo / "tools" / name)
        self.git("init", "-q", "-b", "main")
        self.git("add", ".")
        self.git("-c", "user.name=t", "-c", "user.email=t@localhost", "commit", "-q", "-m", "init")
        self.git("update-ref", "refs/remotes/origin/main", "HEAD")
        self.env = {
            k: v for k, v in os.environ.items()
            if k not in ("CI", "CI_CHANGED_BASE", "GATES_MIRROR", "ROLL_WHAT")
        }
        self.env.update(
            PATH=f"{bin_dir}{os.pathsep}{os.environ['PATH']}",
            XDG_STATE_HOME=str(self.state),
            IOHR_BIN="/bin/true",
        )

    def git(self, *args):
        return subprocess.run(
            ["git", *args], cwd=self.repo, check=True, capture_output=True, text=True
        ).stdout.strip()

    def run_gate(self, *args, **env):
        return subprocess.run(
            ["bash", "tools/ci-changed.sh", *args], cwd=self.repo,
            env={**self.env, **env}, capture_output=True, text=True,
        )

    def records(self):
        return sorted(p.relative_to(self.state) for p in self.state.rglob("*") if p.is_file())

    def test_a_clean_pass_writes_cores_record(self):
        out = self.run_gate()
        self.assertEqual(out.returncode, 0, out.stderr)
        tree = self.git("rev-parse", "HEAD^{tree}")
        path = self.state / "tbd" / "gates" / tree / f"{KEY}.json"
        self.assertEqual(self.records(), [path.relative_to(self.state)])
        rec = json.loads(path.read_text())
        self.assertEqual(
            sorted(rec),
            ["by", "checkout", "cmd", "commit", "host", "passed_at", "seconds", "tree"],
        )
        self.assertEqual(rec["tree"], tree)
        self.assertEqual(rec["cmd"], ["mise", "run", "ci:changed"])
        self.assertEqual(rec["commit"], self.git("rev-parse", "HEAD"))
        self.assertRegex(rec["passed_at"], r"^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ$")
        self.assertIsInstance(rec["seconds"], float)
        self.assertEqual(rec["by"], "ci:changed")
        self.assertEqual(pathlib.Path(rec["checkout"]).resolve(), self.repo.resolve())
        self.assertIn("recorded the pass", out.stdout)

    def test_ci_is_part_of_the_key(self):
        self.assertEqual(self.run_gate(CI="true").returncode, 0)
        ci_key = hashlib.sha256(
            "\0".join(["1", "true", "mise", "run", "ci:changed"]).encode()
        ).hexdigest()[:32]
        self.assertEqual([p.name for p in self.records()], [f"{ci_key}.json"])

    def test_a_failed_task_writes_nothing(self):
        self.assertNotEqual(self.run_gate(MISE_FAIL="1").returncode, 0)
        self.assertEqual(self.records(), [])

    def test_a_dry_run_writes_nothing(self):
        self.assertEqual(self.run_gate("--dry-run").returncode, 0)
        self.assertEqual(self.records(), [])

    def test_a_dirty_checkout_writes_nothing_and_says_why(self):
        (self.repo / "untracked.txt").write_text("x\n")
        out = self.run_gate()
        self.assertEqual(out.returncode, 0, out.stderr)
        self.assertEqual(self.records(), [])
        lines = [l for l in out.stdout.splitlines() if "no gate record" in l]
        self.assertEqual(len(lines), 1, out.stdout)
        self.assertIn("uncommitted or untracked", lines[0])

    def test_a_run_that_dirties_the_checkout_writes_nothing(self):
        out = self.run_gate(MISE_TOUCH=str(self.repo / "left-behind.txt"))
        self.assertEqual(out.returncode, 0, out.stderr)
        self.assertEqual(self.records(), [])
        self.assertIn("changed during the run", out.stdout)

    def test_another_base_writes_nothing(self):
        out = self.run_gate(CI_CHANGED_BASE="HEAD")
        self.assertEqual(out.returncode, 0, out.stderr)
        self.assertEqual(self.records(), [])
        self.assertIn("CI_CHANGED_BASE", out.stdout)


if __name__ == "__main__":
    unittest.main()
