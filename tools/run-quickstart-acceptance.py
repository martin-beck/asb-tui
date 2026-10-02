#!/usr/bin/env python3
# Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Run the bounded selection-driven development quickstart acceptance."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import pty
import subprocess
import tempfile
from pathlib import Path


def invoke(binary: Path, arguments: list[str], payload: dict, env: dict[str, str]) -> dict:
    master, slave = pty.openpty()
    request = tempfile.NamedTemporaryFile(mode="w", encoding="utf-8", delete=False)
    request.write(json.dumps(payload) + "\n")
    request.close()
    request_path = Path(request.name)
    try:
        process = subprocess.Popen(
            [str(binary), *arguments], stdin=request_path.open("rb"), stdout=slave, stderr=slave,
            env=env, close_fds=True,
        )
    finally:
        os.close(slave)
    try:
        output = bytearray()
        while True:
            try:
                block = os.read(master, 65536)
            except OSError:
                break
            if not block:
                break
            output.extend(block)
    finally:
        os.close(master)
        request_path.unlink(missing_ok=True)
    assert process.wait() in (0, 3), output
    for line in reversed(output.decode("utf-8", "replace").splitlines()):
        try:
            return json.loads(line.strip())
        except json.JSONDecodeError:
            continue
    raise AssertionError(f"no JSON response for {arguments}: {output!r}")


def git(checkout: Path, value: str) -> str:
    return subprocess.check_output(["git", "-C", str(checkout), "rev-parse", value], text=True).strip()


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("binary", type=Path)
    parser.add_argument("--receipt", type=Path, required=True)
    parser.add_argument("--asb-checkout", type=Path, default=Path(os.environ.get("ASB_SOURCE_CHECKOUT", "/srv/data/projects/agent-systems-benchmark")))
    parser.add_argument("--asb-head")
    parser.add_argument("--json", action="store_true", help="print the machine-readable receipt")
    args = parser.parse_args()
    checkout = Path(__file__).parents[1].resolve()
    binary = args.binary.resolve()
    asb_checkout = args.asb_checkout.resolve()
    asb_head = args.asb_head or git(asb_checkout, "HEAD")
    env = os.environ.copy()
    with tempfile.TemporaryDirectory(prefix="asb-tui-quickstart-") as disposable:
        env.update({
            "ASB_TUI_CHANNEL_STATE": f"{disposable}/channel.json",
            "ASB_TUI_DEV_INSTALL_ROOT": f"{disposable}/install",
            "ASB_TUI_NETWORK_POLICY": "deny",
            "HTTP_PROXY": "http://127.0.0.1:1",
            "HTTPS_PROXY": "http://127.0.0.1:1",
            "ALL_PROXY": "http://127.0.0.1:1",
            "NO_PROXY": "*",
        })
        status = invoke(binary, ["tui", "status", "--channel", "dev", "--json"], {}, env)
        assert status["channel"] == "dev" and status["code"] == "development_not_installed", status
        launch = invoke(binary, ["tui", "launch", "--channel", "dev", "--json"], {}, env)
        assert launch["code"] == "development_launch_unavailable", launch
        onboarding = invoke(binary, ["onboarding", "--format", "json"], {
            "schema_version": 1, "profile": "development", "bundle_available": True,
            "broker_available": True, "protocol_version": 1,
        }, env)
        assert onboarding["code"] == "development_onboarding_ready", onboarding
        journey = invoke(binary, ["journey", "--format", "json"], {
            "schema_version": 1, "profile": "development", "asb_version": "fixture-1",
            "expected_asb_version": "fixture-1", "catalog_version": 7,
            "expected_catalog_version": 7, "bundle_available": True,
            "broker_available": True, "protocol_version": 1,
            "expected_protocol_version": 1,
        }, env)
        assert journey["code"] == "development_journey_ready", journey
        qualification_tests = [
            "provider_lifecycle_1656",  # OpenCode/OpenDesk/model selection and defaults.
            "coverage_setup_recording",  # fan-out, capture, cancellation, and replay dispatch.
            "development_journey",  # credential-free capture/replay/comparison fixture.
            "end_to_end_qualification",  # complete install-to-comparison acceptance.
        ]
        for test_name in qualification_tests:
            test = subprocess.run(
                ["cargo", "test", "--locked", "--test", test_name],
                cwd=checkout, env=env, text=True, capture_output=True, check=False,
            )
            assert test.returncode == 0, f"{test_name}: {test.stdout}{test.stderr}"
    manifest = checkout / "release" / "channel-status.json"
    receipt = {
        "schema_version": 1, "ar": "AR-1660", "classification": "development/mock",
        "network": "denied", "credentials": "none",
        "selection": {"channel": "dev", "provider": "fixture", "auth": "development_fixture", "agent": "opencode", "model": "fixture-model", "cassette": "strict-replay-cassette", "replay": "offline"},
        "routes": {"install": status, "launch": launch, "onboarding": onboarding, "journey": journey, "provider_selection": "provider_lifecycle_1656 passed", "fanout_capture_replay": "coverage_setup_recording and development_journey passed", "end_to_end_test": "passed"},
        "provenance": {
            "tui_commit": git(checkout, "HEAD"), "tui_tree": git(checkout, "HEAD^{tree}"),
            "asb_commit": asb_head, "asb_tree": git(asb_checkout, f"{asb_head}^{{tree}}"),
            "manifest": "release/channel-status.json", "manifest_sha256": hashlib.sha256(manifest.read_bytes()).hexdigest(),
            "executable_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
        },
    }
    args.receipt.parent.mkdir(parents=True, exist_ok=True)
    args.receipt.write_text(json.dumps(receipt, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(receipt, sort_keys=True) if args.json else "AR-1660 quickstart acceptance passed (development/mock, network denied)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
