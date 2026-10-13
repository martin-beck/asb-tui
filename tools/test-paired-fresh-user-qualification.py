#!/usr/bin/env python3
# Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Contract tests for the AR-1689 paired fresh-user qualifier."""

from __future__ import annotations

import importlib.util
import unittest
from pathlib import Path


def load_runner():
    path = Path(__file__).with_name("run-paired-fresh-user-qualification.py")
    spec = importlib.util.spec_from_file_location("ar1689_runner", path)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


TUI_HEAD = "a" * 40
ASB_HEAD = "b" * 40


def installed_receipt() -> dict:
    return {
        "ar": "AR-1622", "credentials": "none",
        "network": {"install_materialization": "allowed", "benchmark_and_replay": "denied"},
        "selection": {"agent": "opencode", "replay": "offline"},
        "provenance": {"tui_commit": TUI_HEAD, "asb_commit": ASB_HEAD},
        "routes": {
            "status": {"verified": True},
            "provider_selection": "provider_lifecycle_1656 passed",
            "fanout_capture_replay": "coverage_setup_recording and development_journey passed",
            "end_to_end_test": "passed",
            "comparison": "end_to_end_qualification passed",
            "analysis": "end_to_end_qualification passed",
        },
    }


def content_receipt() -> dict:
    return {
        "ar": "AR-1686", "complete": True, "network": "denied", "credentials": "none",
        "provenance": {"tui_commit": TUI_HEAD, "asb_commit": ASB_HEAD},
        "stages": {
            "asb_content_addressed_qualification": {"passed": True, "receipt_sha256": "c" * 64},
            "tui_journey": {"passed": True},
        },
    }


class PairedFreshUserQualificationTests(unittest.TestCase):
    def test_accepts_complete_exact_head_receipts(self):
        runner = load_runner()
        runner.validate_installed_tui(installed_receipt(), TUI_HEAD, ASB_HEAD)
        runner.validate_asb_content(content_receipt(), TUI_HEAD, ASB_HEAD)

    def test_rejects_mismatched_head_or_networked_benchmark(self):
        runner = load_runner()
        installed = installed_receipt()
        installed["provenance"]["asb_commit"] = "c" * 40
        with self.assertRaisesRegex(ValueError, "paired heads"):
            runner.validate_installed_tui(installed, TUI_HEAD, ASB_HEAD)
        content = content_receipt()
        content["network"] = "used"
        with self.assertRaisesRegex(ValueError, "offline"):
            runner.validate_asb_content(content, TUI_HEAD, ASB_HEAD)


if __name__ == "__main__":
    unittest.main()
