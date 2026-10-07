#!/usr/bin/env python3
# Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Contract tests for the public-boundary operator quickstart."""
import importlib.util
import json
import os
import signal
import sys
import tempfile
import time
import unittest
from pathlib import Path

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
        self.assertIn("setsid=True", SOURCE)
        self.assertIn("POSIX_SPAWN_OPEN", SOURCE)

    def test_receipt_records_terminal_contract_without_private_checkout_paths(self):
        self.assertIn('"foreground_process_group": True', SOURCE)
        self.assertIn('"rows": PTY_ROWS', SOURCE)
        self.assertNotIn('"asb_checkout": str', SOURCE)
        self.assertNotIn('"tui_checkout": str', SOURCE)


if __name__ == "__main__":
    unittest.main()
