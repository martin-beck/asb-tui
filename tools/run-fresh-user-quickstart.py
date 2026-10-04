#!/usr/bin/env python3
# Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Build and run the disposable, selection-driven fresh-user quickstart.

The runner owns only orchestration.  The established acceptance runner remains
the authority for the wizard, ``asb tui install`` -> ``asb tui`` handoff,
capture, strict offline replay, comparison, and analysis assertions.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path


SECRET_MARKERS = (
    "API_KEY", "APIKEY", "ACCESS_TOKEN", "AUTH_TOKEN", "CREDENTIAL",
    "PASSWORD", "PRIVATE_KEY", "SECRET", "TOKEN",
)


def safe_environment() -> dict[str, str]:
    environment = {
        key: value
        for key, value in os.environ.items()
        if not any(marker in key.upper() for marker in SECRET_MARKERS)
    }
    environment.update({
        "ASB_TUI_NETWORK_POLICY": "deny",
        "HTTP_PROXY": "http://127.0.0.1:1",
        "HTTPS_PROXY": "http://127.0.0.1:1",
        "ALL_PROXY": "http://127.0.0.1:1",
        "NO_PROXY": "*",
    })
    return environment


def build(checkout: Path, package: str | None, binary: Path) -> None:
    command = ["cargo", "build", "--locked"]
    if package:
        command.extend(["--package", package])
    subprocess.run(command, cwd=checkout, env=safe_environment(), check=True)
    if not binary.is_file():
        raise SystemExit(f"build did not produce expected executable: {binary}")


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Build and run the credential-free fresh-user ASB/TUI quickstart."
    )
    parser.add_argument("--tui-checkout", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--asb-checkout", type=Path, required=True)
    parser.add_argument("--tui-binary", type=Path)
    parser.add_argument("--asb-binary", type=Path)
    parser.add_argument("--receipt", type=Path, required=True)
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args()

    tui_checkout = args.tui_checkout.resolve()
    asb_checkout = args.asb_checkout.resolve()
    tui_binary = (args.tui_binary or tui_checkout / "target/debug/asb-tui").resolve()
    asb_binary = (args.asb_binary or asb_checkout / "target/debug/asb").resolve()
    if not tui_binary.is_file():
        build(tui_checkout, "asb-tui", tui_binary)
    if not asb_binary.is_file():
        build(asb_checkout, None, asb_binary)

    acceptance = Path(__file__).with_name("run-current-main-quickstart.py")
    with tempfile.TemporaryDirectory(prefix="asb-tui-ar1643-") as temporary:
        inner = Path(temporary) / "quickstart.json"
        command = [
            sys.executable, str(acceptance), str(tui_binary),
            "--asb-binary", str(asb_binary), "--asb-checkout", str(asb_checkout),
            "--tui-checkout", str(tui_checkout), "--receipt", str(inner), "--json",
        ]
        completed = subprocess.run(
            command, cwd=tui_checkout, env=safe_environment(),
            text=True, capture_output=True, check=False,
        )
        if completed.returncode != 0:
            raise SystemExit(
                f"fresh-user quickstart failed ({completed.returncode}):\n"
                f"{completed.stdout[-4000:]}{completed.stderr[-4000:]}"
            )
        evidence = json.loads(inner.read_text(encoding="utf-8"))

    evidence["ar"] = "AR-1643"
    evidence["runner"] = {
        "name": "tools/run-fresh-user-quickstart.py",
        "sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
    }
    args.receipt.parent.mkdir(parents=True, exist_ok=True)
    args.receipt.write_text(json.dumps(evidence, indent=2) + "\n", encoding="utf-8")
    if args.json:
        print(json.dumps(evidence, sort_keys=True))
    else:
        print("AR-1643 fresh-user quickstart passed (development/mock; credentials absent; replay network denied)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
