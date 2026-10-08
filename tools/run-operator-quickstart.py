#!/usr/bin/env python3
# Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Qualify the operator journey through the public ASB command boundary."""
from __future__ import annotations
import argparse
import ctypes
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
from typing import Callable, NamedTuple

SECRET_MARKERS = ("API_KEY", "APIKEY", "ACCESS_TOKEN", "AUTH_TOKEN", "CREDENTIAL", "PASSWORD", "PRIVATE_KEY", "SECRET", "TOKEN")
PTY_ROWS = 24
PTY_COLUMNS = 80
MAX_COMMAND_OUTPUT = 2 * 1024 * 1024
PTY_READINESS_TIMEOUT = 10.0
PTY_EXIT_TIMEOUT = 10.0
PR_SET_CHILD_SUBREAPER = 36
PR_GET_CHILD_SUBREAPER = 37

def safe_environment() -> dict[str, str]:
    environment = {k: v for k, v in os.environ.items() if not any(m in k.upper() for m in SECRET_MARKERS)}
    environment.update({"ASB_TUI_NETWORK_POLICY": "deny", "HTTP_PROXY": "http://127.0.0.1:1", "HTTPS_PROXY": "http://127.0.0.1:1", "ALL_PROXY": "http://127.0.0.1:1", "NO_PROXY": "*"})
    return environment

def _get_child_subreaper() -> bool:
    libc = ctypes.CDLL(None, use_errno=True)
    previous = ctypes.c_int()
    if libc.prctl(PR_GET_CHILD_SUBREAPER, ctypes.byref(previous), 0, 0, 0) != 0:
        error = ctypes.get_errno()
        raise OSError(error, os.strerror(error))
    return bool(previous.value)


def _child_subreaper(enable: bool) -> bool:
    """Set Linux child-subreaper state and return its previous value."""
    previous = _get_child_subreaper()
    if previous != enable:
        libc = ctypes.CDLL(None, use_errno=True)
        if libc.prctl(PR_SET_CHILD_SUBREAPER, int(enable), 0, 0, 0) != 0:
            error = ctypes.get_errno()
            raise OSError(error, os.strerror(error))
    return previous


class _ProcessIdentity(NamedTuple):
    pid: int
    session_id: int
    start_time: int
    pidfd: int


def _read_process_identity(pid: int) -> tuple[int, int]:
    """Return (session, start time) from one procfs identity snapshot."""
    value = Path(f"/proc/{pid}/stat").read_text(encoding="ascii")
    fields = value[value.rfind(")") + 2 :].split()
    return int(fields[3]), int(fields[19])


def _open_process_identity(pid: int, expected_session: int) -> _ProcessIdentity | None:
    """Pin a matching process identity before it can be signalled."""
    pidfd: int | None = None
    try:
        session_id, start_time = _read_process_identity(pid)
        if session_id != expected_session:
            return None
        pidfd = os.pidfd_open(pid)
        confirmed_session, confirmed_start = _read_process_identity(pid)
        if (confirmed_session, confirmed_start) != (session_id, start_time):
            os.close(pidfd)
            return None
        return _ProcessIdentity(pid, session_id, start_time, pidfd)
    except (FileNotFoundError, ProcessLookupError):
        if pidfd is not None:
            os.close(pidfd)
        return None


def _identity_alive(identity: _ProcessIdentity) -> bool:
    try:
        signal.pidfd_send_signal(identity.pidfd, 0)
        return True
    except ProcessLookupError:
        return False


def _signal_identity(identity: _ProcessIdentity, signum: int) -> None:
    try:
        signal.pidfd_send_signal(identity.pidfd, signum)
    except ProcessLookupError:
        pass


def _reap_identity(identity: _ProcessIdentity) -> None:
    try:
        os.waitpid(identity.pid, os.WNOHANG)
    except ChildProcessError:
        pass


def _discover_session_members(
    session_id: int, tracked: dict[int, _ProcessIdentity]
) -> None:
    """Discover only members of the spawn-created authenticated session."""
    for entry in Path("/proc").iterdir():
        if not entry.name.isdecimal():
            continue
        pid = int(entry.name)
        if pid in tracked:
            continue
        identity = _open_process_identity(pid, session_id)
        if identity is not None:
            tracked[pid] = identity


def _session_alive(
    session_id: int, tracked: dict[int, _ProcessIdentity]
) -> bool:
    for observation in range(2):
        _discover_session_members(session_id, tracked)
        for identity in tracked.values():
            _reap_identity(identity)
        if any(_identity_alive(identity) for identity in tracked.values()):
            return True
        if observation == 0:
            # A just-terminated parent can expose an adopted child after the
            # procfs directory sweep that observed the parent.  Require a
            # second quiescent observation before declaring the session gone.
            time.sleep(0.01)
    return False


def _terminate_session(leader: _ProcessIdentity) -> None:
    """Terminate and reap every process in the authenticated spawned session.

    A pidfd pins each discovered process identity.  Signals are never sent to a
    bare numeric PID or PGID, so concurrent PID/PGID reuse cannot redirect
    teardown at an unrelated process.
    """
    tracked = {leader.pid: leader}
    try:
        _discover_session_members(leader.session_id, tracked)
        for identity in tracked.values():
            _signal_identity(identity, signal.SIGTERM)
        term_deadline = time.monotonic() + 0.5
        while time.monotonic() < term_deadline:
            known = set(tracked)
            if not _session_alive(leader.session_id, tracked):
                return
            for pid in set(tracked) - known:
                _signal_identity(tracked[pid], signal.SIGTERM)
            time.sleep(0.01)

        kill_deadline = time.monotonic() + 1.0
        while time.monotonic() < kill_deadline:
            known = set(tracked)
            _discover_session_members(leader.session_id, tracked)
            for pid, identity in tracked.items():
                if pid not in known or _identity_alive(identity):
                    _signal_identity(identity, signal.SIGKILL)
            if not _session_alive(leader.session_id, tracked):
                return
            time.sleep(0.01)
        if _session_alive(leader.session_id, tracked):
            raise AssertionError("authenticated fixture session survived SIGKILL")
    finally:
        for identity in tracked.values():
            _reap_identity(identity)
            os.close(identity.pidfd)


def _wait_nohang(pid: int) -> int | None:
    try:
        waited, status = os.waitpid(pid, os.WNOHANG)
    except ChildProcessError:
        return None
    return None if waited == 0 else os.waitstatus_to_exitcode(status)


def _close_fd(fd: int | None) -> None:
    if fd is not None:
        try:
            os.close(fd)
        except OSError as error:
            if error.errno != errno.EBADF:
                raise


class _OwnedFd:
    """An idempotently releasable descriptor acquired by this runner."""

    def __init__(self, fd: int):
        self._fd: int | None = fd

    @property
    def fd(self) -> int:
        if self._fd is None:
            raise RuntimeError("file descriptor ownership was already released")
        return self._fd

    def close(self) -> None:
        fd, self._fd = self._fd, None
        _close_fd(fd)

    def detach(self) -> int:
        fd, self._fd = self.fd, None
        return fd


def _collect_cleanup_errors(
    actions: list[tuple[str, Callable[[], object]]],
) -> list[BaseException]:
    """Run every cleanup action and retain its exact failure and stage."""
    errors: list[BaseException] = []
    for stage, action in actions:
        try:
            action()
        except BaseException as error:
            error.add_note(f"cleanup stage: {stage}")
            errors.append(error)
    return errors


def _raise_operation_and_cleanup_errors(
    primary: BaseException | None, cleanup_errors: list[BaseException]
) -> None:
    """Keep the operation failure first while exposing every cleanup failure."""
    if primary is None and not cleanup_errors:
        return
    if primary is not None and not cleanup_errors:
        raise primary.with_traceback(primary.__traceback__)
    if primary is None and len(cleanup_errors) == 1:
        error = cleanup_errors[0]
        raise error.with_traceback(error.__traceback__)
    failures = ([primary] if primary is not None else []) + cleanup_errors
    raise BaseExceptionGroup(
        "operator quickstart failed and cleanup also reported errors",
        failures,
    )


def _pty_is_interactive_ready(master: int, session_id: int) -> bool:
    """Prove input will be delivered to a foreground raw-mode consumer."""
    try:
        foreground_group = os.tcgetpgrp(master)
        if foreground_group <= 0 or os.getsid(foreground_group) != session_id:
            return False
        local_flags = termios.tcgetattr(master)[3]
    except (OSError, ProcessLookupError):
        return False
    return not (local_flags & termios.ICANON) and not (local_flags & termios.ECHO)


def _spawn_controlling_pty(
    command: list[str], environment: dict[str, str]
) -> tuple[_ProcessIdentity, int]:
    master, slave = pty.openpty()
    owned_master = _OwnedFd(master)
    owned_slave = _OwnedFd(slave)
    leader: _ProcessIdentity | None = None
    pid: int | None = None
    primary: BaseException | None = None
    try:
        slave_name = os.ttyname(owned_slave.fd)
        requested = struct.pack("HHHH", PTY_ROWS, PTY_COLUMNS, 0, 0)
        fcntl.ioctl(owned_slave.fd, termios.TIOCSWINSZ, requested)
        observed = fcntl.ioctl(
            owned_slave.fd, termios.TIOCGWINSZ, b"\0" * len(requested)
        )
        rows, columns, _, _ = struct.unpack("HHHH", observed)
        if rows != PTY_ROWS or columns != PTY_COLUMNS:
            raise AssertionError(f"failed to establish {PTY_COLUMNS}x{PTY_ROWS} PTY")
        actions = [
            (os.POSIX_SPAWN_CLOSE, owned_master.fd),
            (os.POSIX_SPAWN_OPEN, 0, slave_name, os.O_RDWR, 0),
            (os.POSIX_SPAWN_DUP2, 0, 1),
            (os.POSIX_SPAWN_DUP2, 0, 2),
        ]
        pid = os.posix_spawn(
            command[0], command, environment, file_actions=actions, setsid=True
        )
        owned_slave.close()
        leader = _open_process_identity(pid, pid)
        if leader is None:
            raise AssertionError("could not authenticate PTY child session identity")
        if os.tcgetpgrp(owned_master.fd) != pid:
            raise AssertionError("PTY child is not its controlling foreground group")
        actual = fcntl.ioctl(
            owned_master.fd, termios.TIOCGWINSZ, b"\0" * len(requested)
        )
        rows, columns, _, _ = struct.unpack("HHHH", actual)
        if rows <= 0 or columns <= 0:
            raise AssertionError("PTY child received a zero-size terminal")
        os.set_blocking(owned_master.fd, False)
        return leader, owned_master.detach()
    except BaseException as error:
        primary = error
        if leader is None and pid is not None:
            leader = _open_process_identity(pid, pid)
    cleanup_actions: list[tuple[str, Callable[[], object]]] = []
    if leader is not None:
        cleanup_actions.append(
            ("authenticated child session", lambda: _terminate_session(leader))
        )
    cleanup_actions.extend(
        [
            ("PTY slave descriptor", owned_slave.close),
            ("PTY master descriptor", owned_master.close),
        ]
    )
    cleanup_errors = _collect_cleanup_errors(cleanup_actions)
    _raise_operation_and_cleanup_errors(primary, cleanup_errors)
    raise AssertionError("unreachable PTY spawn cleanup state")


def run_asb(
    asb: Path,
    arguments: list[str],
    environment: dict[str, str],
    *,
    json_output: bool,
    interactive_quit: bool = False,
    timeout_seconds: float = 60.0,
    output_limit: int = MAX_COMMAND_OUTPUT,
    readiness_timeout_seconds: float = PTY_READINESS_TIMEOUT,
    exit_timeout_seconds: float = PTY_EXIT_TIMEOUT,
) -> tuple[int, str]:
    command = [str(asb), *arguments]
    previous_subreaper = _child_subreaper(True)
    leader: _ProcessIdentity | None = None
    master: int | None = None
    selector: selectors.BaseSelector | None = None
    chunks = bytearray()
    result_code: int | None = None
    primary: BaseException | None = None
    try:
        leader, master = _spawn_controlling_pty(command, environment)
        selector = selectors.DefaultSelector()
        selector.register(master, selectors.EVENT_READ)
        deadline = time.monotonic() + timeout_seconds
        readiness_deadline = min(deadline, time.monotonic() + readiness_timeout_seconds)
        exit_deadline: float | None = None
        quit_sent = False
        while result_code is None:
            now = time.monotonic()
            if interactive_quit and not quit_sent and now >= readiness_deadline:
                raise AssertionError(f"ASB command did not become interactive-ready: {' '.join(command)}")
            if exit_deadline is not None and now >= exit_deadline:
                raise AssertionError(f"ASB command did not exit after quit: {' '.join(command)}")
            if now >= deadline:
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
            if interactive_quit and not quit_sent and _pty_is_interactive_ready(master, leader.session_id):
                try:
                    written = os.write(master, b"q")
                except BlockingIOError:
                    continue
                if written == 1:
                    quit_sent = True
                    exit_deadline = min(deadline, time.monotonic() + exit_timeout_seconds)
            result_code = _wait_nohang(leader.pid)
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
    except BaseException as error:
        primary = error
    cleanup_actions: list[tuple[str, Callable[[], object]]] = []
    if selector is not None:
        cleanup_actions.append(("selector", selector.close))
    if leader is not None:
        cleanup_actions.append(
            ("authenticated child session", lambda: _terminate_session(leader))
        )
    cleanup_actions.extend(
        [
            ("PTY master descriptor", lambda: _close_fd(master)),
            (
                "child-subreaper restoration",
                lambda: _child_subreaper(previous_subreaper),
            ),
        ]
    )
    cleanup_errors = _collect_cleanup_errors(cleanup_actions)
    _raise_operation_and_cleanup_errors(primary, cleanup_errors)
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
