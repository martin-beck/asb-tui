#!/usr/bin/env python3
# Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Qualify the operator journey through the public ASB command boundary."""
from __future__ import annotations
import argparse, hashlib, json, os, pty, subprocess, sys, tempfile
from pathlib import Path

SECRET_MARKERS = ("API_KEY", "APIKEY", "ACCESS_TOKEN", "AUTH_TOKEN", "CREDENTIAL", "PASSWORD", "PRIVATE_KEY", "SECRET", "TOKEN")

def safe_environment() -> dict[str, str]:
    environment = {k: v for k, v in os.environ.items() if not any(m in k.upper() for m in SECRET_MARKERS)}
    environment.update({"ASB_TUI_NETWORK_POLICY": "deny", "HTTP_PROXY": "http://127.0.0.1:1", "HTTPS_PROXY": "http://127.0.0.1:1", "ALL_PROXY": "http://127.0.0.1:1", "NO_PROXY": "*"})
    return environment

def run_asb(asb: Path, arguments: list[str], environment: dict[str, str], *, json_output: bool) -> tuple[int, str]:
    command = [str(asb), *arguments]
    master, slave = pty.openpty()
    process = subprocess.Popen(command, env=environment, stdin=slave, stdout=slave,
                               stderr=slave, close_fds=True)
    os.close(slave)
    chunks = bytearray()
    try:
        while True:
            try:
                chunk = os.read(master, 65536)
            except OSError:
                break
            if not chunk:
                break
            chunks.extend(chunk)
    finally:
        os.close(master)
    result_code = process.wait(timeout=60)
    output = chunks.decode("utf-8", "replace").strip()
    if not output:
        raise AssertionError(f"ASB command emitted no output: {' '.join(command)}")
    if json_output:
        for line in reversed(output.splitlines()):
            try:
                return result_code, json.dumps(json.loads(line), sort_keys=True)
            except json.JSONDecodeError:
                continue
        raise AssertionError(f"ASB command emitted no JSON: {command}: {output}")
    return result_code, output

def git_identity(checkout: Path) -> tuple[str, str]:
    commit = subprocess.check_output(["git", "-C", str(checkout), "rev-parse", "HEAD"], text=True).strip()
    tree = subprocess.check_output(["git", "-C", str(checkout), "rev-parse", "HEAD^{tree}"], text=True).strip()
    return commit, tree

def verify_binary_identity(binary: Path, checkout: Path, label: str) -> tuple[str, str]:
    commit, tree = git_identity(checkout)
    data = binary.read_bytes()
    if commit.encode() not in data or tree.encode() not in data:
        raise AssertionError(f"{label} binary is not bound to supplied checkout {commit}/{tree}")
    return commit, tree

def require_human(result: tuple[int, str], expected: tuple[str, ...], route: str) -> str:
    code, output = result
    if code != 0 or not any(marker in output for marker in expected):
        raise AssertionError(f"human {route} failed or had an unexpected outcome: {code}: {output}")
    return output

def main() -> int:
    parser = argparse.ArgumentParser(description="Run the ASB/TUI operator quickstart.")
    parser.add_argument("--asb-binary", type=Path, required=True)
    parser.add_argument("--asb-checkout", type=Path, required=True)
    parser.add_argument("--tui-binary", type=Path, required=True)
    parser.add_argument("--tui-checkout", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--receipt", type=Path, required=True)
    parser.add_argument("--bundle", type=Path, required=True,
                        help="validated private development bundle for ASB router")
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args()
    asb, tui = args.asb_binary.resolve(), args.tui_binary.resolve()
    asb_checkout, tui_checkout = args.asb_checkout.resolve(), args.tui_checkout.resolve()
    for path in (asb, tui):
        if not path.is_file() or not os.access(path, os.X_OK):
            raise SystemExit(f"executable is missing or not executable: {path}")
    asb_commit, asb_tree = verify_binary_identity(asb, asb_checkout, "ASB")
    tui_commit, tui_tree = verify_binary_identity(tui, tui_checkout, "TUI")
    with tempfile.TemporaryDirectory(prefix="asb-tui-ar1654-") as temporary:
        root = Path(temporary)
        environment = safe_environment()
        environment.update({
            "HOME": str(root / "home"),
            "XDG_DATA_HOME": str(root / "data"),
            "XDG_STATE_HOME": str(root / "state"),
            "XDG_CACHE_HOME": str(root / "cache"),
            "ASB_TUI_CHANNEL_STATE": str(root / "channel.json"),
            "ASB_TUI_DEV_INSTALL_ROOT": str(root / "install"),
            "ASB_TUI_DEV_RUSTUP_HOME": environment.get("RUSTUP_HOME", str(Path.home() / ".rustup")),
            "RUSTUP_TOOLCHAIN": environment.get("RUSTUP_TOOLCHAIN", "1.93.0-x86_64-unknown-linux-gnu"),
        })
        bundle = args.bundle.resolve()
        if not bundle.is_dir() or bundle.stat().st_mode & 0o077:
            raise SystemExit("--bundle must be a private directory (mode 0700 or stricter)")
        environment["ASB_TUI_DEV_BUNDLE"] = str(bundle)
        install_json = run_asb(asb, ["tui", "install", "--json"], environment, json_output=True)
        if install_json[0] not in (0, 3):
            raise AssertionError(f"asb tui install failed: {install_json}")
        launch_json = run_asb(asb, ["tui", "--json"], environment, json_output=True)
        launch_value = json.loads(launch_json[1])
        # A headless CI PTY can admit the public route but cannot keep the
        # interactive development broker alive.  Preserve that typed outcome
        # as bounded qualification evidence; a real terminal must report
        # ``development_launched``.
        if launch_json[0] != 0 or launch_value.get("ok") is not True or launch_value.get("code") != "development_launched":
            raise AssertionError(f"bare asb tui failed: {launch_json}")
        install_human = run_asb(asb, ["tui", "install"], environment, json_output=False)
        launch_human = run_asb(asb, ["tui"], environment, json_output=False)
        require_human(install_human, ("development_bundle_consumed", "development_bundle_verified"), "asb tui install")
        require_human(launch_human, ("development_launched",), "asb tui")
        qualification_tests = ["provider_lifecycle_1656", "coverage_setup_recording",
                               "development_journey", "end_to_end_qualification"]
        test_results = {}
        for test_name in qualification_tests:
            completed = subprocess.run(["cargo", "test", "--locked", "--test", test_name],
                                       cwd=tui_checkout, env=environment, text=True,
                                       capture_output=True, check=False, timeout=120)
            if completed.returncode != 0:
                raise AssertionError(f"{test_name} failed:\n{completed.stdout[-4000:]}{completed.stderr[-4000:]}")
            test_results[test_name] = "passed"
        journey = {"tests": test_results, "scope": "wizard/benchmark/record-replay/comparison"}
        receipt = {"schema_version": 1, "ar": "AR-1654", "classification": "development/mock", "credentials": "none", "network": {"install": "local_bundle", "benchmark_and_replay": "denied"}, "operator_commands": {"install": "asb tui install", "launch": "asb tui", "install_json": json.loads(install_json[1]), "launch_json": launch_value, "install_human_nonempty": bool(install_human[1]), "launch_human_nonempty": bool(launch_human[1])}, "journey": journey, "provenance": {"asb_binary_sha256": hashlib.sha256(asb.read_bytes()).hexdigest(), "tui_binary_sha256": hashlib.sha256(tui.read_bytes()).hexdigest(), "asb_checkout_head": asb_commit, "asb_checkout_tree": asb_tree, "tui_checkout_head": tui_commit, "tui_checkout_tree": tui_tree, "asb_checkout": str(asb_checkout), "tui_checkout": str(tui_checkout)}}
    args.receipt.parent.mkdir(parents=True, exist_ok=True)
    args.receipt.write_text(json.dumps(receipt, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(receipt, sort_keys=True) if args.json else "AR-1654 operator quickstart passed (development/mock; credentials absent)")
    return 0

if __name__ == "__main__":
    raise SystemExit(main())
