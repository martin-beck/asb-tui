#!/usr/bin/env python3
# Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Verify an immutable development bundle before ASB consumes it."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path


def digest(path: Path) -> str:
    hasher = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            hasher.update(chunk)
    return hasher.hexdigest()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("bundle", type=Path)
    args = parser.parse_args()
    manifest = json.loads((args.bundle / "manifest.json").read_text(encoding="utf-8"))
    active = json.loads((args.bundle.parent.parent / "active-dev.json").read_text(encoding="utf-8"))
    required = {"schema_version", "channel", "development_only", "source_repository",
                "source_ref", "source_commit", "source_tree", "asb_source_commit",
                "asb_source_tree", "target", "executable_sha256", "executable_size",
                "built_unix", "warnings"}
    if set(manifest) != required or manifest != active:
        raise SystemExit("development manifest is incomplete or active pointer differs")
    if manifest["schema_version"] != 1 or manifest["channel"] != "dev" or not manifest["development_only"]:
        raise SystemExit("not a development bundle")
    executable = args.bundle / "asb-tui"
    if not executable.is_file() or executable.stat().st_size != manifest["executable_size"]:
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
