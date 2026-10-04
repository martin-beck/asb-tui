#!/usr/bin/env python3
# Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Run the deterministic AR-1657 agent/provider/model matrix qualification."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import subprocess
from pathlib import Path

EXPECTED_TUPLES = [
    {"agent": "opencode", "provider": "openai", "model": "gpt-4o"},
    {"agent": "opencode", "provider": "openai", "model": "fixture-model"},
    {"agent": "opencode", "provider": "openrouter", "model": "openai/gpt-4o"},
    {"agent": "opendesk", "provider": "openai", "model": "gpt-4o"},
    {"agent": "opendesk", "provider": "openai", "model": "fixture-model"},
    {"agent": "opendesk", "provider": "local", "model": "fixture-model"},
]
REQUIRED_TESTS = (
    "compatibility_matrix_supports_defaults_overrides_restart_and_offline_validation",
    "matrix_evaluates_both_adapters_and_keeps_unavailable_reason_typed",
    "matrix_covers_every_development_tuple_and_opendesk_defaults_offline",
    "connected_catalog_projects_both_adapter_routes",
    "connected_catalog_keeps_unavailable_provider_and_model_visible_but_unselectable",
    "every_guided_route_has_human_and_json_output",
    "malformed_guided_options_are_rejected_without_payload",
)


def git(checkout: Path, revision: str = "HEAD") -> str:
    return subprocess.check_output(["git", "-C", str(checkout), "rev-parse", revision], text=True).strip()


def sanitized_environment() -> dict[str, str]:
    markers = ("API_KEY", "APIKEY", "ACCESS_TOKEN", "AUTH_TOKEN", "CREDENTIAL", "PASSWORD", "SECRET", "TOKEN")
    return {key: value for key, value in os.environ.items() if not any(marker in key.upper() for marker in markers)}


def main() -> int:
    parser = argparse.ArgumentParser(description="Run the offline AR-1657 compatibility matrix.")
    parser.add_argument("--checkout", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--receipt", type=Path, required=True)
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args()
    checkout = args.checkout.resolve()
    environment = sanitized_environment()
    environment["ASB_TUI_NETWORK_POLICY"] = "deny"
    result = subprocess.run(
        ["cargo", "test", "--locked", "--offline", "--test", "provider_lifecycle_1656", "--test", "guided_commands", "--", "--nocapture"],
        cwd=checkout, env=environment, text=True, capture_output=True, check=False,
    )
    output = result.stdout + "\n" + result.stderr
    if result.returncode != 0:
        raise SystemExit(f"AR-1657 matrix failed ({result.returncode}):\n{output[-6000:]}")
    missing = [name for name in REQUIRED_TESTS if name not in output]
    if missing:
        raise SystemExit(f"matrix test output omitted required cases: {missing}")
    receipt = {
        "schema_version": 1, "ar": "AR-1657", "classification": "development/mock",
        "network": "denied", "credentials": "none",
        "matrix": {
            "tuple_count": len(EXPECTED_TUPLES), "tuples": EXPECTED_TUPLES,
            "all_supported": True,
            "unavailable_reason": "development authentication unavailable",
            "defaults": "shared provider/model default is retained",
            "overrides": "per-agent override wins before restart",
            "restart": "restart restores shared defaults and clears transient overrides",
            "offline": "every development tuple validates for offline replay",
        },
        "routes": {
            "human": "guided_commands every route", "json": "guided_commands every route",
            "positive": "provider_lifecycle_1656 passed",
            "negative": "typed unavailable provider/model remains visible and unselectable",
        },
        "provenance": {
            "tui_commit": git(checkout), "tui_tree": git(checkout, "HEAD^{tree}"),
            "runner_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
        },
    }
    args.receipt.parent.mkdir(parents=True, exist_ok=True)
    args.receipt.write_text(json.dumps(receipt, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(receipt, sort_keys=True) if args.json else "AR-1657 agent/provider/model matrix passed (development/mock; network denied)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
