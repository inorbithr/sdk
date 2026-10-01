"""Validate every conformance case against conformance/case.schema.json.

Also checks that a case's file name matches its `name` and its directory its `area`.
Run through `mise run conformance:validate`.
"""

import json
import sys
from pathlib import Path

import jsonschema
import yaml

ROOT = Path(__file__).resolve().parent.parent
SCHEMA = json.loads((ROOT / "conformance/case.schema.json").read_text())


def main() -> int:
    failures = 0
    cases = sorted((ROOT / "conformance/cases").glob("*/*.yaml"))
    for path in cases:
        case = yaml.safe_load(path.read_text())
        rel = path.relative_to(ROOT)
        try:
            jsonschema.validate(case, SCHEMA)
        except jsonschema.ValidationError as err:
            print(f"{rel}: {err.message}", file=sys.stderr)
            failures += 1
            continue
        if case["name"] != path.stem:
            print(f"{rel}: name is {case['name']!r}, file is {path.stem!r}", file=sys.stderr)
            failures += 1
        if case["area"] != path.parent.name:
            print(f"{rel}: area is {case['area']!r}, directory is {path.parent.name!r}", file=sys.stderr)
            failures += 1
    print(f"{len(cases)} cases, {failures} failing")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
