#!/usr/bin/env python3
# Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Focused tests for the exact-main capture/replay receipt runner."""

import importlib.util
import unittest
from unittest import mock
from pathlib import Path


SPEC = importlib.util.spec_from_file_location(
    "run_quickstart_acceptance", Path(__file__).with_name("run-quickstart-acceptance.py")
)
assert SPEC and SPEC.loader
RUNNER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(RUNNER)


class QuickstartAcceptanceTests(unittest.TestCase):
    def test_runner_sanitizes_credentials_and_preserves_network_denial(self):
        with mock.patch.dict(
            RUNNER.os.environ,
            {
                "OPENROUTER_API_KEY": "redacted-test-secret",
                "GITHUB_TOKEN": "redacted-test-token",
                "SAFE_RUNNER_FLAG": "retained",
            },
            clear=False,
        ):
            environment = RUNNER.sanitized_environment()
        self.assertNotIn("OPENROUTER_API_KEY", environment)
        self.assertNotIn("GITHUB_TOKEN", environment)
        self.assertEqual(environment["SAFE_RUNNER_FLAG"], "retained")
        self.assertEqual(environment["ASB_TUI_NETWORK_POLICY"], "deny")

    def test_selects_only_bounded_native_original_workloads(self):
        catalog = {
            "entries": [
                {"id": "original.bug-fix", "platform": "linux-x86_64:native-tested"},
                {"id": "original.feature-addition", "platform": "linux-x86_64:native-tested"},
                {"id": "swe-bench", "platform": "linux-x86_64:fixture-only"},
                {"id": "original.other", "platform": "aarch64"},
            ]
        }
        self.assertEqual(
            RUNNER.select_workload_ids(catalog),
            ["original.bug-fix", "original.feature-addition"],
        )

    def test_receipt_matrix_contract_names_negative_and_strict_paths(self):
        source = Path(RUNNER.__file__).read_text(encoding="utf-8")
        for field in ("selected", "all", "incomplete_matrix", "strict_replay", "strict_replay_reference", "cassette_sha256", "result_digest", "handoff_manifest_sha256", "tamper_negative", "restart_status"):
            self.assertIn(f'"{field}"', source)

    def test_quickstart_binds_omitted_channel_and_actual_handoff_digest(self):
        source = Path(RUNNER.__file__).read_text(encoding="utf-8")
        self.assertIn('["tui", "install", "--json"]', source)
        self.assertIn("ASB_TUI_CHANNEL_MANIFEST", source)
        self.assertIn('"executable_sha256"', source)
        self.assertIn("channel_manifest_sha256", source)


if __name__ == "__main__":
    unittest.main()
