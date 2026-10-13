#!/usr/bin/env python3
"""Builds this extension's artifact (RFC 0028, iohr ADR 0012) as an OCI image layout:
one image index per version, one manifest per platform, each with the config
`application/vnd.inorbit.iohr.extension.config.v1+json` and one layer
`application/vnd.inorbit.iohr.extension.layer.v1.tar+gzip` holding the program.

    iohr ext lint --json > dist/lint.json
    python3 tools/ext_artifact.py --lint dist/lint.json --bin-dir dist/bin --out dist/oci

The config is the one `iohr ext lint --json` derives from extension.toml (its `config`
field), so what is signed is exactly what the lint passed. Programs are read from
<bin-dir>/<os>-<arch>/<entrypoint>. The output is deterministic: the same programs give
the same digests. Push with
`oras cp --from-oci-layout dist/oci:<version> <registry>/iohr-ext/<name>:<version>`.
Prints the index digest.
"""

import argparse
import gzip
import hashlib
import io
import json
import pathlib
import shutil
import sys
import tarfile

CONFIG_TYPE = "application/vnd.inorbit.iohr.extension.config.v1+json"
LAYER_TYPE = "application/vnd.inorbit.iohr.extension.layer.v1.tar+gzip"
MANIFEST_TYPE = "application/vnd.oci.image.manifest.v1+json"
INDEX_TYPE = "application/vnd.oci.image.index.v1+json"


def canonical(obj) -> bytes:
    return json.dumps(obj, sort_keys=True, separators=(",", ":")).encode()


def blob(out: pathlib.Path, data: bytes, media_type: str, **extra) -> dict:
    digest = hashlib.sha256(data).hexdigest()
    path = out / "blobs" / "sha256" / digest
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(data)
    return {"mediaType": media_type, "digest": f"sha256:{digest}", "size": len(data), **extra}


def layer(program: pathlib.Path, name: str) -> bytes:
    raw = io.BytesIO()
    with tarfile.open(fileobj=raw, mode="w", format=tarfile.PAX_FORMAT) as tar:
        info = tarfile.TarInfo(name)
        info.size = program.stat().st_size
        info.mode = 0o755
        info.mtime = 0
        info.uid = info.gid = 0
        info.uname = info.gname = ""
        with program.open("rb") as f:
            tar.addfile(info, f)
    gz = io.BytesIO()
    with gzip.GzipFile(fileobj=gz, mode="wb", mtime=0, compresslevel=9) as g:
        g.write(raw.getvalue())
    return gz.getvalue()


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--lint", required=True, help="the output of `iohr ext lint --json`")
    ap.add_argument("--bin-dir", default="dist/bin")
    ap.add_argument("--out", default="dist/oci")
    ap.add_argument("--source", default="", help="the repository URL, for the annotations")
    ap.add_argument("--revision", default="")
    args = ap.parse_args()

    report = json.loads(pathlib.Path(args.lint).read_text())
    if not report.get("ok"):
        print("the lint failed; fix extension.toml first", file=sys.stderr)
        return 1
    config = report["config"]
    version = config["version"]
    entrypoint = config["entrypoint"]
    platforms = (config.get("targets") or {}).get("platforms") or []
    out = pathlib.Path(args.out)
    if out.exists():
        shutil.rmtree(out)
    out.mkdir(parents=True)
    common = {"org.opencontainers.image.version": version, "org.opencontainers.image.created": "1970-01-01T00:00:00Z"}
    if args.source:
        common["org.opencontainers.image.source"] = args.source
    if args.revision:
        common["org.opencontainers.image.revision"] = args.revision

    manifests = []
    for platform in platforms:
        os_, arch = platform.split("/")
        program = pathlib.Path(args.bin_dir) / f"{os_}-{arch}" / entrypoint
        if not program.exists():
            print(f"{platform}: no {program}, skipped", file=sys.stderr)
            continue
        cfg = blob(out, canonical(config), CONFIG_TYPE)
        lay = blob(out, layer(program, entrypoint), LAYER_TYPE,
                   annotations={"org.opencontainers.image.title": entrypoint})
        manifest = {"schemaVersion": 2, "mediaType": MANIFEST_TYPE, "config": cfg, "layers": [lay], "annotations": common}
        manifests.append(blob(out, canonical(manifest), MANIFEST_TYPE, platform={"os": os_, "architecture": arch}))
        print(f"{platform}: {manifests[-1]['digest']}", file=sys.stderr)
    if not manifests:
        print(f"no programs under {args.bin_dir}/<os>-<arch>/", file=sys.stderr)
        return 1
    index = {"schemaVersion": 2, "mediaType": INDEX_TYPE, "manifests": manifests, "annotations": common}
    idx = blob(out, canonical(index), INDEX_TYPE, annotations={"org.opencontainers.image.ref.name": version})
    (out / "oci-layout").write_text(json.dumps({"imageLayoutVersion": "1.0.0"}))
    (out / "index.json").write_text(json.dumps({"schemaVersion": 2, "mediaType": INDEX_TYPE, "manifests": [idx]}, indent=2))
    print(idx["digest"])
    return 0


if __name__ == "__main__":
    sys.exit(main())
