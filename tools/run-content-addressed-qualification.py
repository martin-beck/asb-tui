#!/usr/bin/env python3
# Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Run the paired ASB/TUI content-addressed qualification boundary.

This runner deliberately reports incomplete downstream stages instead of
turning an unavailable provider selection or missing cassette into a pass.
It is a disposable development check: no credentials are read and no
provider is contacted.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import subprocess
import tempfile
from pathlib import Path


def git(checkout: Path, ref: str = "HEAD") -> str:
    return subprocess.check_output(
        ["git", "-C", str(checkout), "rev-parse", ref], text=True
    ).strip()


def json_result(process: subprocess.CompletedProcess[str]) -> dict:
    for line in reversed((process.stdout + "\n" + process.stderr).splitlines()):
        try:
            value = json.loads(line)
        except json.JSONDecodeError:
            continue
        if isinstance(value, dict):
            return value
    raise AssertionError(f"no JSON result: {process.stdout}{process.stderr}")


def run(binary: Path, *args: str, expected: tuple[int, ...] = (0,)) -> dict:
    process = subprocess.run(
        [str(binary), "--json", *args],
        text=True,
        capture_output=True,
        check=False,
    )
    if process.returncode not in expected:
        raise AssertionError(
            f"{binary.name} {' '.join(args)} exited {process.returncode}: "
            f"{process.stdout}{process.stderr}"
        )
    return json_result(process)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("tui_binary", type=Path)
    parser.add_argument("--asb-binary", type=Path, required=True)
    parser.add_argument("--asb-checkout", type=Path, required=True)
    parser.add_argument("--tui-checkout", type=Path, default=Path(__file__).parents[1])
    parser.add_argument("--receipt", type=Path, required=True)
    args = parser.parse_args()

    asb = args.asb_binary.resolve()
    tui = args.tui_binary.resolve()
    asb_checkout = args.asb_checkout.resolve()
    tui_checkout = args.tui_checkout.resolve()
    receipt: dict = {
        "schema_version": 1,
        "ar": "AR-1686",
        "classification": "development/mock",
        "network": "denied",
        "credentials": "none",
        "provenance": {
            "asb_commit": git(asb_checkout),
            "asb_tree": git(asb_checkout, "HEAD^{tree}"),
            "tui_commit": git(tui_checkout),
            "tui_tree": git(tui_checkout, "HEAD^{tree}"),
            "asb_executable_sha256": hashlib.sha256(asb.read_bytes()).hexdigest(),
            "tui_executable_sha256": hashlib.sha256(tui.read_bytes()).hexdigest(),
        },
        "stages": {},
    }

    with tempfile.TemporaryDirectory(prefix="asb-tui-ar1686-") as directory:
        root = Path(directory)
        agent = root / "agent"
        agent.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
        agent.chmod(0o700)
        plan_a = root / "plan-a.toml"
        plan_b = root / "plan-b.toml"
        create_a = run(
            asb,
            "plan",
            "create",
            "--workload",
            "original.bug-fix",
            "--agent-executable",
            str(agent),
            "--output",
            str(plan_a),
            "--run-id",
            "ar1686-a",
            "--result-root",
            str(root / "results-a"),
            "--work-root",
            str(root / "work-a"),
        )
        validate_a = run(asb, "plan", str(plan_a))
        run_a = run(asb, "run", str(plan_a), "--local-mock")
        receipt["stages"]["plan_create"] = {
            "passed": create_a.get("ok") is True and validate_a.get("ok") is True,
            "experiment_sha256": create_a.get("experiment_sha256"),
            "plan_schema_version": validate_a.get("plan_schema_version"),
        }
        receipt["stages"]["run"] = {
            "passed": run_a.get("ok") is True,
            "run_ids": run_a.get("run_ids", []),
        }

        create_b = run(
            asb,
            "plan",
            "create",
            "--workload",
            "original.bug-fix",
            "--agent-executable",
            str(agent),
            "--output",
            str(plan_b),
            "--run-id",
            "ar1686-b",
            "--result-root",
            str(root / "results-b"),
            "--work-root",
            str(root / "work-b"),
        )
        run_b = run(asb, "run", str(plan_b), "--local-mock")
        report_a = run(asb, "report", str(root / "results-a" / "runs" / "ar1686-a"))
        report_b = run(asb, "report", str(root / "results-b" / "runs" / "ar1686-b"))
        comparison = run(
            asb,
            "compare",
            str(root / "results-a" / "runs" / "ar1686-a"),
            str(root / "results-b" / "runs" / "ar1686-b"),
        )
        receipt["stages"]["comparison"] = {
            "passed": comparison.get("comparable") is True,
            "comparable": comparison.get("comparable"),
            "unavailable_reasons": comparison.get("unavailable_reasons", []),
            "note": "ASB-generated local/mock plans currently lack provider selection",
        }
        receipt["stages"]["reports"] = {
            "passed": report_a.get("ok") is True and report_b.get("ok") is True,
            "runs": [run_a.get("run_ids"), run_b.get("run_ids")],
            "second_plan_experiment_sha256": create_b.get("experiment_sha256"),
        }

        agent.write_text("#!/bin/sh\necho changed\nexit 0\n", encoding="utf-8")
        stale = subprocess.run(
            [str(asb), "--json", "run", str(plan_a), "--local-mock"],
            text=True,
            capture_output=True,
            check=False,
        )
        stale_result = json_result(stale)
        receipt["stages"]["stale_identity"] = {
            "passed": stale.returncode == 3
            and stale_result.get("error", {}).get("message")
            == "agent executable digest does not match",
            "exit_code": stale.returncode,
            "error": stale_result.get("error", {}).get("message"),
        }

        probe_env = {
            **os.environ,
            "ASB_TUI_NETWORK_POLICY": "deny",
            "HTTP_PROXY": "http://127.0.0.1:1",
            "HTTPS_PROXY": "http://127.0.0.1:1",
            "ALL_PROXY": "http://127.0.0.1:1",
            "NO_PROXY": "*",
        }
        tui_probe = subprocess.run(
            [str(tui), "journey", "--format", "json"],
            input=json.dumps(
                {
                    "schema_version": 1,
                    "profile": "development",
                    "asb_version": "fixture-1",
                    "expected_asb_version": "fixture-1",
                    "catalog_version": 7,
                    "expected_catalog_version": 7,
                    "bundle_available": True,
                    "broker_available": True,
                    "protocol_version": 1,
                    "expected_protocol_version": 1,
                }
            )
            + "\n",
            env=probe_env,
            text=True,
            capture_output=True,
            check=False,
        )
        tui_result = json_result(tui_probe)
        receipt["stages"]["tui_journey"] = {
            "passed": tui_probe.returncode == 0 and tui_result.get("ready") is True,
            "code": tui_result.get("code"),
        }

    receipt["stages"]["capture_replay"] = {
        "passed": False,
        "status": "not_run",
        "note": "requires a runner-owned capture/cassette authority; no cassette is fabricated",
    }
    receipt["complete"] = all(
        stage.get("passed") is True for stage in receipt["stages"].values()
    )
    args.receipt.parent.mkdir(parents=True, exist_ok=True)
    args.receipt.write_text(json.dumps(receipt, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(receipt, sort_keys=True))
    return 0 if receipt["complete"] else 2


if __name__ == "__main__":
    raise SystemExit(main())
