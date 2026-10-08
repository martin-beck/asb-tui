#!/usr/bin/env python3
# Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Contract tests for the public-boundary operator quickstart."""
import importlib.util
import json
import os
import signal
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path
from unittest import mock

RUNNER = Path(__file__).with_name("run-operator-quickstart.py")
SOURCE = RUNNER.read_text(encoding="utf-8")
SPEC = importlib.util.spec_from_file_location("operator_quickstart", RUNNER)
assert SPEC is not None and SPEC.loader is not None
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class OperatorQuickstartTests(unittest.TestCase):
    def test_starts_at_top_level_install_then_bare_launch(self):
        for value in ('["tui", "install", "--json"]', '["tui", "--json"]', '"install": "asb tui install"', '"launch": "asb tui"'):
            self.assertIn(value, SOURCE)

    def test_launch_and_provenance_are_strict(self):
        self.assertIn('launch_value.get("code") != "development_launched"', SOURCE)
        self.assertIn("verify_binary_identity", SOURCE)
        self.assertIn("asb_checkout_head", SOURCE)
        self.assertIn("tui_checkout_tree", SOURCE)

    def test_human_routes_check_exit_and_typed_outcome(self):
        self.assertIn("require_human(install_human", SOURCE)
        self.assertIn("require_human(launch_human", SOURCE)
        self.assertIn("code != 0", SOURCE)
    def test_human_json_and_downstream_journey(self):
        for value in ('["tui", "install"]', '["tui"]', 'cargo", "test"', '"install_human_nonempty"', '"launch_human_nonempty"'):
            self.assertIn(value, SOURCE)
    def test_fixture_is_credential_free_and_network_denied(self):
        for value in ('SECRET_MARKERS', '"ASB_TUI_NETWORK_POLICY": "deny"', '"NO_PROXY": "*"', '"credentials": "none"'):
            self.assertIn(value, SOURCE)

    def test_real_controlling_pty_has_foreground_group_size_and_bounded_quit(self):
        fixture = """
import fcntl, json, os, struct, sys, termios, tty
rows, columns, _, _ = struct.unpack('HHHH', fcntl.ioctl(0, termios.TIOCGWINSZ, b'\\0' * 8))
ready = {
    'session_leader': os.getsid(0) == os.getpid(),
    'foreground': os.tcgetpgrp(0) == os.getpgrp(),
    'nonzero_size': rows > 0 and columns > 0,
}
tty.setraw(0)
print('READY', flush=True)
ready['quit'] = os.read(0, 1) == b'q'
ready['ok'] = all(ready.values())
ready['code'] = 'development_launched'
print(json.dumps(ready), flush=True)
raise SystemExit(0 if ready['ok'] else 7)
"""
        code, encoded = MODULE.run_asb(
            Path(sys.executable),
            ["-c", fixture],
            os.environ.copy(),
            json_output=True,
            interactive_quit=True,
            timeout_seconds=5,
        )
        self.assertEqual(code, 0)
        result = json.loads(encoded)
        for key in ("session_leader", "foreground", "nonzero_size", "quit", "ok"):
            self.assertTrue(result[key], key)

    def test_quit_waits_for_foreground_raw_mode_not_early_output(self):
        fixture = """
import json, os, time, tty
print('BOOTING', flush=True)
time.sleep(0.15)
tty.setraw(0)
print('READY', flush=True)
quit_received = os.read(0, 1) == b'q'
print(json.dumps({'ok': quit_received, 'code': 'development_launched'}), flush=True)
raise SystemExit(0 if quit_received else 7)
"""
        code, encoded = MODULE.run_asb(
            Path(sys.executable),
            ["-c", fixture],
            os.environ.copy(),
            json_output=True,
            interactive_quit=True,
            timeout_seconds=5,
        )
        self.assertEqual(code, 0)
        self.assertTrue(json.loads(encoded)["ok"])

    def test_quit_accepts_authenticated_descendant_foreground_group(self):
        fixture = """
import json, os, signal, tty
child = os.fork()
if child == 0:
    os.setpgid(0, 0)
    signal.signal(signal.SIGTTOU, signal.SIG_IGN)
    os.tcsetpgrp(0, os.getpgrp())
    tty.setraw(0)
    quit_received = os.read(0, 1) == b'q'
    print(json.dumps({'ok': quit_received, 'code': 'development_launched'}), flush=True)
    os._exit(0 if quit_received else 7)
_, status = os.waitpid(child, 0)
raise SystemExit(os.waitstatus_to_exitcode(status))
"""
        code, encoded = MODULE.run_asb(
            Path(sys.executable),
            ["-c", fixture],
            os.environ.copy(),
            json_output=True,
            interactive_quit=True,
            timeout_seconds=5,
        )
        self.assertEqual(code, 0)
        self.assertTrue(json.loads(encoded)["ok"])

    def test_timeout_is_bounded_and_reaps_the_session_leader(self):
        with tempfile.TemporaryDirectory() as temporary:
            marker = Path(temporary) / "pid"
            fixture = (
                "import os, pathlib, time; "
                f"pathlib.Path({str(marker)!r}).write_text(str(os.getpid())); "
                "print('READY', flush=True); time.sleep(30)"
            )
            with self.assertRaisesRegex(AssertionError, "timed out"):
                MODULE.run_asb(
                    Path(sys.executable),
                    ["-c", fixture],
                    os.environ.copy(),
                    json_output=False,
                    timeout_seconds=0.1,
                )
            pid = int(marker.read_text(encoding="utf-8"))
            with self.assertRaises(ProcessLookupError):
                os.kill(pid, 0)

    def test_timeout_kills_and_reaps_term_resistant_same_group_descendant(self):
        with tempfile.TemporaryDirectory() as temporary:
            marker = Path(temporary) / "descendant-pid"
            child = """
import os, signal, time
signal.signal(signal.SIGTERM, signal.SIG_IGN)
signal.signal(signal.SIGHUP, signal.SIG_IGN)
open(os.environ['DESCENDANT_PID_FILE'], 'w').write(str(os.getpid()))
print('READY', flush=True)
while True:
    time.sleep(1)
"""
            leader = (
                "import os, subprocess, time; "
                f"os.environ['DESCENDANT_PID_FILE'] = {str(marker)!r}; "
                f"subprocess.Popen([{sys.executable!r}, '-c', {child!r}]); "
                "time.sleep(30)"
            )
            descendant_pid = None
            try:
                with self.assertRaisesRegex(AssertionError, "timed out"):
                    MODULE.run_asb(
                        Path(sys.executable),
                        ["-c", leader],
                        os.environ.copy(),
                        json_output=False,
                        timeout_seconds=0.2,
                    )
                deadline = time.monotonic() + 2
                while not marker.exists() and time.monotonic() < deadline:
                    time.sleep(0.01)
                descendant_pid = int(marker.read_text(encoding="utf-8"))
                with self.assertRaises(ProcessLookupError):
                    os.kill(descendant_pid, 0)
            finally:
                if descendant_pid is not None:
                    try:
                        os.kill(descendant_pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass

    def test_timeout_kills_term_resistant_descendant_in_separate_group(self):
        with tempfile.TemporaryDirectory() as temporary:
            marker = Path(temporary) / "descendant-pid"
            child = """
import os, signal, time
os.setpgid(0, 0)
signal.signal(signal.SIGTERM, signal.SIG_IGN)
signal.signal(signal.SIGHUP, signal.SIG_IGN)
open(os.environ["DESCENDANT_PID_FILE"], "w").write(str(os.getpid()))
while True:
    time.sleep(1)
"""
            leader = (
                "import os, subprocess, time; "
                f"os.environ[\"DESCENDANT_PID_FILE\"] = {str(marker)!r}; "
                f"subprocess.Popen([{sys.executable!r}, \"-c\", {child!r}]); "
                "time.sleep(30)"
            )
            descendant_pid = None
            try:
                with self.assertRaisesRegex(AssertionError, "timed out"):
                    MODULE.run_asb(
                        Path(sys.executable),
                        ["-c", leader],
                        os.environ.copy(),
                        json_output=False,
                        timeout_seconds=0.2,
                    )
                descendant_pid = int(marker.read_text(encoding="utf-8"))
                with self.assertRaises(ProcessLookupError):
                    os.kill(descendant_pid, 0)
            finally:
                if descendant_pid is not None:
                    try:
                        os.kill(descendant_pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass

    def test_timeout_reaps_resistant_setsid_lineage_without_ambient_capture(self):
        with tempfile.TemporaryDirectory() as temporary:
            marker = Path(temporary) / "setsid-descendant-pid"
            child = """
import os, signal, time
os.setsid()
signal.signal(signal.SIGTERM, signal.SIG_IGN)
signal.signal(signal.SIGHUP, signal.SIG_IGN)
open(os.environ["DESCENDANT_PID_FILE"], "w").write(str(os.getpid()))
while True:
    time.sleep(1)
"""
            leader = """
import os, pathlib, subprocess, sys, time
subprocess.Popen([sys.executable, "-c", os.environ["DESCENDANT_SOURCE"]])
marker = pathlib.Path(os.environ["DESCENDANT_PID_FILE"])
deadline = time.monotonic() + 2
while not marker.exists() and time.monotonic() < deadline:
    time.sleep(0.01)
print("READY", flush=True)
time.sleep(30)
"""
            environment = os.environ.copy()
            environment["DESCENDANT_PID_FILE"] = str(marker)
            environment["DESCENDANT_SOURCE"] = child
            real_start = MODULE._start_fixture_supervisor
            real_close = MODULE.os.close
            master_fd = None
            master_close_count = 0
            descendant_pid = None
            ambient = subprocess.Popen(
                [sys.executable, "-c", "import time; time.sleep(30)"]
            )

            def tracked_start(*args, **kwargs):
                nonlocal master_fd
                supervisor = real_start(*args, **kwargs)
                master_fd = supervisor.master
                return supervisor

            def tracked_close(fd):
                nonlocal master_close_count
                if fd == master_fd:
                    master_close_count += 1
                return real_close(fd)

            original_subreaper = MODULE._get_child_subreaper()
            try:
                for initial_subreaper in (False, True):
                    with self.subTest(initial_subreaper=initial_subreaper):
                        marker.unlink(missing_ok=True)
                        master_fd = None
                        master_close_count = 0
                        MODULE._child_subreaper(initial_subreaper)
                        with mock.patch.object(
                            MODULE, "_start_fixture_supervisor", tracked_start
                        ), mock.patch.object(MODULE.os, "close", tracked_close):
                            with self.assertRaisesRegex(AssertionError, "timed out"):
                                MODULE.run_asb(
                                    Path(sys.executable),
                                    ["-c", leader],
                                    environment,
                                    json_output=False,
                                    timeout_seconds=0.3,
                                )
                        descendant_pid = int(marker.read_text(encoding="utf-8"))
                        with self.assertRaises(ProcessLookupError):
                            os.kill(descendant_pid, 0)
                        self.assertIsNone(
                            ambient.poll(), "ambient child was captured by fixture teardown"
                        )
                        self.assertEqual(
                            MODULE._get_child_subreaper(), initial_subreaper
                        )
                        self.assertIsNotNone(master_fd)
                        self.assertEqual(master_close_count, 1)
                        with self.assertRaises(OSError):
                            os.fstat(master_fd)
            finally:
                MODULE._child_subreaper(original_subreaper)
                ambient.kill()
                ambient.wait()
                if descendant_pid is not None:
                    try:
                        os.kill(descendant_pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass

    def test_timeout_reaps_immediate_double_fork_setsid_adoption(self):
        with tempfile.TemporaryDirectory() as temporary:
            marker = Path(temporary) / "daemon-pid"
            fixture = """
import os, pathlib, signal, time
intermediate = os.fork()
if intermediate == 0:
    daemon = os.fork()
    if daemon != 0:
        os._exit(0)
    os.setsid()
    signal.signal(signal.SIGTERM, signal.SIG_IGN)
    signal.signal(signal.SIGHUP, signal.SIG_IGN)
    pathlib.Path(os.environ["DAEMON_PID_FILE"]).write_text(str(os.getpid()))
    while True:
        time.sleep(1)
os.waitpid(intermediate, 0)
deadline = time.monotonic() + 2
while not pathlib.Path(os.environ["DAEMON_PID_FILE"]).exists() and time.monotonic() < deadline:
    time.sleep(0.001)
time.sleep(30)
"""
            environment = os.environ.copy()
            environment["DAEMON_PID_FILE"] = str(marker)
            original_subreaper = MODULE._get_child_subreaper()
            daemon_pid = None
            ambient = subprocess.Popen(
                [sys.executable, "-c", "import time; time.sleep(30)"]
            )
            try:
                for initial_subreaper in (False, True):
                    with self.subTest(initial_subreaper=initial_subreaper):
                        marker.unlink(missing_ok=True)
                        MODULE._child_subreaper(initial_subreaper)
                        with self.assertRaisesRegex(AssertionError, "timed out"):
                            MODULE.run_asb(
                                Path(sys.executable),
                                ["-c", fixture],
                                environment,
                                json_output=False,
                                timeout_seconds=0.2,
                            )
                        daemon_pid = int(marker.read_text(encoding="utf-8"))
                        with self.assertRaises(ProcessLookupError):
                            os.kill(daemon_pid, 0)
                        self.assertIsNone(ambient.poll())
                        self.assertEqual(
                            MODULE._get_child_subreaper(), initial_subreaper
                        )
            finally:
                MODULE._child_subreaper(original_subreaper)
                ambient.kill()
                ambient.wait()
                if daemon_pid is not None:
                    try:
                        os.kill(daemon_pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass

    def test_ambient_post_boundary_double_fork_survives_fixture_cleanup(self):
        fixture = """
import os, pathlib, signal, time
intermediate = os.fork()
if intermediate == 0:
    daemon = os.fork()
    if daemon != 0:
        os._exit(0)
    os.setsid()
    signal.signal(signal.SIGTERM, signal.SIG_IGN)
    signal.signal(signal.SIGHUP, signal.SIG_IGN)
    pathlib.Path(os.environ["FIXTURE_DAEMON_FILE"]).write_text(str(os.getpid()))
    while True:
        time.sleep(1)
os.waitpid(intermediate, 0)
while not pathlib.Path(os.environ["FIXTURE_DAEMON_FILE"]).exists():
    time.sleep(0.001)
time.sleep(30)
"""
        ambient_source = """
import os, pathlib, signal, time
os.read(int(os.environ["RELEASE_FD"]), 1)
intermediate = os.fork()
if intermediate == 0:
    daemon = os.fork()
    if daemon != 0:
        os._exit(0)
    os.setsid()
    signal.signal(signal.SIGTERM, signal.SIG_IGN)
    signal.signal(signal.SIGHUP, signal.SIG_IGN)
    pathlib.Path(os.environ["AMBIENT_DAEMON_FILE"]).write_text(str(os.getpid()))
    while True:
        time.sleep(1)
os.waitpid(intermediate, 0)
"""
        original_subreaper = MODULE._get_child_subreaper()
        try:
            for initial_subreaper in (False, True):
                with self.subTest(
                    initial_subreaper=initial_subreaper
                ), tempfile.TemporaryDirectory() as temporary:
                    root = Path(temporary)
                    ambient_marker = root / "ambient"
                    fixture_marker = root / "fixture"
                    release_read, release_write = os.pipe()
                    MODULE._child_subreaper(initial_subreaper)
                    ambient_environment = os.environ.copy()
                    ambient_environment.update(
                        {
                            "RELEASE_FD": str(release_read),
                            "AMBIENT_DAEMON_FILE": str(ambient_marker),
                        }
                    )
                    ambient = subprocess.Popen(
                        [sys.executable, "-c", ambient_source],
                        env=ambient_environment,
                        pass_fds=(release_read,),
                    )
                    os.close(release_read)
                    fixture_environment = os.environ.copy()
                    fixture_environment["FIXTURE_DAEMON_FILE"] = str(
                        fixture_marker
                    )
                    real_start = MODULE._start_fixture_supervisor
                    ambient_pid = None
                    fixture_pid = None

                    def release_after_boundary(*args, **kwargs):
                        supervisor = real_start(*args, **kwargs)
                        os.write(release_write, b"x")
                        deadline = time.monotonic() + 2
                        while (
                            not ambient_marker.exists()
                            and time.monotonic() < deadline
                        ):
                            time.sleep(0.001)
                        self.assertTrue(ambient_marker.exists())
                        return supervisor

                    try:
                        with mock.patch.object(
                            MODULE, "_start_fixture_supervisor", release_after_boundary
                        ):
                            with self.assertRaisesRegex(AssertionError, "timed out"):
                                MODULE.run_asb(
                                    Path(sys.executable),
                                    ["-c", fixture],
                                    fixture_environment,
                                    json_output=False,
                                    timeout_seconds=0.3,
                                )
                        ambient_pid = int(ambient_marker.read_text())
                        fixture_pid = int(fixture_marker.read_text())
                        os.kill(ambient_pid, 0)
                        with self.assertRaises(ProcessLookupError):
                            os.kill(fixture_pid, 0)
                        self.assertEqual(
                            MODULE._get_child_subreaper(), initial_subreaper
                        )
                    finally:
                        os.close(release_write)
                        ambient.wait(timeout=2)
                        for pid in (ambient_pid, fixture_pid):
                            if pid is not None:
                                try:
                                    os.kill(pid, signal.SIGKILL)
                                except ProcessLookupError:
                                    pass
        finally:
            MODULE._child_subreaper(original_subreaper)

    def test_depth_limit_still_cleans_authenticated_lineage(self):
        self._assert_discovery_limit_is_clean("depth")

    def test_process_limit_still_cleans_authenticated_lineage(self):
        self._assert_discovery_limit_is_clean("process")

    def _assert_discovery_limit_is_clean(self, limit: str):
        with tempfile.TemporaryDirectory() as temporary:
            marker_dir = Path(temporary)
            fixture = """
import os, pathlib, signal, time
root = pathlib.Path(os.environ["LINEAGE_MARKER_DIR"])
signal.signal(signal.SIGTERM, signal.SIG_IGN)
signal.signal(signal.SIGHUP, signal.SIG_IGN)
root.joinpath("leader").write_text(str(os.getpid()))
child = os.fork()
if child == 0:
    signal.signal(signal.SIGTERM, signal.SIG_IGN)
    signal.signal(signal.SIGHUP, signal.SIG_IGN)
    root.joinpath("child").write_text(str(os.getpid()))
    grandchild = os.fork()
    if grandchild == 0:
        signal.signal(signal.SIGTERM, signal.SIG_IGN)
        signal.signal(signal.SIGHUP, signal.SIG_IGN)
        root.joinpath("grandchild").write_text(str(os.getpid()))
        while True:
            time.sleep(1)
    while not root.joinpath("grandchild").exists():
        time.sleep(0.001)
    while True:
        time.sleep(1)
while not root.joinpath("grandchild").exists():
    time.sleep(0.001)
time.sleep(30)
"""
            environment = os.environ.copy()
            environment["LINEAGE_MARKER_DIR"] = str(marker_dir)
            real_start = MODULE._start_fixture_supervisor
            real_close = MODULE.os.close
            master_fd = None
            master_close_count = 0
            observed_pids = []

            def tracked_start(*args, **kwargs):
                nonlocal master_fd
                supervisor = real_start(*args, **kwargs)
                master_fd = supervisor.master
                return supervisor

            def tracked_close(fd):
                nonlocal master_close_count
                if fd == master_fd:
                    master_close_count += 1
                return real_close(fd)

            original_subreaper = MODULE._get_child_subreaper()
            try:
                for initial_subreaper in (False, True):
                    with self.subTest(
                        limit=limit, initial_subreaper=initial_subreaper
                    ):
                        for name in ("leader", "child", "grandchild"):
                            marker_dir.joinpath(name).unlink(missing_ok=True)
                        master_fd = None
                        master_close_count = 0
                        observed_pids.clear()
                        MODULE._child_subreaper(initial_subreaper)
                        limit_patch = (
                            mock.patch.object(MODULE, "MAX_LINEAGE_DEPTH", 1)
                            if limit == "depth"
                            else mock.patch.object(
                                MODULE, "MAX_LINEAGE_PROCESSES", 2
                            )
                        )
                        with limit_patch, mock.patch.object(
                            MODULE, "_start_fixture_supervisor", tracked_start
                        ), mock.patch.object(MODULE.os, "close", tracked_close):
                            with self.assertRaises(BaseException) as caught:
                                MODULE.run_asb(
                                    Path(sys.executable),
                                    ["-c", fixture],
                                    environment,
                                    json_output=False,
                                    timeout_seconds=1,
                                )
                        self.assertIn(
                            f"exceeded {limit} limit", repr(caught.exception)
                        )
                        for name in ("leader", "child", "grandchild"):
                            observed_pids.append(
                                int(marker_dir.joinpath(name).read_text())
                            )
                        for pid in observed_pids:
                            with self.assertRaises(ProcessLookupError):
                                os.kill(pid, 0)
                        self.assertEqual(
                            MODULE._get_child_subreaper(), initial_subreaper
                        )
                        self.assertEqual(master_close_count, 1)
                        self.assertIsNotNone(master_fd)
                        with self.assertRaises(OSError):
                            os.fstat(master_fd)
            finally:
                MODULE._child_subreaper(original_subreaper)
                for pid in observed_pids:
                    try:
                        os.kill(pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass

    def test_selector_construction_failure_restores_every_resource(self):
        self._assert_selector_failure_is_clean("construct")

    def test_selector_registration_failure_restores_every_resource(self):
        self._assert_selector_failure_is_clean("register")

    def test_parent_set_blocking_failure_closes_received_master_once(self):
        fixture = "import time; print('READY', flush=True); time.sleep(30)"
        real_receive = MODULE._receive_supervisor_packet
        real_set_blocking = MODULE.os.set_blocking
        real_close = MODULE.os.close
        real_fork = MODULE.os.fork
        parent_pid = os.getpid()
        received_master = None
        supervisor_pid = None
        close_count = 0

        def tracked_fork():
            nonlocal supervisor_pid
            pid = real_fork()
            if os.getpid() == parent_pid:
                supervisor_pid = pid
            return pid

        def tracked_receive(control):
            nonlocal received_master
            packet, descriptor = real_receive(control)
            if os.getpid() == parent_pid and packet.get("kind") == "ready":
                received_master = descriptor
            return packet, descriptor

        def injected_set_blocking(fd, blocking):
            if os.getpid() == parent_pid and fd == received_master:
                raise OSError("injected parent set_blocking failure")
            return real_set_blocking(fd, blocking)

        def tracked_close(fd):
            nonlocal close_count
            if os.getpid() == parent_pid and fd == received_master:
                close_count += 1
            return real_close(fd)

        with mock.patch.object(
            MODULE, "_receive_supervisor_packet", tracked_receive
        ), mock.patch.object(
            MODULE.os, "fork", tracked_fork
        ), mock.patch.object(
            MODULE.os, "set_blocking", injected_set_blocking
        ), mock.patch.object(MODULE.os, "close", tracked_close):
            with self.assertRaisesRegex(OSError, "injected parent set_blocking"):
                MODULE._start_fixture_supervisor(
                    [sys.executable, "-c", fixture], os.environ.copy()
                )
        self.assertIsNotNone(received_master)
        self.assertEqual(close_count, 1)
        with self.assertRaises(OSError):
            os.fstat(received_master)
        self.assertIsNotNone(supervisor_pid)
        with self.assertRaises(ChildProcessError):
            os.waitpid(supervisor_pid, os.WNOHANG)

    def test_invalid_supervisor_packet_closes_received_descriptor_once(self):
        sender, receiver = MODULE.socket.socketpair(
            MODULE.socket.AF_UNIX, MODULE.socket.SOCK_SEQPACKET
        )
        read_fd, write_fd = os.pipe()
        received_close_count = 0
        pipe_inode = os.fstat(read_fd).st_ino
        real_close = MODULE.os.close
        try:
            sender.sendmsg(
                [b"not-json"],
                [
                    (
                        MODULE.socket.SOL_SOCKET,
                        MODULE.socket.SCM_RIGHTS,
                        MODULE.struct.pack("i", read_fd),
                    )
                ],
            )
            os.close(read_fd)
            read_fd = -1

            def tracked_close(fd):
                nonlocal received_close_count
                try:
                    if os.fstat(fd).st_ino == pipe_inode:
                        received_close_count += 1
                except OSError:
                    pass
                return real_close(fd)

            with mock.patch.object(MODULE.os, "close", tracked_close):
                with self.assertRaises(json.JSONDecodeError):
                    MODULE._receive_supervisor_packet(receiver)
            self.assertEqual(received_close_count, 1)
        finally:
            if read_fd >= 0:
                os.close(read_fd)
            os.close(write_fd)
            sender.close()
            receiver.close()

    def test_stopped_supervisor_start_and_stop_cleanup_are_bounded(self):
        fixture = "import time; print('READY', flush=True); time.sleep(30)"
        real_child = MODULE._fixture_supervisor_child
        real_fork = MODULE.os.fork
        parent_pid = os.getpid()
        startup_supervisor_pid = None

        def tracked_fork():
            nonlocal startup_supervisor_pid
            pid = real_fork()
            if os.getpid() == parent_pid:
                startup_supervisor_pid = pid
            return pid

        def stopped_before_startup(control, command, environment):
            os.kill(os.getpid(), signal.SIGSTOP)
            real_child(control, command, environment)

        with mock.patch.object(
            MODULE, "SUPERVISOR_CONTROL_TIMEOUT", 0.1
        ), mock.patch.object(
            MODULE, "SUPERVISOR_TERM_TIMEOUT", 0.1
        ), mock.patch.object(
            MODULE, "SUPERVISOR_KILL_TIMEOUT", 0.2
        ), mock.patch.object(
            MODULE, "_fixture_supervisor_child", stopped_before_startup
        ), mock.patch.object(
            MODULE.os, "fork", tracked_fork
        ):
            started = time.monotonic()
            with self.assertRaises((TimeoutError, BaseExceptionGroup)):
                MODULE._start_fixture_supervisor(
                    [sys.executable, "-c", fixture], os.environ.copy()
                )
            self.assertLess(time.monotonic() - started, 1.5)
        self.assertIsNotNone(startup_supervisor_pid)
        with self.assertRaises(ChildProcessError):
            os.waitpid(startup_supervisor_pid, os.WNOHANG)

        supervisor = MODULE._start_fixture_supervisor(
            [sys.executable, "-c", fixture], os.environ.copy()
        )
        try:
            os.kill(supervisor.pid, signal.SIGSTOP)
            started = time.monotonic()
            with mock.patch.object(
                MODULE, "SUPERVISOR_CONTROL_TIMEOUT", 0.1
            ), mock.patch.object(
                MODULE, "SUPERVISOR_TERM_TIMEOUT", 0.1
            ), mock.patch.object(MODULE, "SUPERVISOR_KILL_TIMEOUT", 0.2):
                with self.assertRaises((TimeoutError, BaseExceptionGroup)):
                    MODULE._stop_fixture_supervisor(supervisor)
            self.assertLess(time.monotonic() - started, 1.5)
            with self.assertRaises(ChildProcessError):
                os.waitpid(supervisor.pid, os.WNOHANG)
        finally:
            MODULE._close_fd(supervisor.master)
            try:
                os.kill(supervisor.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass

    def test_pre_spawn_ttyname_failure_closes_both_descriptors_once(self):
        self._assert_pre_spawn_failure_is_clean("ttyname")

    def test_pre_spawn_window_ioctl_failures_close_both_descriptors_once(self):
        for failure in ("set-window", "get-window"):
            with self.subTest(failure=failure):
                self._assert_pre_spawn_failure_is_clean(failure)

    def test_posix_spawn_failure_closes_both_descriptors_once(self):
        self._assert_pre_spawn_failure_is_clean("spawn")

    def _assert_pre_spawn_failure_is_clean(self, failure: str):
        real_openpty = MODULE.pty.openpty
        real_close = MODULE.os.close
        real_ttyname = MODULE.os.ttyname
        real_ioctl = MODULE.fcntl.ioctl
        real_posix_spawn = MODULE.os.posix_spawn
        acquired = []
        close_counts = {}
        spawn_calls = 0

        def tracked_openpty():
            descriptors = real_openpty()
            acquired[:] = descriptors
            close_counts.update({descriptor: 0 for descriptor in descriptors})
            return descriptors

        def tracked_close(fd):
            if fd in close_counts:
                close_counts[fd] += 1
            return real_close(fd)

        def injected_ttyname(fd):
            if failure == "ttyname":
                raise OSError("injected ttyname failure")
            return real_ttyname(fd)

        def injected_ioctl(fd, operation, argument=0, mutate_flag=True):
            if failure == "set-window" and operation == MODULE.termios.TIOCSWINSZ:
                raise OSError("injected window set failure")
            if failure == "get-window" and operation == MODULE.termios.TIOCGWINSZ:
                raise OSError("injected window get failure")
            return real_ioctl(fd, operation, argument, mutate_flag)

        def injected_posix_spawn(*args, **kwargs):
            nonlocal spawn_calls
            spawn_calls += 1
            if failure == "spawn":
                raise OSError("injected posix_spawn failure")
            return real_posix_spawn(*args, **kwargs)

        original_subreaper = MODULE._get_child_subreaper()
        try:
            for initial_subreaper in (False, True):
                acquired.clear()
                close_counts.clear()
                spawn_calls = 0
                MODULE._child_subreaper(initial_subreaper)
                with mock.patch.object(MODULE.pty, "openpty", tracked_openpty), mock.patch.object(
                    MODULE.os, "close", tracked_close
                ), mock.patch.object(
                    MODULE.os, "ttyname", injected_ttyname
                ), mock.patch.object(
                    MODULE.fcntl, "ioctl", injected_ioctl
                ), mock.patch.object(
                    MODULE.os, "posix_spawn", injected_posix_spawn
                ):
                    with self.assertRaisesRegex(OSError, "injected"):
                        MODULE._spawn_controlling_pty(
                            [sys.executable, "-c", "raise SystemExit(0)"],
                            os.environ.copy(),
                        )
                self.assertEqual(spawn_calls, 1 if failure == "spawn" else 0)
                self.assertEqual(MODULE._get_child_subreaper(), initial_subreaper)
                self.assertEqual(len(acquired), 2)
                for descriptor in acquired:
                    self.assertEqual(close_counts[descriptor], 1)
                    with self.assertRaises(OSError):
                        os.fstat(descriptor)
        finally:
            MODULE._child_subreaper(original_subreaper)

    def test_cleanup_errors_keep_primary_close_master_and_restore_state(self):
        with tempfile.TemporaryDirectory() as temporary:
            marker = Path(temporary) / "pid"
            fixture = (
                "import os, pathlib, time; "
                f"pathlib.Path({str(marker)!r}).write_text(str(os.getpid())); "
                "time.sleep(30)"
            )
            real_start = MODULE._start_fixture_supervisor
            real_stop = MODULE._stop_fixture_supervisor
            real_close = MODULE.os.close
            spawned_fd = None
            close_count = 0
            cleanup_failure = ""

            def tracked_start(*args, **kwargs):
                nonlocal spawned_fd
                supervisor = real_start(*args, **kwargs)
                spawned_fd = supervisor.master
                return supervisor

            def stop_then_report(supervisor):
                real_stop(supervisor)
                if cleanup_failure == "stop":
                    raise RuntimeError("injected teardown report")

            def tracked_close(fd):
                nonlocal close_count
                if fd == spawned_fd:
                    close_count += 1
                return real_close(fd)

            original_subreaper = MODULE._get_child_subreaper()
            try:
                for cleanup_failure in ("stop",):
                    for initial_subreaper in (False, True):
                        with self.subTest(
                            cleanup_failure=cleanup_failure,
                            initial_subreaper=initial_subreaper,
                        ):
                            spawned_fd = None
                            close_count = 0
                            MODULE._child_subreaper(initial_subreaper)
                            with mock.patch.object(
                                MODULE, "_start_fixture_supervisor", tracked_start
                            ), mock.patch.object(
                                MODULE, "_stop_fixture_supervisor", stop_then_report
                            ), mock.patch.object(MODULE.os, "close", tracked_close):
                                with self.assertRaises(BaseExceptionGroup) as caught:
                                    MODULE.run_asb(
                                        Path(sys.executable),
                                        ["-c", fixture],
                                        os.environ.copy(),
                                        json_output=False,
                                        timeout_seconds=0.2,
                                    )
                            self.assertEqual(len(caught.exception.exceptions), 2)
                            primary, cleanup = caught.exception.exceptions
                            self.assertIsInstance(primary, AssertionError)
                            self.assertIn("timed out", str(primary))
                            self.assertIsInstance(cleanup, RuntimeError)
                            self.assertIn("injected teardown report", str(cleanup))
                            stage = "cleanup stage: authenticated child lineage"
                            self.assertIn(stage, getattr(cleanup, "__notes__", []))
                            self.assertEqual(
                                MODULE._get_child_subreaper(), initial_subreaper
                            )
                            self.assertIsNotNone(spawned_fd)
                            self.assertEqual(close_count, 1)
                            with self.assertRaises(OSError):
                                os.fstat(spawned_fd)
                            pid = int(marker.read_text(encoding="utf-8"))
                            with self.assertRaises(ProcessLookupError):
                                os.kill(pid, 0)
                            marker.unlink()
            finally:
                MODULE._child_subreaper(original_subreaper)

    def _assert_selector_failure_is_clean(self, failure: str):
        with tempfile.TemporaryDirectory() as temporary:
            marker = Path(temporary) / "pid"
            fixture = (
                "import os, pathlib, time; "
                f"pathlib.Path({str(marker)!r}).write_text(str(os.getpid())); "
                "time.sleep(30)"
            )
            selector_fd = None
            spawned_fd = None
            real_start = MODULE._start_fixture_supervisor

            def tracked_start(*args, **kwargs):
                nonlocal spawned_fd
                supervisor = real_start(*args, **kwargs)
                spawned_fd = supervisor.master
                return supervisor

            class FailingSelector:
                def __init__(self):
                    if failure == "construct":
                        deadline = time.monotonic() + 2
                        while not marker.exists() and time.monotonic() < deadline:
                            time.sleep(0.01)
                        raise RuntimeError("injected selector construction failure")

                def register(self, fd, _events, _data=None):
                    nonlocal selector_fd
                    selector_fd = fd
                    deadline = time.monotonic() + 2
                    while not marker.exists() and time.monotonic() < deadline:
                        time.sleep(0.01)
                    raise RuntimeError("injected selector registration failure")

                def close(self):
                    pass

            original_subreaper = MODULE._get_child_subreaper()
            try:
                for initial_subreaper in (False, True):
                    selector_fd = None
                    spawned_fd = None
                    MODULE._child_subreaper(initial_subreaper)
                    with mock.patch.object(
                        MODULE, "_start_fixture_supervisor", tracked_start
                    ), mock.patch.object(
                        MODULE.selectors, "DefaultSelector", FailingSelector
                    ):
                        with self.assertRaisesRegex(RuntimeError, "injected selector"):
                            MODULE.run_asb(
                                Path(sys.executable),
                                ["-c", fixture],
                                os.environ.copy(),
                                json_output=False,
                            )
                    self.assertEqual(
                        MODULE._get_child_subreaper(), initial_subreaper
                    )
                    self.assertTrue(marker.exists(), "child did not start before injection")
                    pid = int(marker.read_text(encoding="utf-8"))
                    with self.assertRaises(ProcessLookupError):
                        os.kill(pid, 0)
                    marker.unlink()
                    self.assertIsNotNone(spawned_fd)
                    with self.assertRaises(OSError):
                        os.fstat(spawned_fd)
                    if failure == "register":
                        self.assertEqual(selector_fd, spawned_fd)
            finally:
                MODULE._child_subreaper(original_subreaper)

    def test_output_limit_and_missing_json_fail_closed(self):
        with self.assertRaisesRegex(AssertionError, "output limit"):
            MODULE.run_asb(
                Path(sys.executable),
                ["-c", "print('x' * 1024)"],
                os.environ.copy(),
                json_output=False,
                output_limit=32,
            )
        with self.assertRaisesRegex(AssertionError, "emitted no JSON"):
            MODULE.run_asb(
                Path(sys.executable),
                ["-c", "print('not-json')"],
                os.environ.copy(),
                json_output=True,
            )

    def test_fast_exit_json_is_drained_before_the_pty_closes(self):
        fixture = "import json; print(json.dumps({'ok': True, 'code': 'fast'}))"
        for _ in range(20):
            code, encoded = MODULE.run_asb(
                Path(sys.executable),
                ["-c", fixture],
                os.environ.copy(),
                json_output=True,
            )
            self.assertEqual(code, 0)
            self.assertEqual(json.loads(encoded)["code"], "fast")

    def test_runner_avoids_thread_unsafe_preexec(self):
        self.assertNotIn("preexec_fn", SOURCE)
        self.assertNotIn("killpg", SOURCE)
        self.assertIn("pidfd_send_signal", SOURCE)
        self.assertIn("setsid=True", SOURCE)
        self.assertIn("POSIX_SPAWN_OPEN", SOURCE)

    def test_receipt_records_terminal_contract_without_private_checkout_paths(self):
        self.assertIn('"foreground_process_group": True', SOURCE)
        self.assertIn('"rows": PTY_ROWS', SOURCE)
        self.assertNotIn('"asb_checkout": str', SOURCE)
        self.assertNotIn('"tui_checkout": str', SOURCE)


if __name__ == "__main__":
    unittest.main()
