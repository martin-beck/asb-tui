#!/usr/bin/env python3
# Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Contract tests for the AR-1657 deterministic matrix runner."""

import unittest
from pathlib import Path

SOURCE = Path(__file__).with_name("run-agent-provider-matrix.py").read_text(encoding="utf-8")


class AgentProviderMatrixTests(unittest.TestCase):
    def test_matrix_declares_both_agents_and_all_development_tuples(self):
        for value in ("opencode", "opendesk", "openrouter", "fixture-model", "gpt-4o"):
            self.assertIn(value, SOURCE)
        self.assertIn('"tuple_count": len(EXPECTED_TUPLES)', SOURCE)

    def test_matrix_is_offline_and_credential_free(self):
        self.assertIn('"--offline"', SOURCE)
        self.assertIn('"credentials": "none"', SOURCE)
        self.assertIn('environment["ASB_TUI_NETWORK_POLICY"] = "deny"', SOURCE)

    def test_matrix_receipt_covers_defaults_overrides_restart_unavailable_and_formats(self):
        for value in ("defaults", "overrides", "restart", "unavailable_reason", "offline", '"human"', '"json"'):
            self.assertIn(value, SOURCE)


if __name__ == "__main__":
    unittest.main()
