"""Validate every conformance case and vector against its schema.

Cases (`conformance/cases/<area>/*.yaml`) against `case.schema.json`; vectors
(`conformance/vectors/<kind>/*.yaml`) against `vector.schema.json`. Also checks that a
file's name matches its `name` and its directory its `area` or `kind`.
Run through `mise run conformance:validate`.
"""

import json
import sys
from pathlib import Path

import jsonschema
import yaml

ROOT = Path(__file__).resolve().parent.parent
KINDS = (
    ("cases", "area", json.loads((ROOT / "conformance/case.schema.json").read_text())),
    ("vectors", "kind", json.loads((ROOT / "conformance/vector.schema.json").read_text())),
)


def main() -> int:
    failures = 0
    counts = []
    for directory, group, schema in KINDS:
        files = sorted((ROOT / "conformance" / directory).glob("*/*.yaml"))
        counts.append(f"{len(files)} {directory}")
        for path in files:
            doc = yaml.safe_load(path.read_text())
            rel = path.relative_to(ROOT)
            try:
                jsonschema.validate(doc, schema)
            except jsonschema.ValidationError as err:
                print(f"{rel}: {err.message}", file=sys.stderr)
                failures += 1
                continue
            if doc["name"] != path.stem:
                print(f"{rel}: name is {doc['name']!r}, file is {path.stem!r}", file=sys.stderr)
                failures += 1
            if doc[group] != path.parent.name:
                print(f"{rel}: {group} is {doc[group]!r}, directory is {path.parent.name!r}", file=sys.stderr)
                failures += 1
    print(f"{', '.join(counts)}, {failures} failing")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
