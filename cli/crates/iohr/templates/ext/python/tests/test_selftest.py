"""The self-test `iohr ext dev --test` runs, with the same token channel, STATE and
CACHE as `iohr ext dev`: one heartbeat, then checks what it wrote and where."""

import json
import os
import subprocess
import sys
import unittest

HERE = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


class SelfTest(unittest.TestCase):
    def test_one_heartbeat_in_state_and_nothing_elsewhere(self):
        before = sorted(os.listdir(HERE))
        env = dict(os.environ, IOHR_EXT_ONCE="1", PYTHONDONTWRITEBYTECODE="1")
        env["PYTHONPATH"] = os.path.join(HERE, "src")
        subprocess.run([sys.executable, "-m", "@@MODULE@@"], env=env, check=True, cwd=HERE)
        with open(os.path.join(os.environ["STATE"], "heartbeat.json"), encoding="utf-8") as f:
            beat = json.load(f)
        self.assertEqual(beat["id"], "@@ID@@")
        self.assertEqual(beat["state"], "ok")
        # Granted when you are signed in, refused when not: either way the channel answered.
        self.assertIn(beat["token"], ("granted", "refused"))
        self.assertEqual(sorted(os.listdir(HERE)), before, "the extension wrote into its folder")

    def test_an_undeclared_scope_is_refused(self):
        sys.path.insert(0, os.path.join(HERE, "src"))
        from importlib import import_module

        channel = import_module("@@MODULE@@.channel")
        with self.assertRaises(channel.Refused):
            channel.token(["tokens:write"])


if __name__ == "__main__":
    unittest.main()
