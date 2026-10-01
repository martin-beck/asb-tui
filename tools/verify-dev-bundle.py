#!/usr/bin/env python3
# Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Verify an immutable development bundle before ASB consumes it."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import stat
import time
from pathlib import Path

REPOSITORY = "https://github.com/martin-beck/asb-tui.git"
TARGETS = {"x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu"}
MAX_EXECUTABLE_BYTES = 256 * 1024 * 1024


def digest(path: Path) -> str:
    hasher = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            hasher.update(chunk)
    return hasher.hexdigest()


def regular(path: Path) -> bool:
    try:
        return stat.S_ISREG(path.stat(follow_symlinks=False).st_mode)
    except (FileNotFoundError, OSError):
        return False


def hex_identity(value: object, length: int) -> bool:
    return isinstance(value, str) and len(value) == length and all(
        character in "0123456789abcdef" for character in value
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("bundle", type=Path)
    args = parser.parse_args()
    if args.bundle.is_symlink() or not args.bundle.is_dir():
        raise SystemExit("development bundle directory is not regular")
    manifest_path = args.bundle / "manifest.json"
    active_path = args.bundle.parent.parent / "active-dev.json"
    if not regular(manifest_path) or not regular(active_path):
        raise SystemExit("development manifest is not a regular file")
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    active = json.loads(active_path.read_text(encoding="utf-8"))
    required = {"schema_version", "channel", "development_only", "source_repository",
                "source_ref", "source_commit", "source_tree", "asb_source_commit",
                "asb_source_tree", "target", "executable_sha256", "executable_size",
                "built_unix", "warnings"}
    if set(manifest) != required or manifest != active:
        raise SystemExit("development manifest is incomplete or active pointer differs")
    if (manifest["schema_version"] != 1 or manifest["channel"] != "dev"
            or manifest["development_only"] is not True
            or manifest["source_repository"] != REPOSITORY
            or manifest["source_ref"] != "refs/heads/main"
            or not hex_identity(manifest["source_commit"], 40)
            or not hex_identity(manifest["source_tree"], 40)
            or not hex_identity(manifest["asb_source_commit"], 40)
            or not hex_identity(manifest["asb_source_tree"], 40)
            or manifest["target"] not in TARGETS
            or not hex_identity(manifest["executable_sha256"], 64)
            or not isinstance(manifest["executable_size"], int)
            or manifest["executable_size"] <= 0
            or manifest["executable_size"] > MAX_EXECUTABLE_BYTES
            or not isinstance(manifest["built_unix"], int)
            or manifest["built_unix"] <= 0
            or manifest["built_unix"] > int(time.time()) + 300
            or manifest["warnings"] != [
                "development_missing_authentication_allowed",
                "development_missing_signatures_allowed",
                "development_missing_key_management_allowed",
            ]):
        raise SystemExit("not a development bundle")
    if args.bundle.name != manifest["executable_sha256"]:
        raise SystemExit("development bundle directory is not content-addressed")
    executable = args.bundle / "asb-tui"
    if not regular(executable) or executable.stat().st_size != manifest["executable_size"]:
        raise SystemExit("development executable is missing or has unexpected size")
    if digest(executable) != manifest["executable_sha256"]:
        raise SystemExit("development executable digest mismatch")
    if not all(isinstance(item, str) and item.startswith("development_")
               for item in manifest["warnings"]):
        raise SystemExit("development warnings are not explicit")
    print(json.dumps({"ok": True, "development_only": True,
                      "source_commit": manifest["source_commit"],
                      "executable_sha256": manifest["executable_sha256"]}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
