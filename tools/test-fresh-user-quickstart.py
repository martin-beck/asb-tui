#!/usr/bin/env python3
# Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Focused contract tests for the executable fresh-user runner."""

import unittest
from pathlib import Path


SOURCE = Path(__file__).with_name("run-fresh-user-quickstart.py").read_text(encoding="utf-8")


class FreshUserQuickstartTests(unittest.TestCase):
    def test_builds_or_accepts_binaries_and_delegates_the_complete_journey(self):
        for value in (
            'cargo", "build", "--locked',
            'with_name("run-current-main-quickstart.py")',
            '"--asb-binary", str(asb_binary)',
            '"--tui-checkout", str(tui_checkout)',
            '"--tui-ref", args.tui_ref',
            'parser.add_argument(\n        "--tui-ref"',
            'evidence["ar"] = "AR-1643"',
        ):
            self.assertIn(value, SOURCE)

    def test_runner_is_secret_free_and_denies_benchmark_network(self):
        for value in (
            "SECRET_MARKERS",
            '"ASB_TUI_NETWORK_POLICY": "deny"',
            '"HTTP_PROXY": "http://127.0.0.1:1"',
            '"NO_PROXY": "*"',
        ):
            self.assertIn(value, SOURCE)


if __name__ == "__main__":
    unittest.main()
