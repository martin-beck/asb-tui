#!/usr/bin/env python3
# Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Contract tests for the AR-1668 paired journey orchestrator."""

import importlib.util
import subprocess
import tempfile
import unittest
from pathlib import Path


SPEC = importlib.util.spec_from_file_location(
    "run_post_launch_journey", Path(__file__).with_name("run-post-launch-journey.py")
)
assert SPEC and SPEC.loader
RUNNER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(RUNNER)


class PostLaunchJourneyTests(unittest.TestCase):
    def test_receipt_binds_all_required_stages_and_selection(self):
        source = Path(RUNNER.__file__).read_text(encoding="utf-8")
        for field in (
            '"AR-1668"',
            '"setup_capture_replay_analysis"',
            '"channel_restart_rollback_remove"',
            '"opencode"',
            '"opendesk"',
            '"authentication"',
            '"defaults"',
            '"offline"',
            '"asb_commit"',
            '"tui_commit"',
        ):
            self.assertIn(field, source)

    def test_child_environment_denies_network_and_removes_credentials(self):
        source = Path(RUNNER.__file__).read_text(encoding="utf-8")
        self.assertIn('"ASB_TUI_NETWORK_POLICY": "deny"', source)
        self.assertIn('"ASB_TUI_DEV_REPOSITORY": f"file://{tui_checkout}"', source)
        self.assertIn('"ASB_TUI_DEV_REF": tui_ref', source)
        self.assertIn('"HTTP_PROXY": "http://127.0.0.1:1"', source)
        self.assertIn('"API_KEY"', source)
        self.assertIn('"TOKEN"', source)

    def test_composes_existing_exact_head_runners(self):
        source = Path(RUNNER.__file__).read_text(encoding="utf-8")
        self.assertIn("run-quickstart-acceptance.py", source)
        self.assertIn("run-channel-compatibility-matrix.py", source)
        self.assertIn("--asb-head", source)

    def test_detached_exact_checkout_uses_validated_main_ref(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            subprocess.run(["git", "init", "--quiet", "--initial-branch", "main", str(root)], check=True)
            subprocess.run(["git", "-C", str(root), "config", "user.name", "Test"], check=True)
            subprocess.run(["git", "-C", str(root), "config", "user.email", "test@example.invalid"], check=True)
            (root / "README").write_text("fixture\n", encoding="utf-8")
            subprocess.run(["git", "-C", str(root), "add", "README"], check=True)
            subprocess.run(["git", "-C", str(root), "commit", "--quiet", "-m", "fixture"], check=True)
            subprocess.run(["git", "-C", str(root), "checkout", "--quiet", "--detach", "HEAD"], check=True)
            self.assertEqual(RUNNER.branch(root), "main")

    def test_detached_checkout_without_validated_ref_fails_actionably(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            subprocess.run(["git", "init", "--quiet", str(root)], check=True)
            subprocess.run(["git", "-C", str(root), "config", "user.name", "Test"], check=True)
            subprocess.run(["git", "-C", str(root), "config", "user.email", "test@example.invalid"], check=True)
            (root / "README").write_text("fixture\n", encoding="utf-8")
            subprocess.run(["git", "-C", str(root), "add", "README"], check=True)
            subprocess.run(["git", "-C", str(root), "commit", "--quiet", "-m", "fixture"], check=True)
            subprocess.run(["git", "-C", str(root), "checkout", "--quiet", "--detach", "HEAD"], check=True)
            with self.assertRaises(SystemExit) as error:
                RUNNER.branch(root)
            self.assertIn("refs/heads/main", str(error.exception))


if __name__ == "__main__":
    unittest.main()
