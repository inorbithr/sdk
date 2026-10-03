# /// script
# requires-python = ">=3.12"
# dependencies = []
# ///
"""Fetch the platform's public OpenAPI document and write spec/.

Run through `mise run spec:sync` (or `uv run --script tools/spec-sync.py [SOURCE]`).
SOURCE is a URL or a local file; the default is the API's public document.

Steps (spec/README.md): keep the operations marked `x-iohr-public`, keep the schemas
they reach, apply the normalisation rules N1 to N6, write spec/openapi.json,
spec/problem.json and spec/SOURCE. The output is deterministic: the same document
always produces the same files, and SOURCE keeps its date while the document's hash is
unchanged.

It also fetches the lab's generic redaction rules and conformance cases (RFC 0035),
which `iohr lab check` embeds, from the developer docs into spec/lab/ (`--lab` for
another folder or URL). `--only lab` syncs those alone and leaves the API contract as
it is, since a newer contract means regenerating every SDK.

`--check` writes nothing and exits 1 when spec/ would change.
"""

from __future__ import annotations

import argparse
import copy
import hashlib
import json
import sys
import urllib.request
from datetime import UTC, datetime
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parent.parent
SPEC = ROOT / "spec"
DEFAULT_SOURCE = "https://api.inorbit.hr/openapi.json"
DEFAULT_LAB = "https://docs.inorbit.hr/lab"
SERVER = {"url": "https://api.inorbit.hr", "description": "The InOrbit API"}
SCHEMAS = "#/components/schemas/"
METHODS = ("get", "put", "post", "delete", "options", "head", "patch", "trace")
TIMEOUT_SECONDS = 30
MAX_BYTES = 8 * 1024 * 1024
USER_AGENT = "inorbithr-sdk-spec-sync (+https://github.com/inorbithr/sdk)"

# N6: the HTTP status of every error code. The platform keeps this table in code
# (crates/protocol/src/error.rs, `Code::http`) and in prose (docs.inorbit.hr/docs/errors),
# not in the document. A code missing here, or one here that the document no longer
# has, fails the sync, so the table cannot drift silently.
HTTP_STATUS = {
    "bad_request": 400,
    "failed_precondition": 400,
    "unauthenticated": 401,
    "forbidden": 403,
    "not_found": 404,
    "method_not_allowed": 405,
    "already_exists": 409,
    "conflict": 409,
    "payload_too_large": 413,
    "unsupported_media_type": 415,
    "unprocessable": 422,
    "rate_limited": 429,
    "quota_exceeded": 429,
    "cancelled": 499,
    "internal": 500,
    "unimplemented": 501,
    "unavailable": 503,
    "timeout": 504,
}

# N3: the properties a union's variants are told apart by.
DISCRIMINATORS = ("type", "kind")


class SyncError(Exception):
    """The document cannot be turned into spec/; the message says why."""


def fetch(source: str) -> bytes:
    if source.startswith(("https://", "http://")):
        if source.startswith("http://") and not source.startswith(
            ("http://127.0.0.1", "http://localhost", "http://[::1]")
        ):
            raise SyncError(f"refusing plain http for {source}; use https")
        # The edge refuses Python's default user agent; say who is asking.
        headers = {"accept": "application/json", "user-agent": USER_AGENT}
        request = urllib.request.Request(source, headers=headers)
        with urllib.request.urlopen(request, timeout=TIMEOUT_SECONDS) as response:
            body = response.read(MAX_BYTES + 1)
    else:
        body = Path(source).read_bytes()
    if len(body) > MAX_BYTES:
        raise SyncError(f"{source} is larger than {MAX_BYTES} bytes")
    return body


def refs(node: Any) -> set[str]:
    """Every schema name a node points at through `$ref`."""
    found: set[str] = set()
    if isinstance(node, dict):
        ref = node.get("$ref")
        if isinstance(ref, str) and ref.startswith(SCHEMAS):
            found.add(ref.removeprefix(SCHEMAS))
        for value in node.values():
            found |= refs(value)
    elif isinstance(node, list):
        for value in node:
            found |= refs(value)
    return found


def rewrite_refs(node: Any, names: dict[str, str], prefix: str = SCHEMAS) -> Any:
    if isinstance(node, dict):
        out = {}
        for key, value in node.items():
            if key == "$ref" and isinstance(value, str) and value.startswith(SCHEMAS):
                out[key] = prefix + names[value.removeprefix(SCHEMAS)]
            else:
                out[key] = rewrite_refs(value, names, prefix)
        return out
    if isinstance(node, list):
        return [rewrite_refs(value, names, prefix) for value in node]
    return node


def public_paths(doc: dict[str, Any]) -> dict[str, Any]:
    """The paths with only their `x-iohr-public` operations."""
    kept: dict[str, Any] = {}
    for path, item in doc.get("paths", {}).items():
        ops = {m: op for m, op in item.items() if m in METHODS and op.get("x-iohr-public") is True}
        if ops:
            shared = {k: v for k, v in item.items() if k not in METHODS}
            kept[path] = shared | ops
    if not kept:
        raise SyncError("the document marks no operation x-iohr-public")
    return kept


def reachable(schemas: dict[str, Any], roots: set[str]) -> set[str]:
    seen: set[str] = set()
    todo = list(roots)
    while todo:
        name = todo.pop()
        if name in seen:
            continue
        if name not in schemas:
            raise SyncError(f"$ref to a schema the document does not define: {name}")
        seen.add(name)
        todo.extend(refs(schemas[name]) - seen)
    return seen


def short_names(names: set[str]) -> dict[str, str]:
    """N1: `iohr.accounts.v1.GetMeResponse` becomes `GetMeResponse`.

    Two schemas that shorten to the same name are told apart by the proto package: the
    schema without a package keeps the short name, a proto schema that would clash takes
    its package as a prefix (`iohr.accounts.v1.Key` becomes `AccountsKey`). A clash that
    remains fails the sync.
    """
    by_short: dict[str, list[str]] = {}
    for name in names:
        by_short.setdefault(name.rsplit(".", 1)[-1], []).append(name)
    out: dict[str, str] = {}
    for short, group in by_short.items():
        if len(group) == 1:
            out[group[0]] = short
            continue
        for name in group:
            if "." not in name:
                out[name] = short
            else:
                package = name.split(".")[1] if name.count(".") >= 3 else name.split(".")[0]
                out[name] = package[:1].upper() + package[1:] + short
    taken: dict[str, str] = {}
    for original, new in sorted(out.items()):
        if new in taken:
            raise SyncError(f"N1: {original} and {taken[new]} both become {new}")
        taken[new] = original
    return out


def require_all(schema: dict[str, Any]) -> None:
    """N2: a transcoded message always carries every field, so every field is required."""
    if schema.get("type") == "object" and isinstance(schema.get("properties"), dict):
        schema["required"] = sorted(schema["properties"])


def discriminate(node: Any, schemas: dict[str, Any]) -> None:
    """N3: give a `oneOf` whose variants each fix `type` (or `kind`) a discriminator."""
    if isinstance(node, dict):
        variants = node.get("oneOf")
        if isinstance(variants, list) and "discriminator" not in node:
            resolved = [
                schemas.get(v["$ref"].removeprefix(SCHEMAS), {}) if "$ref" in v else v
                for v in variants
            ]
            for prop in DISCRIMINATORS:
                if all(fixes(v, prop) for v in resolved):
                    node["discriminator"] = {"propertyName": prop}
                    break
        for value in node.values():
            discriminate(value, schemas)
    elif isinstance(node, list):
        for value in node:
            discriminate(value, schemas)


def fixes(schema: dict[str, Any], prop: str) -> bool:
    """Whether every value of `schema` has `prop` set to one known string."""
    spec = schema.get("properties", {}).get(prop, {})
    single = "const" in spec or len(spec.get("enum", [])) == 1
    return single and prop in schema.get("required", [])


def problem_schema(schemas: dict[str, Any]) -> dict[str, Any]:
    """N6: the error envelope as a JSON Schema, with the code-to-status table."""
    for name in ("Problem", "Code", "Detail"):
        if name not in schemas:
            raise SyncError(f"N6: the document has no {name} schema")
    codes = schemas["Code"].get("enum", [])
    missing = sorted(set(codes) - HTTP_STATUS.keys())
    gone = sorted(HTTP_STATUS.keys() - set(codes))
    if missing or gone:
        raise SyncError(
            "N6: the status table no longer matches the codes "
            f"(new codes without a status: {missing or 'none'}; table codes the document "
            f"dropped: {gone or 'none'}); update HTTP_STATUS from the platform's Code::http"
        )

    defs: dict[str, Any] = {}
    names = {"Code": "Code", "Detail": "Detail"}
    code = copy.deepcopy(schemas["Code"])
    code["x-http-status"] = {c: HTTP_STATUS[c] for c in codes}
    defs["Code"] = code

    variants = []
    for variant in schemas["Detail"].get("oneOf", []):
        kind = variant.get("properties", {}).get("type", {})
        value = kind.get("const", (kind.get("enum") or [None])[0])
        if not isinstance(value, str):
            raise SyncError("N6: a Detail variant has no fixed `type`")
        name = value[:1].upper() + value[1:] + "Detail"
        defs[name] = copy.deepcopy(variant)
        variants.append({"$ref": f"#/$defs/{name}"})
    detail = {k: v for k, v in schemas["Detail"].items() if k != "oneOf"}
    detail["oneOf"] = variants
    detail["discriminator"] = {"propertyName": "type"}
    defs["Detail"] = detail

    problem = rewrite_refs(copy.deepcopy(schemas["Problem"]), names, "#/$defs/")
    return {
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "https://github.com/inorbithr/sdk/spec/problem.json",
        "title": "Problem",
        **problem,
        "$defs": rewrite_refs(defs, names, "#/$defs/"),
    }


def normalise(doc: dict[str, Any]) -> tuple[dict[str, Any], dict[str, Any]]:
    paths = public_paths(doc)
    all_schemas = doc.get("components", {}).get("schemas", {})
    kept = reachable(all_schemas, refs(paths))
    names = short_names(kept)

    schemas: dict[str, Any] = {}
    for original in sorted(kept):
        schema = copy.deepcopy(all_schemas[original])
        if "." in original:
            require_all(schema)
        schemas[names[original]] = rewrite_refs(schema, names)
    discriminate(schemas, schemas)

    out = {
        "openapi": doc["openapi"],
        "info": doc["info"],
        "servers": [SERVER],
        "paths": rewrite_refs(paths, names),
        "components": {"schemas": schemas},
    }
    components = doc.get("components", {})
    if "securitySchemes" in components:
        out["components"]["securitySchemes"] = components["securitySchemes"]
    if "security" in doc:
        out["security"] = doc["security"]
    return out, problem_schema(schemas)


def render(value: Any) -> str:
    return json.dumps(value, indent=2, sort_keys=True, ensure_ascii=False) + "\n"


def lab_files(source: str) -> tuple[dict[str, str], str]:
    """spec/lab/: the rules and the cases, checked for shape, and their joint sha256."""
    raw = {name: fetch(f"{source.rstrip('/')}/{name}") for name in ("rules.json", "conformance.json")}
    rules = json.loads(raw["rules.json"])
    cases = json.loads(raw["conformance.json"])
    if not isinstance(rules.get("rules"), list) or not rules["rules"]:
        raise SyncError("lab/rules.json has no rules")
    for rule in rules["rules"]:
        if not all(isinstance(rule.get(k), str) for k in ("id", "re", "why")):
            raise SyncError(f"lab/rules.json: a rule without id, re or why: {rule!r}")
        if "(?=" in rule["re"] or "(?!" in rule["re"] or "(?<" in rule["re"]:
            raise SyncError(f"lab/rules.json: rule {rule['id']} uses lookaround")
    if not isinstance(cases.get("files"), dict) or not isinstance(cases.get("config"), dict):
        raise SyncError("lab/conformance.json has no files or config")
    digest = hashlib.sha256(raw["rules.json"] + b"\n" + raw["conformance.json"]).hexdigest()
    return {"rules.json": render(rules), "conformance.json": render(cases)}, digest


def source_file(
    source: str, digest: str, doc: dict[str, Any], operations: int, lab: str, lab_digest: str
) -> str:
    """SOURCE, keeping the previous date when neither the document nor the lab changed."""
    synced = datetime.now(UTC).strftime("%Y-%m-%dT%H:%M:%SZ")
    previous = SPEC / "SOURCE"
    if previous.exists():
        old = dict(
            line.split(": ", 1) for line in previous.read_text().splitlines() if ": " in line
        )
        if old.get("sha256") == digest and old.get("lab_sha256") == lab_digest and "synced" in old:
            synced = old["synced"]
    return (
        f"source: {source}\n"
        f"sha256: {digest}\n"
        f"api_version: {doc['info'].get('version', '')}\n"
        f"openapi: {doc['openapi']}\n"
        f"operations: {operations}\n"
        f"synced: {synced}\n"
        "commit: the platform does not publish it in the document yet (spec/README.md)\n"
        f"lab_source: {lab}\n"
        f"lab_sha256: {lab_digest}\n"
    )


def sync_lab(args: argparse.Namespace) -> int:
    """spec/lab/ alone, and the two lab lines of SOURCE; the contract stays as it is."""
    try:
        lab, lab_digest = lab_files(args.lab)
    except (SyncError, OSError, json.JSONDecodeError) as err:
        print(f"spec:sync: {err}", file=sys.stderr)
        return 1
    previous = SPEC / "SOURCE"
    lines = [
        line
        for line in (previous.read_text().splitlines() if previous.exists() else [])
        if not line.startswith(("lab_source: ", "lab_sha256: "))
    ]
    lines += [f"lab_source: {args.lab}", f"lab_sha256: {lab_digest}"]
    files = {
        previous: "\n".join(lines) + "\n",
        **{SPEC / "lab" / name: text for name, text in lab.items()},
    }
    changed = [p for p, text in files.items() if not p.exists() or p.read_text() != text]
    if args.check:
        for path in changed:
            print(f"spec:sync: {path.relative_to(ROOT)} is out of date", file=sys.stderr)
        return 1 if changed else 0
    for path, text in files.items():
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)
    print(
        "spec:sync: lab only; changed: "
        + (", ".join(str(p.relative_to(ROOT)) for p in changed) or "nothing")
    )
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("source", nargs="?", default=DEFAULT_SOURCE, help="URL or file")
    parser.add_argument("--lab", default=DEFAULT_LAB, help="URL or folder of the lab files")
    parser.add_argument("--check", action="store_true", help="exit 1 if spec/ would change")
    parser.add_argument("--only", choices=["lab"], help="sync only this part of spec/")
    args = parser.parse_args()

    if args.only == "lab":
        return sync_lab(args)

    try:
        raw = fetch(args.source)
        doc = json.loads(raw)
        if not str(doc.get("openapi", "")).startswith("3.1"):
            raise SyncError(f"expected OpenAPI 3.1, got {doc.get('openapi')!r}")
        openapi, problem = normalise(doc)
        lab, lab_digest = lab_files(args.lab)
    except (SyncError, OSError, json.JSONDecodeError) as err:
        print(f"spec:sync: {err}", file=sys.stderr)
        return 1

    operations = sum(
        1 for item in openapi["paths"].values() for m in item if m in METHODS
    )
    files = {
        SPEC / "openapi.json": render(openapi),
        SPEC / "problem.json": render(problem),
        SPEC / "SOURCE": source_file(
            args.source, hashlib.sha256(raw).hexdigest(), doc, operations, args.lab, lab_digest
        ),
        **{SPEC / "lab" / name: text for name, text in lab.items()},
    }
    changed = [p for p, text in files.items() if not p.exists() or p.read_text() != text]
    if args.check:
        for path in changed:
            print(f"spec:sync: {path.relative_to(ROOT)} is out of date", file=sys.stderr)
        return 1 if changed else 0
    for path, text in files.items():
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)
    print(
        f"spec:sync: {operations} operations, {len(openapi['components']['schemas'])} schemas, "
        f"API {doc['info'].get('version', '?')}; changed: "
        + (", ".join(str(p.relative_to(ROOT)) for p in changed) or "nothing")
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
