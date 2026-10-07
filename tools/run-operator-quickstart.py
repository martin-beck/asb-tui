#!/usr/bin/env python3
# Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Qualify the operator journey through the public ASB command boundary."""
from __future__ import annotations
import argparse
import errno
import fcntl
import hashlib
import json
import os
import pty
import selectors
import signal
import struct
import subprocess
import sys
import tempfile
import termios
import time
from pathlib import Path

SECRET_MARKERS = ("API_KEY", "APIKEY", "ACCESS_TOKEN", "AUTH_TOKEN", "CREDENTIAL", "PASSWORD", "PRIVATE_KEY", "SECRET", "TOKEN")
PTY_ROWS = 24
PTY_COLUMNS = 80
MAX_COMMAND_OUTPUT = 2 * 1024 * 1024

def safe_environment() -> dict[str, str]:
    environment = {k: v for k, v in os.environ.items() if not any(m in k.upper() for m in SECRET_MARKERS)}
    environment.update({"ASB_TUI_NETWORK_POLICY": "deny", "HTTP_PROXY": "http://127.0.0.1:1", "HTTPS_PROXY": "http://127.0.0.1:1", "ALL_PROXY": "http://127.0.0.1:1", "NO_PROXY": "*"})
    return environment

def _wait_nohang(pid: int) -> int | None:
    waited, status = os.waitpid(pid, os.WNOHANG)
    return None if waited == 0 else os.waitstatus_to_exitcode(status)


def _terminate_session(pid: int) -> None:
    if _wait_nohang(pid) is not None:
        return
    try:
        os.killpg(pid, signal.SIGTERM)
    except ProcessLookupError:
        pass
    deadline = time.monotonic() + 0.5
    while time.monotonic() < deadline:
        if _wait_nohang(pid) is not None:
            return
        time.sleep(0.01)
    try:
        os.killpg(pid, signal.SIGKILL)
    except ProcessLookupError:
        pass
    try:
        os.waitpid(pid, 0)
    except ChildProcessError:
        pass


def _spawn_controlling_pty(
    command: list[str], environment: dict[str, str]
) -> tuple[int, int]:
    master, slave = pty.openpty()
    slave_name = os.ttyname(slave)
    requested = struct.pack("HHHH", PTY_ROWS, PTY_COLUMNS, 0, 0)
    fcntl.ioctl(slave, termios.TIOCSWINSZ, requested)
    observed = fcntl.ioctl(slave, termios.TIOCGWINSZ, b"\0" * len(requested))
    rows, columns, _, _ = struct.unpack("HHHH", observed)
    if rows != PTY_ROWS or columns != PTY_COLUMNS:
        os.close(master)
        os.close(slave)
        raise AssertionError(f"failed to establish {PTY_COLUMNS}x{PTY_ROWS} PTY")
    actions = [
        (os.POSIX_SPAWN_CLOSE, master),
        (os.POSIX_SPAWN_OPEN, 0, slave_name, os.O_RDWR, 0),
        (os.POSIX_SPAWN_DUP2, 0, 1),
        (os.POSIX_SPAWN_DUP2, 0, 2),
    ]
    try:
        pid = os.posix_spawn(
            command[0], command, environment, file_actions=actions, setsid=True
        )
    except BaseException:
        os.close(master)
        raise
    finally:
        os.close(slave)
    try:
        if os.tcgetpgrp(master) != pid:
            raise AssertionError("PTY child is not its controlling foreground group")
        actual = fcntl.ioctl(master, termios.TIOCGWINSZ, b"\0" * len(requested))
        rows, columns, _, _ = struct.unpack("HHHH", actual)
        if rows <= 0 or columns <= 0:
            raise AssertionError("PTY child received a zero-size terminal")
    except BaseException:
        _terminate_session(pid)
        os.close(master)
        raise
    os.set_blocking(master, False)
    return pid, master


def run_asb(
    asb: Path,
    arguments: list[str],
    environment: dict[str, str],
    *,
    json_output: bool,
    interactive_quit: bool = False,
    timeout_seconds: float = 60.0,
    output_limit: int = MAX_COMMAND_OUTPUT,
) -> tuple[int, str]:
    command = [str(asb), *arguments]
    pid, master = _spawn_controlling_pty(command, environment)
    chunks = bytearray()
    selector = selectors.DefaultSelector()
    selector.register(master, selectors.EVENT_READ)
    deadline = time.monotonic() + timeout_seconds
    result_code: int | None = None
    quit_sent = False
    try:
        while result_code is None:
            if time.monotonic() >= deadline:
                raise AssertionError(f"ASB command timed out: {' '.join(command)}")
            for _, _ in selector.select(timeout=0.05):
                try:
                    chunk = os.read(master, min(65536, output_limit + 1 - len(chunks)))
                except BlockingIOError:
                    continue
                except OSError as error:
                    if error.errno == errno.EIO:
                        continue
                    raise
                if chunk:
                    chunks.extend(chunk)
                    if len(chunks) > output_limit:
                        raise AssertionError(f"ASB command exceeded output limit: {' '.join(command)}")
            if interactive_quit and chunks and not quit_sent:
                try:
                    os.write(master, b"q")
                except BlockingIOError:
                    continue
                quit_sent = True
            result_code = _wait_nohang(pid)
        while True:
            try:
                chunk = os.read(master, min(65536, output_limit + 1 - len(chunks)))
            except BlockingIOError:
                break
            except OSError as error:
                if error.errno == errno.EIO:
                    break
                raise
            if not chunk:
                break
            chunks.extend(chunk)
            if len(chunks) > output_limit:
                raise AssertionError(f"ASB command exceeded output limit: {' '.join(command)}")
    finally:
        selector.close()
        if result_code is None:
            _terminate_session(pid)
        os.close(master)
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
        launch_json = run_asb(asb, ["tui", "--json"], environment, json_output=True,
                              interactive_quit=True)
        launch_value = json.loads(launch_json[1])
        if launch_json[0] != 0 or launch_value.get("ok") is not True or launch_value.get("code") != "development_launched":
            raise AssertionError(f"bare asb tui failed: {launch_json}")
        install_human = run_asb(asb, ["tui", "install"], environment, json_output=False)
        launch_human = run_asb(asb, ["tui"], environment, json_output=False,
                               interactive_quit=True)
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
        receipt = {"schema_version": 1, "ar": "AR-1654", "classification": "development/mock", "credentials": "none", "network": {"install": "local_bundle", "benchmark_and_replay": "denied"}, "terminal": {"controlling": True, "foreground_process_group": True, "rows": PTY_ROWS, "columns": PTY_COLUMNS, "quit": "q", "timeout_seconds": 60}, "operator_commands": {"install": "asb tui install", "launch": "asb tui", "install_json": json.loads(install_json[1]), "launch_json": launch_value, "install_human_nonempty": bool(install_human[1]), "launch_human_nonempty": bool(launch_human[1])}, "journey": journey, "provenance": {"asb_binary_sha256": hashlib.sha256(asb.read_bytes()).hexdigest(), "tui_binary_sha256": hashlib.sha256(tui.read_bytes()).hexdigest(), "asb_checkout_head": asb_commit, "asb_checkout_tree": asb_tree, "tui_checkout_head": tui_commit, "tui_checkout_tree": tui_tree}}
    args.receipt.parent.mkdir(parents=True, exist_ok=True)
    args.receipt.write_text(json.dumps(receipt, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(receipt, sort_keys=True) if args.json else "AR-1654 operator quickstart passed (development/mock; credentials absent)")
    return 0

if __name__ == "__main__":
    raise SystemExit(main())
