#!/usr/bin/env python3
"""prune-rust-caches.py keeps the newest rust-cache entry per prefix and touches nothing else."""

import importlib.util
import pathlib
import unittest

spec = importlib.util.spec_from_file_location(
    "prune", pathlib.Path(__file__).resolve().parent / "prune-rust-caches.py"
)
prune = importlib.util.module_from_spec(spec)
spec.loader.exec_module(prune)

MAIN = "refs/heads/main"


def c(key, created, ref=MAIN, i=0):
    return {"id": i, "key": key, "created_at": created, "ref": ref, "size_in_bytes": 1}


class Superseded(unittest.TestCase):
    def test_keeps_newest_per_prefix(self):
        a = "v0-rust-1.99.0-cli-Darwin-arm64-04fd21cd"
        b = "v0-rust-1.94-rust-Linux-x64-928b9d94"
        caches = [
            c(a + "-0cf127b0", "2026-11-22T12:05:00Z"),
            c(a + "-d7b2f9a9", "2026-11-22T19:52:00Z"),
            c(b + "-459007e3", "2026-11-21T11:32:00Z"),
            c(b + "-0a3c0c7b", "2026-11-22T19:35:00Z"),
            c(b + "-aaaaaaaa", "2026-11-20T00:00:00Z"),
        ]
        gone = sorted(x["key"] for x in prune.superseded(caches))
        self.assertEqual(gone, sorted([a + "-0cf127b0", b + "-459007e3", b + "-aaaaaaaa"]))

    def test_single_entry_and_other_prefixes_stay(self):
        caches = [
            c("v0-rust-lint-1.99.0-cli-lint-Linux-x64-0e86b25f-d7b2f9a9", "2026-11-22T19:31:00Z"),
            c("mise-v1-linux-x64-aa-bb", "2026-11-22T19:31:00Z"),
            c("mise-v1-linux-x64-aa-cc", "2026-11-23T19:31:00Z"),
            c("maven-Linux-b95175c7", "2026-11-20T00:00:00Z"),
        ]
        self.assertEqual(prune.superseded(caches), [])

    def test_pull_request_refs_are_never_touched(self):
        k = "v0-rust-1.99.0-cli-Linux-x64-ab1d8b4d"
        caches = [
            c(k + "-11111111", "2026-11-22T00:00:00Z", ref="refs/pull/1/merge"),
            c(k + "-22222222", "2026-11-23T00:00:00Z"),
        ]
        self.assertEqual(prune.superseded(caches), [])


if __name__ == "__main__":
    unittest.main()
