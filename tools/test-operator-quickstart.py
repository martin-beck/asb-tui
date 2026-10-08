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
    @staticmethod
    def _after_child_exit(original, *, expose_identity):
        """Hold spawn observation until the direct child is a zombie."""
        def synchronized(pid, expected_session):
            deadline = time.monotonic() + 2
            while time.monotonic() < deadline:
                try:
                    value = Path(f"/proc/{pid}/stat").read_text(encoding="ascii")
                except FileNotFoundError:
                    break
                state = value[value.rfind(")") + 2 :].split()[0]
                if state == "Z":
                    return (
                        original(pid, expected_session)
                        if expose_identity
                        else None
                    )
                time.sleep(0.001)
            raise AssertionError("fixture child did not exit before observation")

        return synchronized

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

    def test_synthetic_pty_declares_a_deterministic_terminal_capability(self):
        with mock.patch.dict(os.environ, {}, clear=True):
            environment = MODULE.safe_environment()
        self.assertEqual(environment["TERM"], "xterm-256color")

    def test_parent_toolchain_candidates_use_original_home(self):
        cargo, rustup = MODULE.parent_toolchain_candidates(
            {"HOME": "/original/home"}
        )
        self.assertEqual(cargo, "/original/home/.cargo/bin/cargo")
        self.assertEqual(rustup, "/original/home/.rustup")

    def test_parent_toolchain_candidates_preserve_explicit_overrides(self):
        cargo, rustup = MODULE.parent_toolchain_candidates(
            {
                "HOME": "/ignored/home",
                "CARGO_HOME": "/ignored/cargo",
                "RUSTUP_HOME": "/ignored/rustup",
                "ASB_DEV_CARGO": "/validated/cargo",
                "ASB_DEV_RUSTUP_HOME": "/validated/rustup",
            }
        )
        self.assertEqual(cargo, "/validated/cargo")
        self.assertEqual(rustup, "/validated/rustup")

    def test_parent_toolchain_candidates_use_standard_home_overrides(self):
        cargo, rustup = MODULE.parent_toolchain_candidates(
            {
                "HOME": "/ignored/home",
                "CARGO_HOME": "/standard/cargo",
                "RUSTUP_HOME": "/standard/rustup",
            }
        )
        self.assertEqual(cargo, "/standard/cargo/bin/cargo")
        self.assertEqual(rustup, "/standard/rustup")

    def test_parent_toolchain_candidates_can_remain_missing(self):
        self.assertEqual(MODULE.parent_toolchain_candidates({}), (None, None))

    def test_candidate_resolution_does_not_bypass_asb_validation(self):
        cargo, rustup = MODULE.parent_toolchain_candidates(
            {
                "ASB_DEV_CARGO": "relative/cargo",
                "ASB_DEV_RUSTUP_HOME": "relative/rustup",
            }
        )
        self.assertEqual(cargo, "relative/cargo")
        self.assertEqual(rustup, "relative/rustup")

    def test_safe_environment_drops_secrets_but_keeps_tool_candidates(self):
        with mock.patch.dict(
            os.environ,
            {
                "OPENROUTER_API_KEY": "not-retained",
                "ASB_DEV_CARGO": "/candidate/cargo",
                "ASB_DEV_RUSTUP_HOME": "/candidate/rustup",
            },
            clear=True,
        ):
            environment = MODULE.safe_environment()
        self.assertNotIn("OPENROUTER_API_KEY", environment)
        self.assertEqual(environment["ASB_DEV_CARGO"], "/candidate/cargo")
        self.assertEqual(
            environment["ASB_DEV_RUSTUP_HOME"], "/candidate/rustup"
        )

    def test_isolated_environment_keeps_pre_isolation_candidates(self):
        with tempfile.TemporaryDirectory() as temporary, mock.patch.dict(
            os.environ,
            {"HOME": "/original/home"},
            clear=True,
        ):
            root = Path(temporary)
            environment = MODULE.qualification_environment(root)
        self.assertEqual(environment["HOME"], str(root / "home"))
        self.assertEqual(
            environment["ASB_DEV_CARGO"],
            "/original/home/.cargo/bin/cargo",
        )
        self.assertEqual(
            environment["ASB_DEV_RUSTUP_HOME"], "/original/home/.rustup"
        )
        self.assertEqual(
            environment["ASB_TUI_DEV_RUSTUP_HOME"], "/original/home/.rustup"
        )

    def test_isolated_environment_does_not_invent_missing_candidates(self):
        with tempfile.TemporaryDirectory() as temporary, mock.patch.dict(
            os.environ,
            {},
            clear=True,
        ):
            environment = MODULE.qualification_environment(Path(temporary))
        self.assertNotIn("ASB_DEV_CARGO", environment)
        self.assertNotIn("ASB_DEV_RUSTUP_HOME", environment)
        self.assertNotIn("ASB_TUI_DEV_RUSTUP_HOME", environment)

    def test_hostile_candidates_are_forwarded_for_asb_to_reject(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            writable_cargo = root / "cargo"
            writable_cargo.write_text("not cargo", encoding="utf-8")
            writable_cargo.chmod(0o777)
            symlinked_rustup = root / "rustup-link"
            symlinked_rustup.symlink_to(root / "rustup-target")
            with mock.patch.dict(
                os.environ,
                {
                    "ASB_DEV_CARGO": str(writable_cargo),
                    "ASB_DEV_RUSTUP_HOME": str(symlinked_rustup),
                },
                clear=True,
            ):
                environment = MODULE.qualification_environment(root / "fixture")
        self.assertEqual(environment["ASB_DEV_CARGO"], str(writable_cargo))
        self.assertEqual(
            environment["ASB_DEV_RUSTUP_HOME"], str(symlinked_rustup)
        )
        self.assertEqual(
            environment["ASB_TUI_DEV_RUSTUP_HOME"], str(symlinked_rustup)
        )

    def test_parent_rejection_requires_typed_failure(self):
        rejected = json.dumps(
            {"ok": False, "code": "trusted_tool_invalid"}
        )
        with mock.patch.object(MODULE, "run_asb", return_value=(3, rejected)):
            self.assertEqual(
                MODULE.require_toolchain_rejection(
                    Path("/fixture/asb"), {}, "hostile"
                ),
                "trusted_tool_invalid",
            )
        accepted = json.dumps({"ok": True, "code": "development_launched"})
        with mock.patch.object(MODULE, "run_asb", return_value=(0, accepted)):
            with self.assertRaisesRegex(AssertionError, "accepted hostile"):
                MODULE.require_toolchain_rejection(
                    Path("/fixture/asb"), {}, "hostile"
                )
        with mock.patch.object(MODULE, "run_asb", return_value=(7, rejected)):
            with self.assertRaisesRegex(AssertionError, "accepted hostile"):
                MODULE.require_toolchain_rejection(
                    Path("/fixture/asb"), {}, "hostile"
                )
        with mock.patch.object(MODULE, "run_asb", return_value=(4, rejected)):
            with self.assertRaisesRegex(AssertionError, "accepted hostile"):
                MODULE.require_toolchain_rejection(
                    Path("/fixture/asb"), {}, "hostile"
                )
        unavailable = json.dumps(
            {"ok": False, "code": "trusted_tool_unavailable"}
        )
        with mock.patch.object(MODULE, "run_asb", return_value=(4, unavailable)):
            self.assertEqual(
                MODULE.require_toolchain_rejection(
                    Path("/fixture/asb"), {}, "hostile"
                ),
                "trusted_tool_unavailable",
            )
        with mock.patch.object(MODULE, "run_asb", return_value=(3, unavailable)):
            with self.assertRaisesRegex(AssertionError, "accepted hostile"):
                MODULE.require_toolchain_rejection(
                    Path("/fixture/asb"), {}, "hostile"
                )

    def test_exact_parent_boundary_receives_all_hostile_scenarios(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            cargo = root / "cargo"
            cargo.write_bytes(b"cargo fixture")
            cargo.chmod(0o700)
            rustup = root / "rustup"
            rustup.mkdir(mode=0o700)
            observed = {}

            def reject(_asb, candidate, scenario):
                observed[scenario] = candidate
                return "trusted_tool_invalid"

            with mock.patch.object(
                MODULE, "require_toolchain_rejection", side_effect=reject
            ):
                result = MODULE.verify_parent_toolchain_rejections(
                    Path("/fixture/asb"),
                    {
                        "ASB_DEV_CARGO": str(cargo),
                        "ASB_DEV_RUSTUP_HOME": str(rustup),
                    },
                    root,
                )
            cargo_link_is_symlink = Path(
                observed["symlinked_cargo"]["ASB_DEV_CARGO"]
            ).is_symlink()
            writable_cargo_mode = (
                Path(observed["writable_cargo"]["ASB_DEV_CARGO"]).stat().st_mode
                & 0o777
            )
            rustup_link_is_symlink = Path(
                observed["symlinked_rustup"]["ASB_DEV_RUSTUP_HOME"]
            ).is_symlink()

        self.assertEqual(
            set(result),
            {
                "relative_candidate",
                "symlinked_cargo",
                "writable_cargo",
                "symlinked_rustup",
                "missing_candidate",
            },
        )
        self.assertEqual(set(observed), set(result))
        self.assertEqual(
            observed["relative_candidate"]["ASB_DEV_CARGO"], "relative/cargo"
        )
        self.assertTrue(cargo_link_is_symlink)
        self.assertEqual(writable_cargo_mode, 0o777)
        self.assertTrue(rustup_link_is_symlink)
        self.assertFalse(
            Path(observed["missing_candidate"]["ASB_DEV_CARGO"]).exists()
        )
        self.assertFalse(
            Path(observed["missing_candidate"]["ASB_DEV_RUSTUP_HOME"]).exists()
        )

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

    def test_quit_sequence_cancels_first_run_wizard_then_exits_landing(self):
        fixture = """
import json, os, tty
tty.setraw(0)
first_run_cancel = os.read(0, 1) == b'q'
landing_quit = os.read(0, 1) == b'q'
ok = first_run_cancel and landing_quit
print(json.dumps({'ok': ok, 'code': 'development_launched'}), flush=True)
raise SystemExit(0 if ok else 7)
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

    @staticmethod
    def _resistant_double_fork_fixture(marker_dir: Path) -> str:
        return f"""
import os, pathlib, signal, time
root = pathlib.Path({str(marker_dir)!r})
root.joinpath('leader').write_text(str(os.getpid()))
child = os.fork()
if child == 0:
    root.joinpath('child').write_text(str(os.getpid()))
    daemon = os.fork()
    if daemon != 0:
        os._exit(0)
    os.setsid()
    signal.signal(signal.SIGTERM, signal.SIG_IGN)
    signal.signal(signal.SIGHUP, signal.SIG_IGN)
    root.joinpath('daemon').write_text(str(os.getpid()))
    time.sleep(30)
    os._exit(0)
deadline = time.monotonic() + 5
while not root.joinpath('daemon').exists():
    if time.monotonic() >= deadline:
        raise SystemExit(9)
    time.sleep(0.01)
print('READY', flush=True)
time.sleep(30)
"""

    def _assert_marker_pids_gone(self, marker_dir: Path):
        pids = [int(path.read_text()) for path in marker_dir.iterdir()]
        deadline = time.monotonic() + 1.0
        while time.monotonic() < deadline:
            if all(not Path(f"/proc/{pid}").exists() for pid in pids):
                break
            time.sleep(0.01)
        for pid in pids:
            self.assertFalse(Path(f"/proc/{pid}").exists(), f"fixture PID {pid} survived")

    def test_post_rights_startup_failure_reaps_resistant_double_fork_lineage(self):
        real_fork = MODULE.os.fork
        real_receive = MODULE._receive_supervisor_packet
        real_set_blocking = MODULE.os.set_blocking
        real_pidfd_open = MODULE.os.pidfd_open
        real_close = MODULE.os.close
        real_socketpair = MODULE.socket.socketpair
        real_socket_close = MODULE.socket.socket.close
        parent_pid = os.getpid()
        original_subreaper = MODULE._get_child_subreaper()
        try:
            for initial_subreaper in (False, True):
                with self.subTest(initial_subreaper=initial_subreaper), tempfile.TemporaryDirectory() as temporary:
                    marker_dir = Path(temporary)
                    fixture = self._resistant_double_fork_fixture(marker_dir)
                    supervisor_pid = None
                    received_master = None
                    parent_control = None
                    master_close_count = 0
                    control_close_count = 0
                    supervisor_pidfds = {}

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

                    def tracked_socketpair(*args, **kwargs):
                        nonlocal parent_control
                        pair = real_socketpair(*args, **kwargs)
                        if os.getpid() == parent_pid:
                            parent_control = pair[0]
                        return pair

                    def tracked_socket_close(sock):
                        nonlocal control_close_count
                        if os.getpid() == parent_pid and sock is parent_control:
                            control_close_count += 1
                        return real_socket_close(sock)

                    def tracked_pidfd_open(pid):
                        descriptor = real_pidfd_open(pid)
                        if os.getpid() == parent_pid and pid == supervisor_pid:
                            supervisor_pidfds[descriptor] = 0
                        return descriptor

                    def tracked_close(fd):
                        nonlocal master_close_count
                        if os.getpid() == parent_pid:
                            if fd == received_master:
                                master_close_count += 1
                            if fd in supervisor_pidfds:
                                supervisor_pidfds[fd] += 1
                        return real_close(fd)

                    def fail_after_rights(fd, blocking):
                        if os.getpid() == parent_pid and fd == received_master:
                            deadline = time.monotonic() + 2
                            while not marker_dir.joinpath("daemon").exists():
                                if time.monotonic() >= deadline:
                                    raise AssertionError("fixture daemon did not start")
                                time.sleep(0.01)
                            raise OSError("injected post-SCM_RIGHTS failure")
                        return real_set_blocking(fd, blocking)

                    MODULE._child_subreaper(initial_subreaper)
                    started = time.monotonic()
                    with mock.patch.object(MODULE.os, "fork", tracked_fork), mock.patch.object(
                        MODULE, "_receive_supervisor_packet", tracked_receive
                    ), mock.patch.object(MODULE.os, "set_blocking", fail_after_rights), mock.patch.object(
                        MODULE, "SUPERVISOR_TERM_TIMEOUT", 0.1
                    ), mock.patch.object(MODULE, "SUPERVISOR_KILL_TIMEOUT", 0.3), mock.patch.object(
                        MODULE.socket, "socketpair", tracked_socketpair
                    ), mock.patch.object(MODULE.socket.socket, "close", tracked_socket_close), mock.patch.object(
                        MODULE.os, "pidfd_open", tracked_pidfd_open
                    ), mock.patch.object(MODULE.os, "close", tracked_close):
                        with self.assertRaisesRegex(OSError, "post-SCM_RIGHTS"):
                            MODULE._start_fixture_supervisor(
                                [sys.executable, "-c", fixture], os.environ.copy()
                            )
                    self.assertLess(time.monotonic() - started, 1.5)
                    self.assertIsNotNone(supervisor_pid)
                    with self.assertRaises(ChildProcessError):
                        os.waitpid(supervisor_pid, os.WNOHANG)
                    self._assert_marker_pids_gone(marker_dir)
                    self.assertEqual(MODULE._get_child_subreaper(), initial_subreaper)
                    self.assertIsNotNone(received_master)
                    self.assertEqual(master_close_count, 1)
                    self.assertEqual(control_close_count, 1)
                    self.assertTrue(supervisor_pidfds)
                    self.assertTrue(all(count == 1 for count in supervisor_pidfds.values()))
                    with self.assertRaises(OSError):
                        os.fstat(received_master)
        finally:
            MODULE._child_subreaper(original_subreaper)

    def test_stopped_teardown_reaps_resistant_double_fork_lineage(self):
        original_subreaper = MODULE._get_child_subreaper()
        try:
            for initial_subreaper in (False, True):
                with self.subTest(initial_subreaper=initial_subreaper), tempfile.TemporaryDirectory() as temporary:
                    marker_dir = Path(temporary)
                    MODULE._child_subreaper(initial_subreaper)
                    supervisor = MODULE._start_fixture_supervisor(
                        [sys.executable, "-c", self._resistant_double_fork_fixture(marker_dir)],
                        os.environ.copy(),
                    )
                    real_pidfd_open = MODULE.os.pidfd_open
                    real_close = MODULE.os.close
                    real_socket_close = MODULE.socket.socket.close
                    supervisor_pidfds = {}
                    pidfd_close_counts = {}
                    control_close_count = 0
                    master_close_count = 0

                    def tracked_pidfd_open(pid):
                        descriptor = real_pidfd_open(pid)
                        if pid == supervisor.pid:
                            supervisor_pidfds[descriptor] = True
                            pidfd_close_counts[descriptor] = 0
                        return descriptor

                    def tracked_close(fd):
                        nonlocal master_close_count
                        if fd == supervisor.master:
                            master_close_count += 1
                        if fd in pidfd_close_counts:
                            pidfd_close_counts[fd] += 1
                        return real_close(fd)

                    def tracked_socket_close(sock):
                        nonlocal control_close_count
                        if sock is supervisor.control:
                            control_close_count += 1
                        return real_socket_close(sock)
                    try:
                        deadline = time.monotonic() + 2
                        while not marker_dir.joinpath("daemon").exists():
                            if time.monotonic() >= deadline:
                                self.fail("fixture daemon did not start")
                            time.sleep(0.01)
                        os.kill(supervisor.pid, signal.SIGSTOP)
                        started = time.monotonic()
                        with mock.patch.object(
                            MODULE, "SUPERVISOR_CONTROL_TIMEOUT", 0.1
                        ), mock.patch.object(
                            MODULE, "SUPERVISOR_TERM_TIMEOUT", 0.1
                        ), mock.patch.object(MODULE, "SUPERVISOR_KILL_TIMEOUT", 0.3), mock.patch.object(
                            MODULE.os, "pidfd_open", tracked_pidfd_open
                        ), mock.patch.object(MODULE.os, "close", tracked_close), mock.patch.object(
                            MODULE.socket.socket, "close", tracked_socket_close
                        ):
                            with self.assertRaises((TimeoutError, BaseExceptionGroup)):
                                MODULE._stop_fixture_supervisor(supervisor)
                            MODULE._close_fd(supervisor.master)
                        self.assertLess(time.monotonic() - started, 1.5)
                        with self.assertRaises(ChildProcessError):
                            os.waitpid(supervisor.pid, os.WNOHANG)
                        self._assert_marker_pids_gone(marker_dir)
                        self.assertEqual(MODULE._get_child_subreaper(), initial_subreaper)
                        self.assertEqual(supervisor.control.fileno(), -1)
                        self.assertEqual(control_close_count, 1)
                        self.assertEqual(master_close_count, 1)
                        self.assertTrue(supervisor_pidfds)
                        self.assertTrue(
                            all(count == 1 for count in pidfd_close_counts.values())
                        )
                    finally:
                        try:
                            os.fstat(supervisor.master)
                        except OSError:
                            pass
                        else:
                            MODULE._close_fd(supervisor.master)
                        try:
                            os.kill(supervisor.pid, signal.SIGKILL)
                        except ProcessLookupError:
                            pass
        finally:
            MODULE._child_subreaper(original_subreaper)

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

    def test_json_response_may_follow_terminal_teardown_without_newline(self):
        encoded = MODULE.trailing_json(
            "screen contents\x1b[?25h\x1b[?1049l"
            '{"ok":true,"details":{"route":"{wizard}","quote":"\\\""}}\r\n'
        )
        self.assertEqual(
            json.loads(encoded),
            {"ok": True, "details": {"route": "{wizard}", "quote": '"'}},
        )

    def test_json_response_rejects_non_whitespace_suffix(self):
        self.assertIsNone(MODULE.trailing_json('{"ok":true}trailing'))

    def test_json_response_rejects_near_limit_hostile_output_linearly(self):
        hostile = "}" * (MODULE.MAX_COMMAND_OUTPUT - 1)
        with mock.patch.object(
            MODULE.json.JSONDecoder,
            "raw_decode",
            side_effect=AssertionError("decoder must not run without an opening object"),
        ):
            self.assertIsNone(MODULE.trailing_json(hostile))

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

    def test_synchronized_exit_before_identity_observation_is_classified(self):
        fixture = "import json; print(json.dumps({\"ok\": True, \"code\": \"fast\"}))"
        barrier = self._after_child_exit(
            MODULE._open_process_identity, expose_identity=False
        )
        with mock.patch.object(MODULE, "_open_process_identity", barrier):
            code, encoded = MODULE.run_asb(
                Path(sys.executable),
                ["-c", fixture],
                os.environ.copy(),
                json_output=True,
            )
        self.assertEqual(code, 0)
        self.assertEqual(json.loads(encoded), {"ok": True, "code": "fast"})

    def test_synchronized_fast_human_and_nonzero_exit_are_preserved(self):
        barrier = self._after_child_exit(
            MODULE._open_process_identity, expose_identity=False
        )
        with mock.patch.object(MODULE, "_open_process_identity", barrier):
            code, output = MODULE.run_asb(
                Path(sys.executable),
                ["-c", "print(\"fast human\"); raise SystemExit(7)"],
                os.environ.copy(),
                json_output=False,
            )
        self.assertEqual((code, output), (7, "fast human"))

    def test_synchronized_fast_malformed_and_empty_output_fail_closed(self):
        for fixture, message in (
            ("print(\"not-json\")", "emitted no JSON"),
            ("raise SystemExit(0)", "emitted no output"),
        ):
            with self.subTest(message=message):
                barrier = self._after_child_exit(
                    MODULE._open_process_identity, expose_identity=False
                )
                with mock.patch.object(MODULE, "_open_process_identity", barrier):
                    with self.assertRaisesRegex(AssertionError, message):
                        MODULE.run_asb(
                            Path(sys.executable),
                            ["-c", fixture],
                            os.environ.copy(),
                            json_output=True,
                        )

    def test_live_noninteractive_wrong_foreground_group_is_rejected(self):
        real_tcgetpgrp = MODULE.os.tcgetpgrp

        def wrong_foreground(fd):
            return real_tcgetpgrp(fd) + 1

        fixture = "import time; print(\"READY\", flush=True); time.sleep(30)"
        with mock.patch.object(
            MODULE.os, "tcgetpgrp", side_effect=wrong_foreground
        ):
            with self.assertRaisesRegex(
                RuntimeError, "live PTY child is not its controlling foreground group"
            ):
                MODULE.run_asb(
                    Path(sys.executable),
                    ["-c", fixture],
                    os.environ.copy(),
                    json_output=False,
                    timeout_seconds=2,
                )

    def test_interactive_fast_exit_does_not_bypass_live_pty_proof(self):
        barrier = self._after_child_exit(
            MODULE._open_process_identity, expose_identity=True
        )
        with mock.patch.object(MODULE, "_open_process_identity", barrier):
            with self.assertRaisesRegex(
                RuntimeError, "interactive PTY child exited before live foreground proof"
            ):
                MODULE.run_asb(
                    Path(sys.executable),
                    ["-c", "print(\"not interactive\")"],
                    os.environ.copy(),
                    json_output=False,
                    interactive_quit=True,
                )

    def test_reused_pid_identity_is_rejected_and_pidfd_closed(self):
        read_fd, write_fd = os.pipe()
        first = MODULE._ProcessSnapshot(os.getpid(), 400, 10)
        changed = MODULE._ProcessSnapshot(os.getpid(), 400, 11)
        try:
            with mock.patch.object(
                MODULE, "_read_process_identity", side_effect=(first, changed)
            ), mock.patch.object(MODULE.os, "pidfd_open", return_value=read_fd):
                self.assertIsNone(MODULE._open_process_identity(12345, 400))
            with self.assertRaises(OSError):
                os.fstat(read_fd)
        finally:
            os.close(write_fd)

    def test_synchronized_fast_exit_status_property_matrix(self):
        for expected in (0, 1, 2, 3, 7, 64, 126, 127, 255):
            with self.subTest(exit_code=expected):
                barrier = self._after_child_exit(
                    MODULE._open_process_identity, expose_identity=False
                )
                fixture = f"print(\"status {expected}\"); raise SystemExit({expected})"
                with mock.patch.object(MODULE, "_open_process_identity", barrier):
                    code, output = MODULE.run_asb(
                        Path(sys.executable),
                        ["-c", fixture],
                        os.environ.copy(),
                        json_output=False,
                    )
                self.assertEqual((code, output), (expected, f"status {expected}"))

    def test_synchronized_fast_exit_closes_received_master_once(self):
        real_start = MODULE._start_fixture_supervisor
        real_close = MODULE.os.close
        master = None
        close_count = 0

        def tracked_start(*args, **kwargs):
            nonlocal master
            supervisor = real_start(*args, **kwargs)
            master = supervisor.master
            return supervisor

        def tracked_close(fd):
            nonlocal close_count
            if fd == master:
                close_count += 1
            return real_close(fd)

        barrier = self._after_child_exit(
            MODULE._open_process_identity, expose_identity=False
        )
        with mock.patch.object(
            MODULE, "_open_process_identity", barrier
        ), mock.patch.object(
            MODULE, "_start_fixture_supervisor", tracked_start
        ), mock.patch.object(MODULE.os, "close", tracked_close):
            code, output = MODULE.run_asb(
                Path(sys.executable),
                ["-c", "print(\"closed\")"],
                os.environ.copy(),
                json_output=False,
            )
        self.assertEqual((code, output), (0, "closed"))
        self.assertIsNotNone(master)
        self.assertEqual(close_count, 1)
        with self.assertRaises(OSError):
            os.fstat(master)

    def test_synchronized_fast_exit_reaps_descendant_and_spares_unrelated_child(self):
        with tempfile.TemporaryDirectory() as temporary:
            marker = Path(temporary) / "descendant"
            fixture = """
import json, os, pathlib, signal, time
child = os.fork()
if child == 0:
    os.setsid()
    signal.signal(signal.SIGTERM, signal.SIG_IGN)
    pathlib.Path(os.environ["FAST_DESCENDANT"]).write_text(str(os.getpid()))
    while True:
        time.sleep(1)
deadline = time.monotonic() + 2
while not pathlib.Path(os.environ["FAST_DESCENDANT"]).exists():
    if time.monotonic() >= deadline:
        raise SystemExit(9)
    time.sleep(0.001)
print(json.dumps({"ok": True, "code": "fast"}), flush=True)
"""
            environment = os.environ.copy()
            environment["FAST_DESCENDANT"] = str(marker)
            ambient = subprocess.Popen(
                [sys.executable, "-c", "import time; time.sleep(30)"]
            )
            descendant_pid = None
            try:
                barrier = self._after_child_exit(
                    MODULE._open_process_identity, expose_identity=False
                )
                with mock.patch.object(MODULE, "_open_process_identity", barrier):
                    code, encoded = MODULE.run_asb(
                        Path(sys.executable),
                        ["-c", fixture],
                        environment,
                        json_output=True,
                    )
                descendant_pid = int(marker.read_text(encoding="utf-8"))
                self.assertEqual(code, 0)
                self.assertEqual(json.loads(encoded)["code"], "fast")
                with self.assertRaises(ProcessLookupError):
                    os.kill(descendant_pid, 0)
                self.assertIsNone(ambient.poll())
            finally:
                ambient.kill()
                ambient.wait()
                if descendant_pid is not None:
                    try:
                        os.kill(descendant_pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass

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
