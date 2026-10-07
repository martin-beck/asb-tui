#!/usr/bin/env python3
# Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Focused contract tests for the AR-1622 wrapper."""

import unittest
from pathlib import Path


ROOT = Path(__file__).parents[1]
SOURCE = (ROOT / "tools/run-current-main-quickstart.py").read_text(encoding="utf-8")


class CurrentMainQuickstartTests(unittest.TestCase):
    def test_wrapper_binds_exact_heads_and_ar_receipt(self):
        for value in (
            'receipt["ar"] = "AR-1622"',
            '"asb_checkout_head": asb_head',
            '"tui_checkout_head": tui_head',
            '"tui_ref": tui_ref',
            "quickstart receipt is not bound to the requested ASB head",
            "quickstart receipt is not bound to the requested TUI head",
            "require_exact_ref(tui_checkout, tui_ref, tui_head)",
            "--tui-ref must resolve to the tested TUI checkout HEAD",
        ):
            self.assertIn(value, SOURCE)

    def test_wrapper_preserves_secret_free_and_network_denied_defaults(self):
        self.assertIn("SECRET_MARKERS", SOURCE)
        self.assertIn('"ASB_TUI_NETWORK_POLICY": "deny"', SOURCE)
        self.assertIn('"HTTP_PROXY": "http://127.0.0.1:1"', SOURCE)
        self.assertIn('"NO_PROXY": "*"', SOURCE)

    def test_wrapper_delegates_existing_behavior_matrix(self):
        self.assertIn('with_name("run-quickstart-acceptance.py")', SOURCE)
        self.assertIn('"--asb-binary", str(asb_binary)', SOURCE)
        self.assertIn('"--receipt", str(inner_receipt)', SOURCE)


if __name__ == "__main__":
    unittest.main()
