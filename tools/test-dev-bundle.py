#!/usr/bin/env python3
# Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Exercise development-bundle integrity and pointer checks."""

from __future__ import annotations

import hashlib
import json
import subprocess
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
VERIFY = ROOT / "tools" / "verify-dev-bundle.py"


def main() -> int:
    with tempfile.TemporaryDirectory(prefix="asb-tui-dev-bundle-test-") as temporary:
        root = Path(temporary)
        version = root / "dev-versions" / ("a" * 64)
        version.mkdir(parents=True)
        executable = version / "asb-tui"
        executable.write_bytes(b"development fixture")
        digest = hashlib.sha256(executable.read_bytes()).hexdigest()
        version = root / "dev-versions" / digest
        version.mkdir(parents=True)
        executable = version / "asb-tui"
        executable.write_bytes(b"development fixture")
        manifest = {
            "schema_version": 1, "channel": "dev", "development_only": True,
            "source_repository": "https://github.com/martin-beck/asb-tui.git",
            "source_ref": "refs/heads/main", "source_commit": "a" * 40,
            "source_tree": "b" * 40, "asb_source_commit": "c" * 40,
            "asb_source_tree": "d" * 40, "target": "x86_64-unknown-linux-gnu",
            "executable_sha256": digest, "executable_size": executable.stat().st_size,
            "built_unix": 1, "warnings": ["development_missing_authentication_allowed"],
        }
        encoded = (json.dumps(manifest, indent=2, sort_keys=True) + "\n").encode()
        (version / "manifest.json").write_bytes(encoded)
        (root / "active-dev.json").write_bytes(encoded)
        subprocess.run(["python3", str(VERIFY), str(version)], check=True,
                       stdout=subprocess.PIPE, text=True)
        executable.write_bytes(b"tampered")
        rejected = subprocess.run(["python3", str(VERIFY), str(version)], check=False,
                                  stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        if rejected.returncode == 0:
            raise SystemExit("tampered development executable was accepted")
    print("development bundle integrity checks passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
