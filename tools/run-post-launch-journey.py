#!/usr/bin/env python3
# Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT

"""Run the complete post-launch ASB/TUI development journey.

This is an orchestration receipt, not a second implementation of either
qualification.  It composes the exact-main quickstart (setup, provider/model
selection, capture, replay, comparison, and analysis) with the channel matrix
(restart, rollback-preserving upgrade, and removal) and records both outputs
at the same paired heads.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import subprocess
import sys
from pathlib import Path


def git(checkout: Path, revision: str = "HEAD") -> str:
    return subprocess.check_output(
        ["git", "-C", str(checkout), "rev-parse", revision], text=True
    ).strip()


def branch(checkout: Path) -> str:
    try:
        return subprocess.check_output(
            ["git", "-C", str(checkout), "symbolic-ref", "--short", "HEAD"],
            text=True,
            stderr=subprocess.DEVNULL,
        ).strip()
    except subprocess.CalledProcessError:
        # Exact qualification commonly checks out a detached immutable head.
        # The materializer needs a branch/ref for its bounded clone; use a
        # validated local or origin main ref when available rather than
        # accidentally substituting a mutable working-tree identity.
        for ref in ("refs/heads/main", "refs/remotes/origin/main"):
            result = subprocess.run(
                ["git", "-C", str(checkout), "show-ref", "--verify", "--quiet", ref],
                check=False,
            )
            if result.returncode == 0:
                return "main"
        # Preserve an explicit actionable failure for repositories with no
        # usable branch ref; never pass the non-branch name HEAD to clone.
        raise SystemExit(
            "AR-1668 qualification requires a detached checkout with "
            "refs/heads/main or refs/remotes/origin/main"
        )


def run_json(command: list[str], *, cwd: Path, env: dict[str, str]) -> dict:
    process = subprocess.run(
        command, cwd=cwd, env=env, text=True, capture_output=True, check=False
    )
    output = process.stdout + "\n" + process.stderr
    for line in reversed(output.splitlines()):
        try:
            value = json.loads(line)
        except json.JSONDecodeError:
            continue
        if isinstance(value, dict):
            if process.returncode != 0:
                raise SystemExit(
                    f"qualification failed ({process.returncode}): {value}"
                )
            return value
    raise SystemExit(
        f"qualification emitted no JSON ({process.returncode}): {output[-4000:]}"
    )


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("binary", type=Path)
    parser.add_argument("--asb-binary", type=Path, required=True)
    parser.add_argument("--asb-checkout", type=Path, required=True)
    parser.add_argument("--receipt", type=Path, required=True)
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args()

    tui_checkout = Path(__file__).parents[1].resolve()
    asb_checkout = args.asb_checkout.resolve()
    binary = args.binary.resolve()
    asb_binary = args.asb_binary.resolve()
    asb_head = git(asb_checkout)
    tui_head = git(tui_checkout)
    tui_ref = branch(tui_checkout)
    env = {
        name: value
        for name, value in os.environ.items()
        if not any(
            marker in name.upper()
            for marker in ("API_KEY", "APIKEY", "ACCESS_TOKEN", "AUTH_TOKEN", "TOKEN", "SECRET", "PASSWORD")
        )
    }
    env.update(
        {
            "ASB_TUI_NETWORK_POLICY": "deny",
            "ASB_TUI_DEV_REPOSITORY": f"file://{tui_checkout}",
            # The materializer clones a branch/ref; bind that ref to the
            # checked-out branch while the receipt binds the immutable HEAD.
            "ASB_TUI_DEV_REF": tui_ref,
            "HTTP_PROXY": "http://127.0.0.1:1",
            "HTTPS_PROXY": "http://127.0.0.1:1",
            "ALL_PROXY": "http://127.0.0.1:1",
            "NO_PROXY": "*",
        }
    )
    common = [
        sys.executable,
        str(tui_checkout / "tools/run-quickstart-acceptance.py"),
        str(binary),
        "--asb-binary",
        str(asb_binary),
        "--asb-checkout",
        str(asb_checkout),
        "--receipt",
        str(args.receipt.with_name("ar1693-quickstart.json")),
        "--json",
    ]
    quickstart = run_json(common, cwd=tui_checkout, env=env)
    matrix = run_json(
        [
            sys.executable,
            str(tui_checkout / "tools/run-channel-compatibility-matrix.py"),
            str(binary),
            "--asb-checkout",
            str(asb_checkout),
            "--asb-head",
            asb_head,
            "--receipt",
            str(args.receipt.with_name("ar1676-channel-matrix.json")),
        ],
        cwd=tui_checkout,
        env=env,
    )
    receipt = {
        "schema_version": 1,
        "ar": "AR-1668",
        "classification": "development/mock",
        "network": {"install_materialization": "allowed", "journey": "denied"},
        "credentials": "none",
        "selection": {
            "channel": "dev",
            "provider": "fixture",
            "authentication": "development_fixture",
            "agents": ["opencode", "opendesk"],
            "models": "provider_catalog_compatible",
            "defaults": "shared",
            "replay": "offline",
        },
        "stages": {
            "setup_capture_replay_analysis": quickstart,
            "channel_restart_rollback_remove": matrix,
        },
        "provenance": {
            "asb_commit": asb_head,
            "asb_tree": git(asb_checkout, "HEAD^{tree}"),
            "tui_commit": tui_head,
            "tui_tree": git(tui_checkout, "HEAD^{tree}"),
            "asb_binary_sha256": hashlib.sha256(asb_binary.read_bytes()).hexdigest(),
            "tui_binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
        },
    }
    args.receipt.parent.mkdir(parents=True, exist_ok=True)
    args.receipt.write_text(json.dumps(receipt, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(receipt, sort_keys=True) if args.json else "AR-1668 post-launch journey passed (development/mock; replay network denied)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
