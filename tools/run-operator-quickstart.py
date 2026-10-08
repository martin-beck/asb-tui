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
import socket
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
PTY_QUIT_SEQUENCE = b"qq"
SUPERVISOR_CONTROL_TIMEOUT = 5.0
SUPERVISOR_TERM_TIMEOUT = 0.5
SUPERVISOR_KILL_TIMEOUT = 1.0
PR_SET_CHILD_SUBREAPER = 36
PR_GET_CHILD_SUBREAPER = 37

def safe_environment() -> dict[str, str]:
    environment = {k: v for k, v in os.environ.items() if not any(m in k.upper() for m in SECRET_MARKERS)}
    # The qualification runner creates its own PTY, so its terminal capability
    # must not depend on whether the parent automation process happened to
    # inherit TERM.  xterm-256color is the conservative capability contract
    # used by the fixture; the runner does not claim a real terminal emulator.
    environment.update({"TERM": "xterm-256color", "ASB_TUI_NETWORK_POLICY": "deny", "HTTP_PROXY": "http://127.0.0.1:1", "HTTPS_PROXY": "http://127.0.0.1:1", "ALL_PROXY": "http://127.0.0.1:1", "NO_PROXY": "*"})
    return environment


def trailing_json(output: str) -> str | None:
    """Return the final complete JSON value after bounded terminal output.

    Alternate-screen teardown does not necessarily end with a newline, so the
    ASB response may immediately follow a CSI sequence.  Only a JSON object
    consuming the complete non-whitespace suffix is accepted.
    """
    decoder = json.JSONDecoder()
    offset = output.rfind("{")
    while offset >= 0:
        try:
            value, end = decoder.raw_decode(output, offset)
        except json.JSONDecodeError:
            pass
        else:
            if isinstance(value, dict) and not output[end:].strip():
                return json.dumps(value, sort_keys=True)
        offset = output.rfind("{", 0, offset)
    return None

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


class _ProcessSnapshot(NamedTuple):
    parent_pid: int
    session_id: int
    start_time: int


class _FixtureSupervisor(NamedTuple):
    pid: int
    control: socket.socket
    leader_pid: int
    session_id: int
    master: int


def _read_process_identity(pid: int) -> _ProcessSnapshot:
    """Return the ancestry and identity fields from one procfs snapshot."""
    value = Path(f"/proc/{pid}/stat").read_text(encoding="ascii")
    fields = value[value.rfind(")") + 2 :].split()
    return _ProcessSnapshot(int(fields[1]), int(fields[3]), int(fields[19]))


def _open_process_identity(pid: int, expected_session: int) -> _ProcessIdentity | None:
    """Pin a matching process identity before it can be signalled."""
    pidfd: int | None = None
    try:
        snapshot = _read_process_identity(pid)
        if snapshot.session_id != expected_session:
            return None
        pidfd = os.pidfd_open(pid)
        confirmed = _read_process_identity(pid)
        if confirmed != snapshot:
            os.close(pidfd)
            return None
        return _ProcessIdentity(
            pid, snapshot.session_id, snapshot.start_time, pidfd
        )
    except (FileNotFoundError, ProcessLookupError):
        if pidfd is not None:
            os.close(pidfd)
        return None


def _identity_matches_proc(identity: _ProcessIdentity) -> bool:
    """Check that a numeric PID still denotes the identity pinned by pidfd."""
    try:
        if _read_process_identity(identity.pid).start_time != identity.start_time:
            return False
    except (FileNotFoundError, ProcessLookupError):
        return False
    # Check the pinned identity after the numeric lookup.  If the original
    # process exited and its PID was reused during the lookup, its pidfd is no
    # longer signalable and the ancestry edge is rejected.
    return _identity_alive(identity)


def _open_descendant_identity(
    pid: int, parent: _ProcessIdentity
) -> _ProcessIdentity | None:
    """Pin a direct child while both sides of the ancestry edge are stable."""
    pidfd: int | None = None
    try:
        if not _identity_matches_proc(parent):
            return None
        snapshot = _read_process_identity(pid)
        if snapshot.parent_pid != parent.pid:
            return None
        pidfd = os.pidfd_open(pid)
        confirmed = _read_process_identity(pid)
        if confirmed != snapshot or not _identity_matches_proc(parent):
            os.close(pidfd)
            return None
        return _ProcessIdentity(
            pid, snapshot.session_id, snapshot.start_time, pidfd
        )
    except (FileNotFoundError, ProcessLookupError):
        if pidfd is not None:
            os.close(pidfd)
        return None


def _open_direct_child_identity(
    pid: int, parent_pid: int
) -> _ProcessIdentity | None:
    """Pin a direct child of this runner without trusting a bare PID."""
    pidfd: int | None = None
    try:
        snapshot = _read_process_identity(pid)
        if snapshot.parent_pid != parent_pid:
            return None
        pidfd = os.pidfd_open(pid)
        confirmed = _read_process_identity(pid)
        if confirmed != snapshot or confirmed.parent_pid != parent_pid:
            os.close(pidfd)
            return None
        return _ProcessIdentity(
            pid, snapshot.session_id, snapshot.start_time, pidfd
        )
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


MAX_LINEAGE_DEPTH = 64
MAX_LINEAGE_PROCESSES = 4096


def _discover_descendants(
    tracked: dict[int, _ProcessIdentity],
    boundary: _ProcessIdentity | None = None,
) -> None:
    """Pin descendants through stable parent edges, independent of sessions.

    Discovery starts only from the authenticated spawn leader and identities
    already proven to descend from it.  It never treats the runner's ambient
    children, a shared session, or a numeric PID/PGID as authority.
    """
    supervisor_pid = os.getpid() if boundary is None else boundary.pid
    process_limit_reached = False
    for depth in range(MAX_LINEAGE_DEPTH):
        added = False
        parents = dict(tracked)
        for entry in Path("/proc").iterdir():
            if not entry.name.isdecimal():
                continue
            pid = int(entry.name)
            if pid in tracked:
                continue
            try:
                parent_pid = _read_process_identity(pid).parent_pid
            except (FileNotFoundError, ProcessLookupError):
                continue
            parent = parents.get(parent_pid)
            if parent is not None:
                identity = _open_descendant_identity(pid, parent)
            elif parent_pid == supervisor_pid and (
                boundary is None or _identity_matches_proc(boundary)
            ):
                # This process is a fixture-exclusive subreaper: its only
                # possible children are the launch leader and descendants
                # adopted from that leader.  The structural boundary makes a
                # rapid double-fork authentic without sampling a parent edge.
                identity = _open_direct_child_identity(pid, supervisor_pid)
            else:
                identity = None
            if identity is None:
                continue
            if len(tracked) >= MAX_LINEAGE_PROCESSES:
                os.close(identity.pidfd)
                process_limit_reached = True
                continue
            tracked[pid] = identity
            added = True
        if process_limit_reached:
            raise AssertionError("authenticated fixture lineage exceeded process limit")
        if not added:
            return
        if depth + 1 == MAX_LINEAGE_DEPTH:
            raise AssertionError(
                f"authenticated fixture lineage exceeded depth limit {MAX_LINEAGE_DEPTH}"
            )


def _record_discovery_error(
    errors: list[BaseException], error: BaseException
) -> None:
    if not any(
        type(existing) is type(error) and str(existing) == str(error)
        for existing in errors
    ):
        error.add_note("cleanup stage: authenticated lineage discovery")
        errors.append(error)


def _discover_for_cleanup(
    tracked: dict[int, _ProcessIdentity],
    errors: list[BaseException],
    boundary: _ProcessIdentity | None = None,
) -> None:
    try:
        _discover_descendants(tracked, boundary)
    except BaseException as error:
        _record_discovery_error(errors, error)


def _retire_dead_identities(tracked: dict[int, _ProcessIdentity]) -> None:
    """Reap and release dead identities so bounded discovery can continue."""
    for pid, identity in list(tracked.items()):
        _reap_identity(identity)
        if not _identity_alive(identity):
            os.close(identity.pidfd)
            del tracked[pid]


def _lineage_alive(
    tracked: dict[int, _ProcessIdentity],
    errors: list[BaseException],
) -> bool:
    _discover_for_cleanup(tracked, errors)
    _retire_dead_identities(tracked)
    return bool(tracked)


def _terminate_session(
    leader: _ProcessIdentity,
    tracked: dict[int, _ProcessIdentity] | None = None,
) -> None:
    """Terminate and reap the authenticated spawned process lineage.

    Stable parent edges authenticate descendants even after setpgid or setsid.
    A pidfd pins each identity, and signals are never sent to a bare numeric
    PID or PGID, so reuse cannot redirect teardown at an unrelated process.
    """
    if tracked is None:
        tracked = {leader.pid: leader}
    elif tracked.get(leader.pid) != leader:
        raise AssertionError("authenticated fixture lineage lost its leader")
    cleanup_errors: list[BaseException] = []
    try:
        _discover_for_cleanup(tracked, cleanup_errors)
        for identity in tracked.values():
            _signal_identity(identity, signal.SIGTERM)
        term_deadline = time.monotonic() + 0.5
        while time.monotonic() < term_deadline:
            known = set(tracked)
            alive = _lineage_alive(tracked, cleanup_errors)
            for pid in set(tracked) - known:
                _signal_identity(tracked[pid], signal.SIGTERM)
            if not alive:
                time.sleep(0.01)
                if not _lineage_alive(tracked, cleanup_errors):
                    break
            time.sleep(0.01)

        if tracked:
            kill_deadline = time.monotonic() + 1.0
            while time.monotonic() < kill_deadline:
                _discover_for_cleanup(tracked, cleanup_errors)
                for identity in tracked.values():
                    _signal_identity(identity, signal.SIGKILL)
                if not _lineage_alive(tracked, cleanup_errors):
                    time.sleep(0.01)
                    if not _lineage_alive(tracked, cleanup_errors):
                        break
                time.sleep(0.01)
        if tracked:
            cleanup_errors.append(
                AssertionError("authenticated fixture lineage survived SIGKILL")
            )
    finally:
        for identity in list(tracked.values()):
            _reap_identity(identity)
            os.close(identity.pidfd)
        tracked.clear()
    _raise_operation_and_cleanup_errors(None, cleanup_errors)


def _wait_nohang(pid: int) -> int | None:
    try:
        waited, status = os.waitpid(pid, os.WNOHANG)
    except ChildProcessError:
        return None
    return None if waited == 0 else os.waitstatus_to_exitcode(status)


def _poll_child_reaped(pid: int, timeout_seconds: float) -> bool:
    """Boundedly poll one direct child without ever entering blocking waitpid."""
    deadline = time.monotonic() + timeout_seconds
    while True:
        try:
            waited, _ = os.waitpid(pid, os.WNOHANG)
        except ChildProcessError:
            return True
        if waited == pid:
            return True
        if time.monotonic() >= deadline:
            return False
        time.sleep(0.01)


def _poll_identity_stopped(
    identity: _ProcessIdentity, timeout_seconds: float
) -> bool:
    """Wait boundedly until a pinned process enters a stopped state."""
    deadline = time.monotonic() + timeout_seconds
    while _identity_matches_proc(identity):
        try:
            value = Path(f"/proc/{identity.pid}/stat").read_text(encoding="ascii")
        except (FileNotFoundError, ProcessLookupError):
            return False
        state = value[value.rfind(")") + 2 :].split()[0]
        if state in {"T", "t"}:
            return True
        if time.monotonic() >= deadline:
            return False
        time.sleep(0.01)
    return False


def _terminate_and_reap_supervisor(pid: int) -> None:
    """Boundedly empty a fixture-only boundary before destroying its supervisor.

    The supervisor is the only subreaper with authority over rapid daemonizing
    fixture descendants. Freeze and pin that boundary, terminate every child
    identity proven beneath it, and only then destroy the supervisor. This
    preserves lineage authority even when its control loop is stopped or
    otherwise unresponsive, without changing this parent's subreaper state.
    """
    if _poll_child_reaped(pid, 0.0):
        return
    supervisor = _open_direct_child_identity(pid, os.getpid())
    if supervisor is None:
        if _poll_child_reaped(pid, 0.0):
            return
        raise RuntimeError("could not authenticate fixture supervisor identity")
    tracked: dict[int, _ProcessIdentity] = {}
    cleanup_errors: list[BaseException] = []
    try:
        _signal_identity(supervisor, signal.SIGSTOP)
        if not _poll_identity_stopped(supervisor, SUPERVISOR_TERM_TIMEOUT):
            if _poll_child_reaped(pid, 0.0):
                return
            raise TimeoutError(
                "fixture supervisor did not stop before lineage cleanup"
            )
        _discover_for_cleanup(tracked, cleanup_errors, supervisor)
        term_deadline = time.monotonic() + SUPERVISOR_TERM_TIMEOUT
        while time.monotonic() < term_deadline:
            _discover_for_cleanup(tracked, cleanup_errors, supervisor)
            for identity in tracked.values():
                _signal_identity(identity, signal.SIGTERM)
            time.sleep(0.01)

        kill_deadline = time.monotonic() + SUPERVISOR_KILL_TIMEOUT
        while time.monotonic() < kill_deadline:
            known = set(tracked)
            _discover_for_cleanup(tracked, cleanup_errors, supervisor)
            for identity in tracked.values():
                _signal_identity(identity, signal.SIGKILL)
            # Require a quiet rescan before removing the fixture-only
            # subreaper. Dead children may remain as zombies until then.
            if known == set(tracked):
                time.sleep(0.01)
                _discover_for_cleanup(tracked, cleanup_errors, supervisor)
                if known == set(tracked):
                    break
            time.sleep(0.01)

        _signal_identity(supervisor, signal.SIGKILL)
        if not _poll_child_reaped(pid, SUPERVISOR_KILL_TIMEOUT):
            cleanup_errors.append(
                TimeoutError(
                    "fixture supervisor did not exit after bounded lineage cleanup"
                )
            )

        descendant_deadline = time.monotonic() + SUPERVISOR_KILL_TIMEOUT
        while tracked and time.monotonic() < descendant_deadline:
            for identity in tracked.values():
                _signal_identity(identity, signal.SIGKILL)
            _retire_dead_identities(tracked)
            if tracked:
                time.sleep(0.01)
        if tracked:
            cleanup_errors.append(
                AssertionError("authenticated fixture lineage survived SIGKILL")
            )
    finally:
        for identity in tracked.values():
            _reap_identity(identity)
            os.close(identity.pidfd)
        os.close(supervisor.pidfd)
    _raise_operation_and_cleanup_errors(None, cleanup_errors)


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
    command: list[str],
    environment: dict[str, str],
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
            (
                "authenticated child session",
                lambda: _terminate_session(leader),
            )
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


def _send_supervisor_packet(
    control: socket.socket, value: dict[str, object], fd: int | None = None
) -> None:
    ancillary = []
    if fd is not None:
        ancillary.append((socket.SOL_SOCKET, socket.SCM_RIGHTS, struct.pack("i", fd)))
    control.sendmsg([json.dumps(value).encode("utf-8")], ancillary)


def _receive_supervisor_packet(
    control: socket.socket,
) -> tuple[dict[str, object], int | None]:
    payload, ancillary, flags, _ = control.recvmsg(
        65536, socket.CMSG_SPACE(struct.calcsize("i"))
    )
    received_fds: list[int] = []
    try:
        for level, kind, data in ancillary:
            if level == socket.SOL_SOCKET and kind == socket.SCM_RIGHTS:
                if len(data) < struct.calcsize("i"):
                    raise RuntimeError(
                        "fixture supervisor sent invalid descriptor data"
                    )
                received_fds.append(
                    struct.unpack("i", data[: struct.calcsize("i")])[0]
                )
        if not payload:
            raise RuntimeError("fixture supervisor closed its control channel")
        if flags & socket.MSG_CTRUNC:
            raise RuntimeError("fixture supervisor sent truncated descriptor data")
        if len(received_fds) > 1:
            raise RuntimeError("fixture supervisor sent multiple descriptors")
        value = json.loads(payload.decode("utf-8"))
        if not isinstance(value, dict):
            raise RuntimeError("fixture supervisor sent an invalid control packet")
        return value, received_fds[0] if received_fds else None
    except BaseException:
        for received_fd in received_fds:
            _close_fd(received_fd)
        raise


def _fixture_supervisor_child(
    control: socket.socket, command: list[str], environment: dict[str, str]
) -> None:
    """Own the entire fixture lineage beneath an otherwise childless subreaper."""
    leader: _ProcessIdentity | None = None
    tracked: dict[int, _ProcessIdentity] = {}
    master: int | None = None
    discovery_errors: list[BaseException] = []
    try:
        _child_subreaper(True)
        leader, master = _spawn_controlling_pty(command, environment)
        tracked[leader.pid] = leader
        _send_supervisor_packet(
            control,
            {
                "kind": "ready",
                "leader_pid": leader.pid,
                "session_id": leader.session_id,
            },
            master,
        )
        _close_fd(master)
        master = None
        control.setblocking(False)
        exit_sent = False
        while True:
            _discover_for_cleanup(tracked, discovery_errors)
            if not exit_sent:
                result_code = _wait_nohang(leader.pid)
                if result_code is not None:
                    _send_supervisor_packet(
                        control, {"kind": "exit", "code": result_code}
                    )
                    exit_sent = True
            try:
                request, _ = _receive_supervisor_packet(control)
            except BlockingIOError:
                request = None
            except RuntimeError:
                request = {"kind": "cleanup"}
            if request is not None:
                if request.get("kind") != "cleanup":
                    raise RuntimeError("fixture supervisor received an invalid request")
                break
            time.sleep(0.01)

        cleanup_errors: list[BaseException] = []
        try:
            _terminate_session(leader, tracked)
        except BaseException as error:
            cleanup_errors.append(error)
        errors = [*discovery_errors, *cleanup_errors]
        _send_supervisor_packet(
            control,
            {"kind": "cleanup", "errors": [repr(error) for error in errors]},
        )
    except BaseException as error:
        try:
            if leader is not None:
                _terminate_session(leader, tracked)
        except BaseException as cleanup_error:
            error = BaseExceptionGroup(
                "fixture supervisor failed and cleanup also reported errors",
                [error, cleanup_error],
            )
        try:
            _send_supervisor_packet(control, {"kind": "error", "message": repr(error)})
        except BaseException:
            pass
    finally:
        _close_fd(master)
        control.close()


def _start_fixture_supervisor(
    command: list[str], environment: dict[str, str]
) -> _FixtureSupervisor:
    parent_control, child_control = socket.socketpair(
        socket.AF_UNIX, socket.SOCK_SEQPACKET
    )
    supervisor_pid = os.fork()
    if supervisor_pid == 0:
        parent_control.close()
        _fixture_supervisor_child(child_control, command, environment)
        os._exit(0)
    child_control.close()
    parent_control.settimeout(SUPERVISOR_CONTROL_TIMEOUT)
    owned_master: _OwnedFd | None = None
    try:
        packet, master = _receive_supervisor_packet(parent_control)
        if master is not None:
            owned_master = _OwnedFd(master)
        if packet.get("kind") != "ready" or master is None:
            raise RuntimeError(f"fixture supervisor startup failed: {packet}")
        os.set_blocking(owned_master.fd, False)
        parent_control.setblocking(False)
        return _FixtureSupervisor(
            supervisor_pid,
            parent_control,
            int(packet["leader_pid"]),
            int(packet["session_id"]),
            owned_master.detach(),
        )
    except BaseException as primary:
        cleanup_actions: list[tuple[str, Callable[[], object]]] = [
            ("supervisor control socket", parent_control.close),
        ]
        cleanup_actions.append(
            (
                "fixture supervisor process",
                lambda: _terminate_and_reap_supervisor(supervisor_pid),
            )
        )
        if owned_master is not None:
            cleanup_actions.append(
                ("received PTY master descriptor", owned_master.close)
            )
        _raise_operation_and_cleanup_errors(
            primary, _collect_cleanup_errors(cleanup_actions)
        )
        raise AssertionError("unreachable fixture supervisor startup cleanup state")


def _stop_fixture_supervisor(supervisor: _FixtureSupervisor) -> None:
    errors: list[str] = []
    primary: BaseException | None = None
    try:
        supervisor.control.settimeout(SUPERVISOR_CONTROL_TIMEOUT)
        _send_supervisor_packet(supervisor.control, {"kind": "cleanup"})
        while True:
            packet, received_fd = _receive_supervisor_packet(supervisor.control)
            _close_fd(received_fd)
            kind = packet.get("kind")
            if kind == "cleanup":
                errors.extend(str(value) for value in packet.get("errors", []))
                break
            if kind == "error":
                errors.append(str(packet.get("message")))
                break
    except BaseException as error:
        primary = error
    cleanup_errors = _collect_cleanup_errors(
        [
            ("supervisor control socket", supervisor.control.close),
            (
                "fixture supervisor process",
                lambda: _terminate_and_reap_supervisor(supervisor.pid),
            ),
        ]
    )
    _raise_operation_and_cleanup_errors(primary, cleanup_errors)
    if errors:
        raise AssertionError("; ".join(errors))


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
    supervisor: _FixtureSupervisor | None = None
    master: int | None = None
    selector: selectors.BaseSelector | None = None
    chunks = bytearray()
    result_code: int | None = None
    primary: BaseException | None = None
    try:
        supervisor = _start_fixture_supervisor(command, environment)
        master = supervisor.master
        selector = selectors.DefaultSelector()
        selector.register(master, selectors.EVENT_READ, "pty")
        selector.register(supervisor.control, selectors.EVENT_READ, "supervisor")
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
            for key, _ in selector.select(timeout=0.05):
                if key.data == "supervisor":
                    packet, received_fd = _receive_supervisor_packet(supervisor.control)
                    _close_fd(received_fd)
                    if packet.get("kind") == "exit":
                        result_code = int(packet["code"])
                    elif packet.get("kind") == "error":
                        raise AssertionError(
                            f"fixture supervisor failed: {packet.get('message')}"
                        )
                    continue
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
            if interactive_quit and not quit_sent and _pty_is_interactive_ready(master, supervisor.session_id):
                try:
                    # An unconfigured installation opens the first-run wizard.
                    # Its first q is the documented cancel-to-landing action;
                    # the second q is the landing-screen quit action.  A
                    # configured installation exits on the first byte and
                    # harmlessly leaves the second byte unread.
                    written = os.write(master, PTY_QUIT_SEQUENCE)
                except BlockingIOError:
                    continue
                if written == len(PTY_QUIT_SEQUENCE):
                    quit_sent = True
                    exit_deadline = min(deadline, time.monotonic() + exit_timeout_seconds)
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
    if supervisor is not None:
        cleanup_actions.append(
            (
                "authenticated child lineage",
                lambda: _stop_fixture_supervisor(supervisor),
            )
        )
    cleanup_actions.extend(
        [
            ("PTY master descriptor", lambda: _close_fd(master)),
        ]
    )
    cleanup_errors = _collect_cleanup_errors(cleanup_actions)
    _raise_operation_and_cleanup_errors(primary, cleanup_errors)
    output = chunks.decode("utf-8", "replace").strip()
    if not output:
        raise AssertionError(f"ASB command emitted no output: {' '.join(command)}")
    if json_output:
        encoded = trailing_json(output)
        if encoded is not None:
            return result_code, encoded
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
        receipt = {"schema_version": 1, "ar": "AR-1654", "classification": "development/mock", "credentials": "none", "network": {"install": "local_bundle", "benchmark_and_replay": "denied"}, "terminal": {"controlling": True, "foreground_process_group": True, "rows": PTY_ROWS, "columns": PTY_COLUMNS, "quit": "q; repeated once to cancel an automatically opened first-run wizard", "timeout_seconds": 60}, "operator_commands": {"install": "asb tui install", "launch": "asb tui", "install_json": json.loads(install_json[1]), "launch_json": launch_value, "install_human_nonempty": bool(install_human[1]), "launch_human_nonempty": bool(launch_human[1])}, "journey": journey, "provenance": {"asb_binary_sha256": hashlib.sha256(asb.read_bytes()).hexdigest(), "tui_binary_sha256": hashlib.sha256(tui.read_bytes()).hexdigest(), "asb_checkout_head": asb_commit, "asb_checkout_tree": asb_tree, "tui_checkout_head": tui_commit, "tui_checkout_tree": tui_tree}}
    args.receipt.parent.mkdir(parents=True, exist_ok=True)
    args.receipt.write_text(json.dumps(receipt, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(receipt, sort_keys=True) if args.json else "AR-1654 operator quickstart passed (development/mock; credentials absent)")
    return 0

if __name__ == "__main__":
    raise SystemExit(main())
