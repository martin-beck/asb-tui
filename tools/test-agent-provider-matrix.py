#!/usr/bin/env python3
# Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Contract tests for the AR-1657 deterministic matrix runner."""

import unittest
import json
from pathlib import Path

SOURCE = Path(__file__).with_name("run-agent-provider-matrix.py").read_text(encoding="utf-8")
FIXTURE = json.loads((Path(__file__).parents[1] / "tests/fixtures/agent-provider-matrix.json").read_text(encoding="utf-8"))


class AgentProviderMatrixTests(unittest.TestCase):
    def test_matrix_declares_both_agents_and_all_development_tuples(self):
        for value in ("AR1657_CATALOG_JSON=", "agent-provider-matrix.json"):
            self.assertIn(value, SOURCE)
        self.assertIn("catalog", SOURCE)
        self.assertEqual(len(FIXTURE["tuples"]), 6)
        self.assertEqual({tuple["agent"] for tuple in FIXTURE["tuples"]}, {"opencode", "opendesk"})

    def test_matrix_rejects_catalog_drift(self):
        self.assertIn("catalog drifted from the reviewed fixture", SOURCE)
        self.assertIn("actual != expected", SOURCE)
        self.assertIn("authoritative_asb_catalog", SOURCE)
        self.assertIn("provider-catalog", SOURCE)
        self.assertIn("does not cover paired ASB catalog choices", SOURCE)

    def test_matrix_has_parent_installed_command_parity_acceptance(self):
        for value in ("tui", "install", "--offline", "installed_asb_tui_status", "installed_asb_tui_launch", "installed_asb_tui_direct_status", "--tui-binary", "parity"):
            self.assertIn(value, SOURCE)
        self.assertIn("asb-tui install did not produce an installed frontend", SOURCE)
        self.assertIn("installed asb-tui status is not ready", SOURCE)
        self.assertIn('str(tui_binary), "tui", "status"', SOURCE)
        lifecycle = SOURCE.split("def run_asb_tui_parity", 1)[1].split("def main", 1)[0]
        self.assertNotIn('[str(asb_binary), "tui"', lifecycle)

    def test_matrix_is_offline_and_credential_free(self):
        self.assertIn('"--offline"', SOURCE)
        self.assertIn('"credentials": "none"', SOURCE)
        self.assertIn('environment["ASB_TUI_NETWORK_POLICY"] = "deny"', SOURCE)

    def test_matrix_receipt_covers_defaults_overrides_restart_unavailable_and_formats(self):
        for value in ("defaults", "overrides", "restart", "unavailable_reason", "offline", '"human"', '"json"'):
            self.assertIn(value, SOURCE)


if __name__ == "__main__":
    unittest.main()
