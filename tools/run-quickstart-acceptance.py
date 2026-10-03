#!/usr/bin/env python3
# Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Run the bounded selection-driven development quickstart acceptance."""

from __future__ import annotations

import argparse
import copy
import hashlib
import json
import os
import pty
import subprocess
import sys
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


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def find_installed_executable(root: Path) -> Path:
    direct = root / "asb-tui"
    if direct.is_file():
        return direct
    candidates = sorted(root.glob("dev-versions/*/asb-tui"))
    if len(candidates) != 1:
        raise AssertionError(f"expected one development executable, found {candidates}")
    return candidates[0]


def development_handoff(asb_checkout: Path, tui_commit: str, tui_tree: str, executable: Path, path: Path) -> dict:
    """Write the privacy-safe ASB-to-TUI development handoff envelope."""
    value = {
        "schema_version": 1,
        "channel": "dev",
        "development_only": True,
        "asb_repository": "https://github.com/martin-beck/agent-systems-benchmark.git",
        "asb_ref": "refs/heads/main",
        "asb_source_commit": git(asb_checkout, "HEAD"),
        "asb_source_tree": git(asb_checkout, "HEAD^{tree}"),
        "tui_repository": "https://github.com/martin-beck/asb-tui.git",
        "tui_ref": "refs/heads/main",
        "tui_source_commit": tui_commit,
        "tui_source_tree": tui_tree,
        "executable_sha256": sha256(executable),
        "executable_size": executable.stat().st_size,
        "built_unix": 1,
        "warnings": [
            "development_missing_authentication_allowed",
            "development_missing_signatures_allowed",
            "development_missing_key_management_allowed",
        ],
    }
    path.write_text(json.dumps(value), encoding="utf-8")
    return value


_SECRET_ENV_MARKERS = (
    "API_KEY",
    "APIKEY",
    "ACCESS_TOKEN",
    "AUTH_TOKEN",
    "CREDENTIAL",
    "PASSWORD",
    "PRIVATE_KEY",
    "SECRET",
    "TOKEN",
)


def sanitized_environment() -> dict[str, str]:
    """Keep runner diagnostics credential-free while preserving deny policy."""
    environment = {
        key: value
        for key, value in os.environ.items()
        if not any(marker in key.upper() for marker in _SECRET_ENV_MARKERS)
    }
    environment.update(
        {
            "ASB_TUI_NETWORK_POLICY": "deny",
            "HTTP_PROXY": "http://127.0.0.1:1",
            "HTTPS_PROXY": "http://127.0.0.1:1",
            "ALL_PROXY": "http://127.0.0.1:1",
            "NO_PROXY": "*",
        }
    )
    return environment


def asb_json(binary: Path, arguments: list[str], env: dict[str, str]) -> dict:
    result = subprocess.run(
        [str(binary), "--json", *arguments],
        env=env,
        text=True,
        capture_output=True,
        check=False,
        timeout=30,
    )
    lines = result.stdout.splitlines()
    for line in reversed(lines):
        try:
            value = json.loads(line)
            if result.returncode not in (0, 3):
                raise AssertionError(f"ASB command failed: {arguments}: {result.stderr}")
            return value
        except json.JSONDecodeError:
            continue
    raise AssertionError(f"ASB command emitted no JSON: {arguments}: {result.stdout}{result.stderr}")


def run_channel_compatibility_matrix(
    binary: Path, asb_checkout: Path, asb_head: str, receipt: Path
) -> dict:
    """Run the paired lifecycle matrix as part of the quickstart release gate."""
    runner = Path(__file__).with_name("run-channel-compatibility-matrix.py")
    result = subprocess.run(
        [sys.executable, str(runner), str(binary), "--asb-checkout", str(asb_checkout),
         "--asb-head", asb_head, "--receipt", str(receipt)],
        cwd=runner.parent.parent, text=True, capture_output=True, check=False,
    )
    if result.returncode != 0:
        raise AssertionError(f"paired channel matrix failed: {result.stdout}{result.stderr}")
    try:
        value = json.loads(result.stdout.splitlines()[-1])
    except (IndexError, json.JSONDecodeError) as error:
        raise AssertionError(f"paired channel matrix emitted no JSON: {result.stdout}{result.stderr}") from error
    assert value["ar"] == "AR-1676"
    assert len(value["cases"]) == 11
    return value


def select_workload_ids(catalog: dict) -> list[str]:
    return sorted(
        entry["id"]
        for entry in catalog["entries"]
        if entry["platform"].startswith("linux-x86_64") and entry["id"].startswith("original.")
    )


def run_capture_replay_matrix(asb_binary: Path, asb_checkout: Path, env: dict[str, str], root: Path) -> dict:
    root.mkdir(parents=True, exist_ok=True)
    # Use the same provider dialect accepted by the runtime-owned strict
    # replay seam.  The generic synthetic fixture is schema-valid but is not a
    # runnable strict-replay route, so recording it would falsely report
    # offline readiness.
    fixture = asb_checkout / "crates/asb-replay/fixtures/v1/gemini-generate-content.json"
    fixture_document = json.loads(fixture.read_text(encoding="utf-8"))
    # The public Gemini fixture intentionally contains a redacted API-key
    # header and a provider-specific selector set.  The development capture
    # route must produce a fresh, sealable artifact without carrying that
    # already-redacted credential marker into the runner-owned cassette.
    contents = copy.deepcopy(fixture_document["contents"])
    for interaction in contents["interactions"]:
        interaction["request"]["headers"] = [
            header
            for header in interaction["request"]["headers"]
            if header["name"] != "x-goog-api-key"
        ]
    selectors = contents["redaction"]["selectors"]
    selectors["header_names"] = [
        name for name in selectors["header_names"] if name != "x-goog-api-key"
    ]
    contents["redaction"]["selector_sha256"] = ""
    capture = {
        "schema_version": 1,
        "provider_profile_sha256": "a" * 64,
        "agent_id": "codex",
        "network": "loopback_only",
        "estimated_cost_minor": 0,
        "confirmation": {"record": True, "network": True, "cost": False},
        "contents": contents,
    }
    capture_path = root / "capture.json"
    capture_path.write_text(json.dumps(capture), encoding="utf-8")
    catalog = asb_json(asb_binary, ["workload-catalog"], env)
    workload_ids = select_workload_ids(catalog)
    assert workload_ids, "no fixture-only workload catalog entries"

    def campaign(name: str, selected: list[str]) -> tuple[dict, Path]:
        campaign_root = root / name
        campaign_root.mkdir()
        entries = []
        for workload_id in selected:
            entries.append({
                "workload_id": workload_id,
                "capture_path": str(capture_path),
                "cassette_path": str(campaign_root / f"{workload_id}.json"),
            })
        manifest = {
            "schema_version": 1,
            "provider_profile_sha256": "a" * 64,
            "agent_ids": ["codex"],
            "workload_ids": selected,
            "entries": entries,
        }
        manifest_path = campaign_root / "manifest.json"
        manifest_path.write_text(json.dumps(manifest), encoding="utf-8")
        return asb_json(asb_binary, ["easy", "record-campaign", str(manifest_path), "--local-mock"], env), campaign_root

    selected, selected_root = campaign("selected", workload_ids[:1])
    assert selected["complete_coverage"] and selected["offline_ready"], selected
    generated_cassette = selected_root / f"{workload_ids[0]}.json"
    generated_replay = asb_json(
        asb_binary,
        ["easy", "replay-offline", str(generated_cassette), "a" * 64, "codex", "--local-mock"],
        env,
    )
    assert generated_replay["ok"] and generated_replay["network"] == "denied", generated_replay
    all_workloads, all_root = campaign("all", workload_ids)
    assert all_workloads.get("complete_coverage") and all_workloads.get("offline_ready"), all_workloads

    incomplete_manifest = json.loads((all_root / "manifest.json").read_text(encoding="utf-8"))
    incomplete_manifest["entries"] = incomplete_manifest["entries"][:-1]
    incomplete_path = root / "incomplete.json"
    incomplete_path.write_text(json.dumps(incomplete_manifest), encoding="utf-8")
    incomplete = asb_json(asb_binary, ["easy", "record-campaign", str(incomplete_path), "--local-mock"], env)
    assert not incomplete["complete_coverage"] and not incomplete["offline_ready"], incomplete

    strict_fixture = asb_checkout / "crates/asb-replay/fixtures/v1/gemini-generate-content.json"
    strict_digest = json.loads(strict_fixture.read_text(encoding="utf-8"))["integrity"]["digest"]
    replay = asb_json(
        asb_binary,
        ["easy", "replay-offline", str(strict_fixture), "a" * 64, "codex", "--local-mock"],
        env,
    )
    assert replay["ok"] and replay["network"] == "denied" and replay["source"] == "strict_replay", replay
    return {
        "selected": {
            "workload_count": 1,
            "tuple_count": selected["tuple_count"],
            "cassette_sha256": sha256(generated_cassette),
            "replay": {
                "cassette_sha256": generated_replay["cassette_sha256"],
                "result_digest": generated_replay["result_digest"],
                "network": generated_replay["network"],
            },
        },
        "all": {"workload_count": len(workload_ids), "tuple_count": all_workloads["tuple_count"]},
        "incomplete_matrix": {"complete_coverage": incomplete["complete_coverage"], "offline_ready": incomplete["offline_ready"]},
        "strict_replay": {
            "cassette_sha256": generated_replay["cassette_sha256"],
            "result_digest": generated_replay["result_digest"],
            "network": generated_replay["network"],
        },
        "strict_replay_reference": {
            "cassette_sha256": strict_digest,
            "result_digest": replay["result_digest"],
            "network": replay["network"],
        },
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("binary", type=Path)
    parser.add_argument("--receipt", type=Path, required=True)
    parser.add_argument("--asb-checkout", type=Path, default=Path(os.environ.get("ASB_SOURCE_CHECKOUT", "/srv/data/projects/agent-systems-benchmark")))
    parser.add_argument("--asb-head")
    parser.add_argument("--asb-binary", type=Path, default=Path(os.environ.get("ASB_BINARY", "")) if os.environ.get("ASB_BINARY") else None)
    parser.add_argument("--json", action="store_true", help="print the machine-readable receipt")
    args = parser.parse_args()
    checkout = Path(__file__).parents[1].resolve()
    binary = args.binary.resolve()
    asb_checkout = args.asb_checkout.resolve()
    asb_head = args.asb_head or git(asb_checkout, "HEAD")
    asb_binary = (args.asb_binary or (asb_checkout / "target/debug/asb")).resolve()
    if not asb_binary.is_file():
        raise SystemExit(f"ASB binary is missing: {asb_binary}")
    env = sanitized_environment()
    with tempfile.TemporaryDirectory(prefix="asb-tui-quickstart-") as disposable:
        env.update({
            "ASB_TUI_CHANNEL_STATE": f"{disposable}/channel.json",
            "ASB_TUI_DEV_INSTALL_ROOT": f"{disposable}/install",
            # Default to the verified public main ref. Qualification runners
            # may explicitly bind a local exact-head repository/ref; retaining
            # those values prevents a receipt from mixing the tested binary
            # with a different materialized source head.
            "ASB_TUI_DEV_REPOSITORY": os.environ.get(
                "ASB_TUI_DEV_REPOSITORY",
                "https://github.com/martin-beck/asb-tui.git",
            ),
            "ASB_TUI_DEV_REF": os.environ.get("ASB_TUI_DEV_REF", "main"),
            # The materializer gives Cargo a disposable HOME; retain the
            # owner-private pinned rustup toolchain explicitly.
            "ASB_TUI_DEV_RUSTUP_HOME": os.environ.get(
                "RUSTUP_HOME", str(Path.home() / ".rustup")
            ),
            "ASB_TUI_NETWORK_POLICY": "deny",
            "HTTP_PROXY": "http://127.0.0.1:1",
            "HTTPS_PROXY": "http://127.0.0.1:1",
            "ALL_PROXY": "http://127.0.0.1:1",
            "NO_PROXY": "*",
        })
        channel_matrix = run_channel_compatibility_matrix(
            binary, asb_checkout, asb_head, Path(disposable) / "channel-matrix.json"
        )
        matrix = run_capture_replay_matrix(asb_binary, asb_checkout, env, Path(disposable) / "matrix")
        # Omitted channel must select dev. First materialize from the exact
        # local main checkout, then build a content-bound handoff from the
        # resulting executable and consume it on a fresh install root.
        # Installation is the only network-capable step (clone/build); all
        # benchmark and replay operations below retain the deny policy.
        install_env = dict(env)
        install_env["ASB_TUI_NETWORK_POLICY"] = "allow"
        for key in ("HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "NO_PROXY"):
            install_env.pop(key, None)
        initial = invoke(binary, ["tui", "install", "--json"], {}, install_env)
        assert initial["channel"] == "dev" and initial["ok"], initial
        installed = find_installed_executable(Path(env["ASB_TUI_DEV_INSTALL_ROOT"]))
        handoff_path = Path(disposable) / "channel-manifest.json"
        tui_commit = initial["source_commit"]
        tui_tree = initial["source_tree"]
        assert len(tui_commit) == 40 and len(tui_tree) == 40, initial
        handoff = development_handoff(asb_checkout, tui_commit, tui_tree, installed, handoff_path)
        # Capture this before the disposable workspace is torn down.  The
        # receipt is emitted after the acceptance scope so it must not retain
        # a path into TemporaryDirectory.
        handoff_manifest_sha256 = hashlib.sha256(handoff_path.read_bytes()).hexdigest()
        env["ASB_TUI_CHANNEL_MANIFEST"] = str(handoff_path)
        env["ASB_TUI_DEV_INSTALL_ROOT"] = f"{disposable}/handoff-install"
        install_env["ASB_TUI_CHANNEL_MANIFEST"] = str(handoff_path)
        install_env["ASB_TUI_DEV_INSTALL_ROOT"] = env["ASB_TUI_DEV_INSTALL_ROOT"]
        installed_with_handoff = invoke(binary, ["tui", "install", "--json"], {}, install_env)
        assert installed_with_handoff["channel"] == "dev" and installed_with_handoff["ok"], installed_with_handoff
        installed_executable = find_installed_executable(Path(env["ASB_TUI_DEV_INSTALL_ROOT"]))
        installed_executable_sha256 = sha256(installed_executable)
        status = invoke(binary, ["tui", "status", "--json"], {}, env)
        assert status["channel"] == "dev" and status["installed"] and status["verified"], status
        assert status["channel_manifest_sha256"] == hashlib.sha256(handoff_path.read_bytes()).hexdigest(), status
        assert status["asb_source_commit"] == handoff["asb_source_commit"], status
        launch = invoke(binary, ["tui", "launch", "--json"], {}, env)
        assert launch["channel"] == "dev" and launch["code"] in {"development_launch_ready", "development_launch_unavailable"}, launch
        restarted = invoke(binary, ["tui", "status", "--json"], {}, env)
        assert restarted["channel_manifest_sha256"] == status["channel_manifest_sha256"], restarted
        # Typed tamper negative: status must not accept edited handoff bytes.
        installed_manifest = Path(env["ASB_TUI_DEV_INSTALL_ROOT"]) / "channel-manifest.json"
        original_handoff = installed_manifest.read_bytes()
        installed_manifest.write_bytes(original_handoff + b" tampered")
        tampered = invoke(binary, ["tui", "status", "--json"], {}, env)
        assert tampered["code"] in {"dev_channel_manifest_invalid", "dev_channel_manifest_digest_mismatch"}, tampered
        installed_manifest.write_bytes(original_handoff)
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
    manifest_digest = hashlib.sha256(manifest.read_bytes()).hexdigest() if manifest.is_file() else None
    receipt = {
        "schema_version": 1, "ar": "AR-1693", "classification": "development/mock",
        "network": {"install_materialization": "allowed", "benchmark_and_replay": "denied"}, "credentials": "none",
        "selection": {"channel": "dev", "provider": "fixture", "auth": "development_fixture", "agent": "opencode", "model": "fixture-model", "cassette": "strict-replay-cassette", "replay": "offline"},
        "routes": {"install": installed_with_handoff, "status": status, "restart_status": restarted, "launch": launch, "tamper_negative": tampered, "onboarding": onboarding, "journey": journey, "provider_selection": "provider_lifecycle_1656 passed", "fanout_capture_replay": "coverage_setup_recording and development_journey passed", "end_to_end_test": "passed", "channel_compatibility_matrix": channel_matrix, "capture_replay_matrix": matrix, "comparison": "end_to_end_qualification passed", "analysis": "end_to_end_qualification passed"},
        "provenance": {
            "tui_commit": tui_commit, "tui_tree": tui_tree,
            "asb_commit": asb_head, "asb_tree": git(asb_checkout, f"{asb_head}^{{tree}}"),
            "manifest": "release/channel-status.json", "manifest_sha256": manifest_digest,
            "handoff_manifest_sha256": handoff_manifest_sha256,
            "handoff": handoff,
            "runner_binary_sha256": sha256(binary),
            "installed_executable_sha256": installed_executable_sha256,
            "asb_binary_sha256": sha256(asb_binary),
        },
    }
    args.receipt.parent.mkdir(parents=True, exist_ok=True)
    args.receipt.write_text(json.dumps(receipt, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(receipt, sort_keys=True) if args.json else "AR-1693 quickstart acceptance passed (development/mock; install materialization allowed, benchmark/replay network denied)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
