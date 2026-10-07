#!/usr/bin/env python3
# Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Run the disposable current-main TUI quickstart qualification.

This is the AR-1622 operator entrypoint.  The established quickstart runner
owns the lifecycle and capture/replay assertions; this wrapper binds it to
the caller's exact ASB/TUI heads and emits an AR-1622 receipt.
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


def git(checkout: Path, revision: str = "HEAD") -> str:
    return subprocess.check_output(
        ["git", "-C", str(checkout), "rev-parse", revision], text=True
    ).strip()


def branch(checkout: Path, requested: str | None) -> str:
    if requested:
        return requested
    try:
        return subprocess.check_output(
            ["git", "-C", str(checkout), "symbolic-ref", "--short", "HEAD"],
            text=True,
        ).strip()
    except subprocess.CalledProcessError as error:
        raise SystemExit(
            "AR-1622 requires --tui-ref when the TUI checkout is detached"
        ) from error


def require_exact_ref(checkout: Path, reference: str, head: str) -> None:
    """Reject a materializer ref that cannot produce the tested checkout."""
    try:
        resolved = git(checkout, reference)
    except subprocess.CalledProcessError as error:
        raise SystemExit(
            f"AR-1622 materializer ref does not exist: {reference}"
        ) from error
    if resolved != head:
        raise SystemExit(
            "AR-1622 --tui-ref must resolve to the tested TUI checkout HEAD "
            f"({head}); {reference} resolves to {resolved}"
        )


def safe_environment() -> dict[str, str]:
    return {
        key: value
        for key, value in os.environ.items()
        if not any(marker in key.upper() for marker in SECRET_MARKERS)
    }


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Run the credential-free current-main ASB/TUI quickstart."
    )
    parser.add_argument("binary", type=Path, help="built asb-tui executable")
    parser.add_argument("--asb-binary", type=Path, required=True)
    parser.add_argument("--asb-checkout", type=Path, required=True)
    parser.add_argument(
        "--tui-checkout", type=Path,
        default=Path(__file__).resolve().parents[1],
        help="TUI checkout containing the exact tested head",
    )
    parser.add_argument(
        "--tui-ref",
        help="branch/ref to materialize (required for detached TUI checkouts)",
    )
    parser.add_argument("--receipt", type=Path, required=True)
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args()

    binary = args.binary.resolve()
    asb_binary = args.asb_binary.resolve()
    asb_checkout = args.asb_checkout.resolve()
    tui_checkout = args.tui_checkout.resolve()
    if not binary.is_file():
        raise SystemExit(f"TUI binary is missing: {binary}")
    if not asb_binary.is_file():
        raise SystemExit(f"ASB binary is missing: {asb_binary}")

    tui_head = git(tui_checkout)
    asb_head = git(asb_checkout)
    tui_ref = branch(tui_checkout, args.tui_ref)
    require_exact_ref(tui_checkout, tui_ref, tui_head)
    environment = safe_environment()
    environment.update(
        {
            "ASB_TUI_NETWORK_POLICY": "deny",
            "ASB_TUI_DEV_REPOSITORY": f"file://{tui_checkout}",
            "ASB_TUI_DEV_REF": tui_ref,
            "HTTP_PROXY": "http://127.0.0.1:1",
            "HTTPS_PROXY": "http://127.0.0.1:1",
            "ALL_PROXY": "http://127.0.0.1:1",
            "NO_PROXY": "*",
        }
    )

    runner = Path(__file__).with_name("run-quickstart-acceptance.py")
    with tempfile.TemporaryDirectory(prefix="asb-tui-ar1622-") as temporary:
        inner_receipt = Path(temporary) / "quickstart.json"
        command = [
            sys.executable, str(runner), str(binary),
            "--asb-binary", str(asb_binary),
            "--asb-checkout", str(asb_checkout),
            "--receipt", str(inner_receipt), "--json",
        ]
        completed = subprocess.run(
            command, cwd=tui_checkout, env=environment,
            text=True, capture_output=True, check=False,
        )
        if completed.returncode != 0:
            raise SystemExit(
                f"AR-1622 quickstart failed ({completed.returncode}):\n"
                f"{completed.stdout[-4000:]}{completed.stderr[-4000:]}"
            )
        if not inner_receipt.is_file():
            raise SystemExit("quickstart runner did not produce a receipt")
        evidence = json.loads(inner_receipt.read_text(encoding="utf-8"))

    provenance = evidence.get("provenance", {})
    if provenance.get("asb_commit") != asb_head:
        raise SystemExit("quickstart receipt is not bound to the requested ASB head")
    if provenance.get("tui_commit") != tui_head:
        raise SystemExit("quickstart receipt is not bound to the requested TUI head")
    receipt = dict(evidence)
    receipt["ar"] = "AR-1622"
    receipt["runner"] = {
        "name": "tools/run-current-main-quickstart.py",
        "sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
    }
    receipt["provenance"] = dict(provenance)
    receipt["provenance"].update({
        "asb_checkout_head": asb_head,
        "tui_checkout_head": tui_head,
        "tui_ref": tui_ref,
    })
    args.receipt.parent.mkdir(parents=True, exist_ok=True)
    args.receipt.write_text(json.dumps(receipt, indent=2) + "\n", encoding="utf-8")
    if args.json:
        print(json.dumps(receipt, sort_keys=True))
    else:
        print("AR-1622 current-main quickstart passed (development/mock; credentials absent; replay network denied)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
