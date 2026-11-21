import sys
import types
import unittest

# A stand-in for PyYAML that records which loader the importer asks for, so the check needs
# no package and builds no object.
calls = []
fake = types.ModuleType("yaml")
fake.Loader, fake.FullLoader, fake.UnsafeLoader = object(), object(), object()
fake.SafeLoader, fake.CSafeLoader = object(), object()


def _load(stream, Loader=None):
    calls.append(Loader)
    return {"name": "nightly"}


def _safe_load(stream):
    calls.append(fake.SafeLoader)
    return {"name": "nightly"}


fake.load, fake.safe_load = _load, _safe_load
fake.full_load = fake.unsafe_load = lambda stream: (calls.append(None), {})[1]
sys.modules["yaml"] = fake

import importer  # noqa: E402


class EvalCheck(unittest.TestCase):
    def test_uploads_use_the_safe_loader(self):
        calls.clear()
        importer.load_settings("name: nightly")
        self.assertTrue(calls, "load_settings did not call yaml")
        self.assertTrue(
            all(c in (fake.SafeLoader, fake.CSafeLoader) for c in calls),
            "an upload is parsed with a loader that builds arbitrary objects",
        )
