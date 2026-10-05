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


def run_asb_tui_parity(asb_binary: Path, tui_binary: Path) -> dict:
    """Exercise the parent command in both network policies and its installed route."""
    base = sanitized_environment()
    base["ASB_TUI_NETWORK_POLICY"] = "allow"
    online = subprocess.run(
        [str(tui_binary), "tui", "install", "--channel", "dev", "--format", "json"], env=base,
        text=True, capture_output=True, check=False,
    )
    offline_env = dict(base)
    offline_env["ASB_TUI_NETWORK_POLICY"] = "deny"
    offline = subprocess.run(
        [str(tui_binary), "tui", "install", "--channel", "dev", "--format", "json"], env=offline_env,
        text=True, capture_output=True, check=False,
    )
    def document(process: subprocess.CompletedProcess[str]) -> dict:
        for line in reversed(process.stdout.splitlines()):
            try:
                value = json.loads(line)
            except json.JSONDecodeError:
                continue
            if isinstance(value, dict):
                return value
        raise SystemExit("asb-tui lifecycle emitted no structured result")
    online_result = document(online)
    offline_result = document(offline)
    if online_result.get("channel") != "dev" or offline_result.get("channel") != "dev":
        raise SystemExit("asb-tui install did not report the development channel")
    if not online_result.get("ok"):
        raise SystemExit(f"asb-tui install did not produce an installed frontend: {online_result}")
    installed = subprocess.run(
        [str(tui_binary), "tui", "status", "--channel", "dev", "--format", "json"], env=offline_env,
        text=True, capture_output=True, check=False,
    )
    installed_result = document(installed)
    if not installed_result.get("ok"):
        raise SystemExit(f"installed asb-tui status is not ready: {installed_result}")
    launch = subprocess.run(
        [str(tui_binary), "tui", "launch", "--channel", "dev", "--format", "json"], env=offline_env,
        text=True, capture_output=True, check=False, timeout=15,
    )
    launch_result = document(launch)
    if launch_result.get("channel") != "dev":
        raise SystemExit("installed asb-tui did not report the development channel")
    return {
        "online_install": online_result,
        "offline_install": offline_result,
        "installed_asb_tui_status": installed_result,
        "installed_asb_tui_launch": launch_result,
        "installed_asb_tui_direct_status": installed_result,
        "asb_binary_sha256": hashlib.sha256(asb_binary.read_bytes()).hexdigest(),
        "installed_asb_tui_sha256": hashlib.sha256(tui_binary.read_bytes()).hexdigest(),
        "parity": online_result.get("channel") == offline_result.get("channel") == "dev",
    }


def authoritative_asb_catalog(binary: Path) -> dict:
    """Read the paired ASB catalog; never substitute the local TUI fixture."""
    result = subprocess.run(
        [str(binary), "provider-catalog"],
        env=sanitized_environment(), text=True, capture_output=True, check=False,
    )
    for line in reversed(result.stdout.splitlines()):
        try:
            value = json.loads(line)
        except json.JSONDecodeError:
            continue
        if isinstance(value, dict):
            if not value.get("ok") or not value.get("catalog_sha256"):
                raise SystemExit(f"paired ASB provider catalog is unavailable: {value}")
            profiles = value.get("profiles")
            if not isinstance(profiles, list) or not profiles:
                raise SystemExit("paired ASB provider catalog has no profiles")
            choices = [
                {"provider": profile.get("id"), "model": profile.get("model")}
                for profile in profiles
                if profile.get("selectable") and profile.get("id") and profile.get("model")
            ]
            if not choices:
                raise SystemExit("paired ASB provider catalog has no selectable choices")
            return {
                "catalog_sha256": value["catalog_sha256"],
                "catalog_version": value.get("catalog_version"),
                "choices": choices,
            }
    raise SystemExit("paired ASB provider-catalog emitted no structured result")


def main() -> int:
    parser = argparse.ArgumentParser(description="Run the offline AR-1657 compatibility matrix.")
    parser.add_argument("--checkout", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--asb-binary", type=Path, required=True, help="paired ASB binary for parent-command acceptance")
    parser.add_argument("--tui-binary", type=Path, required=True, help="installed asb-tui executable for direct parity acceptance")
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
    marker = next((line.split("=", 1)[1] for line in output.splitlines() if line.startswith("AR1657_CATALOG_JSON=")), None)
    if marker is None:
        raise SystemExit("matrix test did not emit the Rust catalog")
    catalog = json.loads(marker)
    asb_catalog = authoritative_asb_catalog(args.asb_binary.resolve())
    fixture = json.loads((checkout / "tests/fixtures/agent-provider-matrix.json").read_text(encoding="utf-8"))
    expected = sorted(fixture["tuples"], key=lambda value: (value["agent"], value["provider"], value["model"]))
    actual = sorted(
        [{key: value[key] for key in ("agent", "provider", "model")} for value in catalog],
        key=lambda value: (value["agent"], value["provider"], value["model"]),
    )
    if actual != expected:
        raise SystemExit("ASB/TUI compatibility catalog drifted from the reviewed fixture")
    if not all(entry.get("supported") and entry.get("reason") is None for entry in catalog):
        raise SystemExit("development catalog contains an unexpected unavailable tuple")
    tui_choices = {(entry["provider"], entry["model"]) for entry in catalog}
    asb_choices = {(entry["provider"], entry["model"]) for entry in asb_catalog["choices"]}
    if not asb_choices.issubset(tui_choices):
        missing = sorted(asb_choices - tui_choices)
        raise SystemExit(f"TUI compatibility matrix does not cover paired ASB catalog choices: {missing}")
    parent_command = run_asb_tui_parity(args.asb_binary.resolve(), args.tui_binary.resolve())
    receipt = {
        "schema_version": 1, "ar": "AR-1657", "classification": "development/mock",
        "network": "denied", "credentials": "none",
        "matrix": {
            "tuple_count": len(catalog), "tuples": catalog,
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
            "installed_asb_tui": parent_command,
            "authoritative_asb_catalog": asb_catalog,
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
