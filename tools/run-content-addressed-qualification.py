#!/usr/bin/env python3
# Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Consume ASB's content-addressed qualification runner from the TUI boundary.

The ASB repository owns plan creation, local/mock execution, cassette capture,
strict replay, and comparison. This wrapper does not recreate those documents
or call their individual commands: it invokes the versioned ASB runner once,
validates its privacy-safe receipt, and records the paired TUI journey probe.
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
from typing import Any


_SECRET_ENV_MARKERS = (
    "API_KEY", "APIKEY", "ACCESS_TOKEN", "AUTH_TOKEN", "CREDENTIAL",
    "PASSWORD", "PRIVATE_KEY", "SECRET", "TOKEN",
)
_RUNNER = Path("tools/run-content-addressed-qualification.py")
_REQUIRED_UPSTREAM_KEYS = {
    "schema_version", "status", "network", "credentials", "source_revision",
    "paired_heads", "runner_sha256", "fixture_sha256", "provider_profile_sha256", "selection",
    "plan_run_journey", "campaign", "negative_identity", "negative_incomplete_coverage",
    "receipt_sha256",
}
_IDENTITY_NEGATIVES = {
    "stale_experiment": "experiment identity is invalid",
    "executable_mismatch": "agent executable digest does not match",
    "workload_mismatch": "unknown or invalid workload",
}
_INCOMPLETE_CASSETTE_REASON = "recording-coverage-incomplete"


def sanitized_environment() -> dict[str, str]:
    """Return a child environment with credential-bearing names removed."""

    environment = {
        name: value
        for name, value in os.environ.items()
        if not any(marker in name.upper() for marker in _SECRET_ENV_MARKERS)
    }
    environment.update({
        "ASB_TUI_NETWORK_POLICY": "deny",
        "HTTP_PROXY": "http://127.0.0.1:1",
        "HTTPS_PROXY": "http://127.0.0.1:1",
        "ALL_PROXY": "http://127.0.0.1:1",
        "NO_PROXY": "*",
    })
    return environment


def git(checkout: Path, ref: str = "HEAD") -> str:
    return subprocess.check_output(
        ["git", "-C", str(checkout), "rev-parse", ref], text=True
    ).strip()


def json_result(process: subprocess.CompletedProcess[str]) -> dict[str, Any]:
    for line in reversed((process.stdout + "\n" + process.stderr).splitlines()):
        try:
            value = json.loads(line)
        except json.JSONDecodeError:
            continue
        if isinstance(value, dict):
            return value
    raise AssertionError("no structured JSON result")


def negative_diagnostics(value: dict[str, Any]) -> dict[str, Any]:
    """Validate and compact ASB-owned typed negative qualification evidence.

    The TUI does not recreate an ASB plan or cassette.  It does, however,
    reject any upstream qualification that has not demonstrated each required
    identity and coverage failure.  The projected evidence retains only stable
    diagnostic classes and exit codes, not capture contents or paths.
    """

    identity = value.get("negative_identity")
    if not isinstance(identity, dict) or set(identity) != set(_IDENTITY_NEGATIVES):
        raise ValueError("ASB qualification has incomplete stale-identity diagnostics")
    cases: dict[str, dict[str, Any]] = {}
    for name, message in _IDENTITY_NEGATIVES.items():
        item = identity[name]
        if not isinstance(item, dict) or set(item) != {"rejected", "exit_code", "message"}:
            raise ValueError("ASB qualification has malformed stale-identity diagnostics")
        if item.get("rejected") is not True or item.get("exit_code") != 3 or item.get("message") != message:
            raise ValueError("ASB qualification did not retain a typed stale-identity rejection")
        cases[name] = {"rejected": True, "exit_code": 3}
    incomplete = value.get("negative_incomplete_coverage")
    if not isinstance(incomplete, dict) or set(incomplete) != {"rejected", "reason"}:
        raise ValueError("ASB qualification has malformed incomplete-cassette diagnostics")
    if incomplete.get("rejected") is not True or incomplete.get("reason") != _INCOMPLETE_CASSETTE_REASON:
        raise ValueError("ASB qualification did not retain the incomplete-cassette rejection")
    return {
        "stale_identity": {"rejected": True, "cases": cases},
        "incomplete_cassette": {"rejected": True, "reason": _INCOMPLETE_CASSETTE_REASON},
    }


def load_upstream_receipt(path: Path, asb_head: str, tui_head: str) -> dict[str, Any]:
    """Reject incomplete, mismatched, or non-private ASB qualification output."""

    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict) or set(value) != _REQUIRED_UPSTREAM_KEYS:
        raise ValueError("ASB qualification receipt has an unexpected shape")
    if value["status"] != "qualified" or value["network"] != "denied" or value["credentials"] != "none":
        raise ValueError("ASB qualification is not credential-free and offline")
    paired = value["paired_heads"]
    if not isinstance(paired, dict) or paired != {"asb": asb_head, "asb_tui": tui_head}:
        raise ValueError("ASB qualification receipt is not bound to the requested paired heads")
    journey, campaign = value["plan_run_journey"], value["campaign"]
    if not isinstance(journey, dict) or journey.get("comparison", {}).get("comparable") is not True:
        raise ValueError("ASB qualification did not produce a comparable plan/run journey")
    if not isinstance(campaign, dict) or campaign.get("comparison", {}).get("comparable") is not True:
        raise ValueError("ASB qualification did not produce comparable replay evidence")
    negative_diagnostics(value)
    return value


def probe_tui(binary: Path) -> dict[str, Any]:
    """Exercise the closed development journey without a broker or credentials."""

    process = subprocess.run(
        [str(binary), "journey", "--format", "json"],
        input=json.dumps({
            "schema_version": 1, "profile": "development", "asb_version": "fixture-1",
            "expected_asb_version": "fixture-1", "catalog_version": 7,
            "expected_catalog_version": 7, "bundle_available": True,
            "broker_available": True, "protocol_version": 1,
            "expected_protocol_version": 1,
        }) + "\n",
        env=sanitized_environment(), text=True, capture_output=True, check=False,
    )
    result = json_result(process)
    if process.returncode != 0 or result.get("ready") is not True:
        raise ValueError("TUI development journey probe did not become ready")
    return {"passed": True, "code": result.get("code")}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("tui_binary", type=Path)
    parser.add_argument("--asb-binary", type=Path, required=True)
    parser.add_argument("--asb-checkout", type=Path, required=True)
    parser.add_argument("--tui-checkout", type=Path, default=Path(__file__).parents[1])
    parser.add_argument("--receipt", type=Path, required=True)
    args = parser.parse_args()

    asb, tui = args.asb_binary.resolve(), args.tui_binary.resolve()
    asb_checkout, tui_checkout = args.asb_checkout.resolve(), args.tui_checkout.resolve()
    runner = asb_checkout / _RUNNER
    if not asb.is_file() or not tui.is_file() or not runner.is_file():
        parser.error("ASB binary, TUI binary, and ASB qualification runner must exist")
    asb_head, tui_head = git(asb_checkout), git(tui_checkout)
    with tempfile.TemporaryDirectory(prefix="asb-tui-ar1686-") as temporary:
        upstream_path = Path(temporary) / "asb-receipt.json"
        completed = subprocess.run(
            [sys.executable, str(runner), "--root", str(asb_checkout), "--asb", str(asb),
             "--tui-head", tui_head, "--receipt", str(upstream_path)],
            cwd=tui_checkout, env=sanitized_environment(), text=True, capture_output=True, check=False,
        )
        if completed.returncode != 0:
            raise SystemExit("ASB-owned content-addressed qualification failed")
        upstream = load_upstream_receipt(upstream_path, asb_head, tui_head)

    receipt = {
        "schema_version": 1, "ar": "AR-1686", "classification": "development/mock",
        "network": "denied", "credentials": "none",
        "provenance": {
            "asb_commit": asb_head, "asb_tree": git(asb_checkout, "HEAD^{tree}"),
            "tui_commit": tui_head, "tui_tree": git(tui_checkout, "HEAD^{tree}"),
            "asb_executable_sha256": hashlib.sha256(asb.read_bytes()).hexdigest(),
            "tui_executable_sha256": hashlib.sha256(tui.read_bytes()).hexdigest(),
        },
        "stages": {
            "asb_content_addressed_qualification": {
                "passed": True, "receipt_sha256": upstream["receipt_sha256"],
            },
            "negative_diagnostics": negative_diagnostics(upstream),
            "tui_journey": probe_tui(tui),
        },
        "complete": True,
    }
    args.receipt.parent.mkdir(parents=True, exist_ok=True)
    args.receipt.write_text(json.dumps(receipt, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(receipt, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
