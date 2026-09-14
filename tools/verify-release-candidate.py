#!/usr/bin/env python3
# Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Independently verify a staged asb-tui release bundle before installation."""

from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
import sys
from pathlib import Path

ARTIFACTS = {
    "asb-tui": "asb-tui",
    "source": "source.tar.gz",
    "licenses": "licenses.json",
    "sbom": "sbom.spdx.json",
    "provenance": "provenance.json",
}


def fail(message: str) -> "NoReturn":
    raise SystemExit(f"verification failed: {message}")


def digest(path: Path) -> str:
    hasher = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            hasher.update(chunk)
    return hasher.hexdigest()


def verify(bundle: Path, allowed_signers: Path, identity: str, namespace: str) -> dict[str, object]:
    manifest_path = bundle / "manifest.json"
    signature_path = bundle / "manifest.json.sig"
    if not manifest_path.is_file() or not signature_path.is_file():
        fail("manifest and detached signature are required")
    try:
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        fail(f"invalid manifest: {error}")
    if not isinstance(manifest, dict) or manifest.get("schema_version") != 1:
        fail("unsupported manifest schema")
    required = {"release", "source_commit", "source_tree", "artifacts", "components"}
    if not required.issubset(manifest):
        fail("manifest is missing required fields")
    if not isinstance(manifest["artifacts"], list) or len(manifest["artifacts"]) != 5:
        fail("manifest must contain exactly five artifacts")
    names = {entry.get("name") for entry in manifest["artifacts"] if isinstance(entry, dict)}
    if names != set(ARTIFACTS):
        fail("manifest artifact names are not the closed required set")
    try:
        subprocess.run(
            ["ssh-keygen", "-Y", "verify", "-f", str(allowed_signers), "-I", identity,
             "-n", namespace, "-s", str(signature_path)],
            input=manifest_path.read_bytes(), check=True, stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
    except (OSError, subprocess.CalledProcessError) as error:
        fail(f"manifest signature is invalid: {error}")
    for entry in manifest["artifacts"]:
        if not isinstance(entry, dict):
            fail("artifact entry is not an object")
        name = entry.get("name")
        path = bundle / ARTIFACTS.get(name, "")
        if not path.is_file():
            fail(f"missing artifact: {name}")
        actual_size = path.stat().st_size
        actual_digest = digest(path)
        if entry.get("size") != actual_size or entry.get("sha256") != actual_digest:
            fail(f"artifact digest or size mismatch: {name}")
    provenance = bundle / ARTIFACTS["provenance"]
    try:
        provenance_doc = json.loads(provenance.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        fail(f"invalid provenance: {error}")
    if provenance_doc.get("source_commit") != manifest["source_commit"] or provenance_doc.get("source_tree") != manifest["source_tree"]:
        fail("provenance identity does not match manifest")
    return manifest


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("bundle", type=Path)
    parser.add_argument("--allowed-signers", type=Path, required=True)
    parser.add_argument("--identity", required=True)
    parser.add_argument("--namespace", default="asb-tui-bundle-v1")
    args = parser.parse_args()
    manifest = verify(args.bundle.resolve(), args.allowed_signers.resolve(), args.identity, args.namespace)
    print(json.dumps({"ok": True, "release": manifest["release"], "source_commit": manifest["source_commit"]}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
