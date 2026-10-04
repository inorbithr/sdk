# /// script
# requires-python = ">=3.12"
# dependencies = []
# ///
"""Fetch the platform's public OpenAPI document and write spec/.

Run through `mise run spec:sync` (or `uv run --script tools/spec-sync.py [SOURCE]`).
SOURCE is a URL or a local file; the default is the API's public document.

Steps (spec/README.md): keep the operations marked `x-iohr-public`, keep the schemas
they reach, apply rule N1 (N5 is settled behaviour, not a rewrite), check the facts the platform
states itself since core #218 (the former rules N2, N3, N4, N6), write spec/openapi.json,
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
SERVER_URL = "https://api.inorbit.hr"
SCHEMAS = "#/components/schemas/"
METHODS = ("get", "put", "post", "delete", "options", "head", "patch", "trace")
TIMEOUT_SECONDS = 30
MAX_BYTES = 8 * 1024 * 1024
USER_AGENT = "inorbithr-sdk-spec-sync (+https://github.com/inorbithr/sdk)"

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


def response_only(paths: dict[str, Any], schemas: dict[str, Any]) -> set[str]:
    """The schemas only an answer reaches: no request body or parameter leads to them."""
    asked: set[str] = set()
    answered: set[str] = set()
    for item in paths.values():
        for method, op in item.items():
            if method not in METHODS:
                continue
            asked |= refs(op.get("requestBody", {})) | refs(op.get("parameters", []))
            answered |= refs(op.get("responses", {}))
        asked |= refs(item.get("parameters", []))
    return reachable(schemas, answered) - reachable(schemas, asked)


def check_required(paths: dict[str, Any], schemas: dict[str, Any]) -> None:
    """Former N2, fixed upstream (core #218): a message only an answer carries lists its
    fields without presence as `required`; a request-side message marks nothing.

    The sync fails when an answer's message has fields the platform always sends but
    the document marks none (`required` gone from every answer), or names a field the
    schema does not have, so a regression is caught here instead of patched over.
    """
    answers = [
        n for n in sorted(response_only(paths, schemas))
        if "." in n and not n.startswith("google.") and schemas[n].get("type") == "object"
    ]
    for name in answers:
        listed = schemas[name].get("required", [])
        unknown = sorted(set(listed) - set(schemas[name].get("properties", {})))
        if unknown:
            raise SyncError(f"N2: {name} requires fields it does not define: {unknown}")
    with_scalars = [
        n for n in answers
        if any("$ref" not in p for p in schemas[n].get("properties", {}).values())
    ]
    if with_scalars and not any(schemas[n].get("required") for n in answers):
        raise SyncError(
            "N2: no answer's message marks a field required; the platform states which "
            "fields are always sent since core #218, so the document regressed upstream"
        )


def check_server(doc: dict[str, Any]) -> None:
    """Former N4, fixed upstream (core #218): the document names its server by an absolute
    https URL (`https://api.inorbit.hr` in production), not `/`."""
    servers = doc.get("servers") or [{}]
    url = servers[0].get("url")
    if not isinstance(url, str) or not url.startswith("https://") or len(url) <= len("https://"):
        raise SyncError(
            f"N4: servers[0].url is {url!r}, not an absolute https URL such as {SERVER_URL}; "
            "the document regressed upstream (core #218)"
        )


def check_envelope(schemas: dict[str, Any]) -> None:
    """Former N3 and N6, fixed upstream (core #218): `Detail` names its discriminator and
    `Code` carries `x-http-status`, a status for every code and no other."""
    for name in ("Problem", "Code", "Detail"):
        if name not in schemas:
            raise SyncError(f"N6: the document has no {name} schema")
    if schemas["Detail"].get("discriminator", {}).get("propertyName") != "type":
        raise SyncError(
            "N3: Detail has no discriminator on `type`; the document regressed upstream "
            "(core #218)"
        )
    codes = schemas["Code"].get("enum", [])
    table = schemas["Code"].get("x-http-status")
    if not isinstance(table, dict):
        raise SyncError(
            "N6: Code has no x-http-status; the document regressed upstream (core #218)"
        )
    missing = sorted(set(codes) - table.keys())
    extra = sorted(table.keys() - set(codes))
    bad = sorted(
        c for c, s in table.items()
        if isinstance(s, bool) or not isinstance(s, int) or not 100 <= s <= 599
    )
    if missing or extra or bad:
        raise SyncError(
            "N6: Code's x-http-status does not match its codes "
            f"(codes without a status: {missing or 'none'}; statuses for no code: "
            f"{extra or 'none'}; not an HTTP status: {bad or 'none'})"
        )


def problem_schema(schemas: dict[str, Any]) -> dict[str, Any]:
    """The error envelope as a JSON Schema: `Problem`, with `Code` (and its
    `x-http-status`) and each `Detail` variant as its own definition."""
    defs: dict[str, Any] = {}
    names = {"Code": "Code", "Detail": "Detail"}
    defs["Code"] = copy.deepcopy(schemas["Code"])

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
    check_server(doc)
    check_required(paths, all_schemas)
    names = short_names(kept)

    schemas: dict[str, Any] = {}
    for original in sorted(kept):
        schemas[names[original]] = rewrite_refs(copy.deepcopy(all_schemas[original]), names)
    check_envelope(schemas)

    out = {
        "openapi": doc["openapi"],
        "info": doc["info"],
        "servers": doc["servers"],
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
