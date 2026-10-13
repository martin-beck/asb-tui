#!/usr/bin/env python3
# Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Qualify the complete exact-head ASB/TUI fresh-user development route.

The standalone TUI owns the installed, keyboard-driven setup journey and the
ASB repository owns creation, execution, capture, strict replay, comparison,
and analysis of an experiment plan.  This runner composes those two existing
authoritative qualifications without reimplementing either side.  It rejects
any receipt that is not bound to the supplied exact checkout heads.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any


def git(checkout: Path, revision: str = "HEAD") -> str:
    return subprocess.check_output(
        ["git", "-C", str(checkout), "rev-parse", revision], text=True
    ).strip()


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def receipt(path: Path) -> dict[str, Any]:
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise ValueError(f"qualification receipt is not an object: {path.name}")
    return value


def invoke(command: list[str], checkout: Path, output: Path, name: str) -> dict[str, Any]:
    completed = subprocess.run(
        command, cwd=checkout, text=True, capture_output=True, check=False
    )
    if completed.returncode != 0:
        diagnostic = (completed.stderr or completed.stdout).strip()
        raise RuntimeError(f"{name} failed ({completed.returncode}): {diagnostic[-1200:]}")
    if not output.is_file():
        raise RuntimeError(f"{name} did not write its receipt")
    return receipt(output)


def validate_installed_tui(value: dict[str, Any], tui_head: str, asb_head: str) -> None:
    """Validate the TUI-owned install/wizard/capture/replay side of the route."""

    if value.get("ar") != "AR-1622" or value.get("credentials") != "none":
        raise ValueError("installed TUI receipt is not credential-free AR-1622 evidence")
    if value.get("network") != {
        "install_materialization": "allowed", "benchmark_and_replay": "denied"
    }:
        raise ValueError("installed TUI receipt did not deny benchmark and replay network")
    provenance = value.get("provenance")
    routes = value.get("routes")
    selection = value.get("selection")
    if not isinstance(provenance, dict) or not isinstance(routes, dict) or not isinstance(selection, dict):
        raise ValueError("installed TUI receipt is missing provenance, route, or selection evidence")
    if provenance.get("tui_commit") != tui_head or provenance.get("asb_commit") != asb_head:
        raise ValueError("installed TUI receipt is not bound to the requested paired heads")
    if not routes.get("status", {}).get("verified"):
        raise ValueError("installed TUI receipt did not verify the materialized development channel")
    expected = {
        "provider_selection": "provider_lifecycle_1656 passed",
        "unavailable_provider": "provider_lifecycle_1656 passed",
        "fanout_capture_replay": "coverage_setup_recording and development_journey passed",
        "end_to_end_test": "passed",
        "comparison": "end_to_end_qualification passed",
        "analysis": "end_to_end_qualification passed",
    }
    if {name: routes.get(name) for name in expected} != expected:
        raise ValueError("installed TUI receipt is missing a complete wizard-to-analysis route")
    if routes.get("tamper_negative", {}).get("code") not in {
        "dev_channel_manifest_invalid", "dev_channel_manifest_digest_mismatch"
    }:
        raise ValueError("installed TUI receipt did not retain a typed manifest-tamper rejection")
    if selection.get("replay") != "offline" or not selection.get("agent"):
        raise ValueError("installed TUI receipt has no bounded selected agent/offline replay")


def validate_asb_content(value: dict[str, Any], tui_head: str, asb_head: str) -> None:
    """Validate the ASB-owned plan/run/capture/replay/comparison side."""

    if value.get("ar") != "AR-1686" or value.get("complete") is not True:
        raise ValueError("ASB content qualification is incomplete")
    if value.get("network") != "denied" or value.get("credentials") != "none":
        raise ValueError("ASB content qualification is not offline and credential-free")
    provenance = value.get("provenance")
    stages = value.get("stages")
    if not isinstance(provenance, dict) or not isinstance(stages, dict):
        raise ValueError("ASB content qualification is missing provenance or stages")
    if provenance.get("tui_commit") != tui_head or provenance.get("asb_commit") != asb_head:
        raise ValueError("ASB content qualification is not bound to the requested paired heads")
    if stages.get("asb_content_addressed_qualification", {}).get("passed") is not True:
        raise ValueError("ASB plan/run/capture/replay qualification did not pass")
    if stages.get("tui_journey", {}).get("passed") is not True:
        raise ValueError("TUI closed development journey did not pass")
    negatives = stages.get("negative_diagnostics")
    expected_identity = {
        name: {"rejected": True, "exit_code": 3}
        for name in ("stale_experiment", "executable_mismatch", "workload_mismatch")
    }
    if not isinstance(negatives, dict) or negatives.get("stale_identity") != {
        "rejected": True, "cases": expected_identity
    } or negatives.get("incomplete_cassette") != {
        "rejected": True, "reason": "recording-coverage-incomplete"
    }:
        raise ValueError("ASB content qualification is missing typed negative diagnostics")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("tui_binary", type=Path, help="exact built asb-tui executable")
    parser.add_argument("--asb-binary", type=Path, required=True, help="exact built asb executable")
    parser.add_argument("--asb-checkout", type=Path, required=True)
    parser.add_argument("--tui-checkout", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--tui-ref", required=True, help="ref resolved by the disposable TUI materializer")
    parser.add_argument("--receipt", type=Path, required=True)
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args()

    tui_binary = args.tui_binary.resolve()
    asb_binary = args.asb_binary.resolve()
    tui_checkout = args.tui_checkout.resolve()
    asb_checkout = args.asb_checkout.resolve()
    if not tui_binary.is_file() or not asb_binary.is_file():
        parser.error("--asb-binary and tui_binary must identify existing executable files")
    tui_head, asb_head = git(tui_checkout), git(asb_checkout)
    if git(tui_checkout, args.tui_ref) != tui_head:
        parser.error("--tui-ref must resolve to the supplied TUI checkout HEAD")

    current_runner = Path(__file__).with_name("run-current-main-quickstart.py")
    content_runner = Path(__file__).with_name("run-content-addressed-qualification.py")
    with tempfile.TemporaryDirectory(prefix="asb-tui-ar1689-") as temporary:
        root = Path(temporary)
        installed = invoke(
            [
                sys.executable, str(current_runner), str(tui_binary),
                "--asb-binary", str(asb_binary), "--asb-checkout", str(asb_checkout),
                "--tui-checkout", str(tui_checkout), "--tui-ref", args.tui_ref,
                "--receipt", str(root / "installed-tui.json"), "--json",
            ],
            tui_checkout, root / "installed-tui.json", "installed TUI qualification",
        )
        content = invoke(
            [
                sys.executable, str(content_runner), str(tui_binary),
                "--asb-binary", str(asb_binary), "--asb-checkout", str(asb_checkout),
                "--tui-checkout", str(tui_checkout),
                "--receipt", str(root / "asb-content.json"),
            ],
            tui_checkout, root / "asb-content.json", "ASB content qualification",
        )
        validate_installed_tui(installed, tui_head, asb_head)
        validate_asb_content(content, tui_head, asb_head)
        installed_receipt_sha256 = sha256(root / "installed-tui.json")
        content_receipt_sha256 = sha256(root / "asb-content.json")

    output = {
        "schema_version": 1,
        "ar": "AR-1689",
        "status": "qualified",
        "classification": "development/mock",
        "credentials": "none",
        "network": {"install_materialization": "allowed", "benchmark_capture_replay": "denied"},
        "paired_heads": {"asb": asb_head, "asb_tui": tui_head},
        "selection_contract": {
            "wizard": installed["selection"],
            "wizard_to_plan": "end_to_end_qualification passed",
            "plan": "ASB content-addressed plan/run qualification passed",
        },
        "stages": {
            "installed_wizard": {
                "passed": True,
                "receipt_sha256": installed_receipt_sha256,
            },
            "content_addressed_plan_run": {
                "passed": True,
                "receipt_sha256": content_receipt_sha256,
            },
            "negative_diagnostics": {
                "installed_tamper": {"rejected": True, "code": installed["routes"]["tamper_negative"]["code"]},
                "unavailable_provider": {"rejected": True, "evidence": installed["routes"]["unavailable_provider"]},
                "stale_identity": content["stages"]["negative_diagnostics"]["stale_identity"],
                "incomplete_cassette": content["stages"]["negative_diagnostics"]["incomplete_cassette"],
                "network": "denied",
            },
        },
        "provenance": {
            "tui_tree": git(tui_checkout, "HEAD^{tree}"),
            "asb_tree": git(asb_checkout, "HEAD^{tree}"),
            "tui_executable_sha256": sha256(tui_binary),
            "asb_executable_sha256": sha256(asb_binary),
            "runner_sha256": sha256(Path(__file__)),
            "installed_tui_runner_sha256": installed["runner"]["sha256"],
            "asb_content_receipt_sha256": content_receipt_sha256,
        },
        "complete": True,
    }
    args.receipt.parent.mkdir(parents=True, exist_ok=True)
    args.receipt.write_text(json.dumps(output, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps(output, sort_keys=True) if args.json else "AR-1689 paired fresh-user qualification passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
