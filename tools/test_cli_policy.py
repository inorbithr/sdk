#!/usr/bin/env python3
"""Unit tests of cli/release/policy.py: which line moves, and when nothing does."""

import importlib.util
import pathlib
import unittest

spec = importlib.util.spec_from_file_location(
    "policy", pathlib.Path(__file__).parent.parent / "cli/release/policy.py"
)
policy = importlib.util.module_from_spec(spec)
spec.loader.exec_module(policy)

TOML = '''[clients.policy.ios]
latest = "0.1.0"
minimum = "0.1.0"

[clients.policy.cli]
name = "The iohr command line"
latest = "0.1.0-alpha.14"
minimum = "0.1.0-alpha.11"

[clients.policy.agent]
latest = "0.1.0-alpha.10"
'''


class Bump(unittest.TestCase):
    def test_moves_only_the_cli_latest(self):
        new = policy.bump(TOML, "0.1.0-alpha.28")
        self.assertEqual(
            new, TOML.replace('"0.1.0-alpha.14"', '"0.1.0-alpha.28"')
        )

    def test_numeric_not_text_order(self):
        self.assertIsNone(policy.bump(TOML, "0.1.0-alpha.9"))
        self.assertIsNotNone(policy.bump(TOML, "0.1.0-alpha.15"))

    def test_same_or_older_changes_nothing(self):
        self.assertIsNone(policy.bump(TOML, "0.1.0-alpha.14"))
        self.assertIsNone(policy.bump(TOML, "0.1.0-alpha.2"))

    def test_a_release_is_above_its_prereleases(self):
        self.assertIsNotNone(policy.bump(TOML, "0.1.0"))
        stable = TOML.replace("0.1.0-alpha.14", "0.1.0")
        self.assertIsNone(policy.bump(stable, "0.1.0-alpha.28"))

    def test_a_missing_section_is_an_error(self):
        with self.assertRaises(ValueError):
            policy.bump("[clients.policy.ios]\nlatest = \"1.0.0\"\n", "0.1.0")

    def test_not_a_version(self):
        with self.assertRaises(ValueError):
            policy.bump(TOML, "latest")


if __name__ == "__main__":
    unittest.main()
