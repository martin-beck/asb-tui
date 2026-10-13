#!/usr/bin/env python3
# Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Unit checks for the AR-1686 qualification process boundary."""

from __future__ import annotations

import importlib.util
import json
import os
from pathlib import Path
import tempfile
from unittest import TestCase, main
from unittest.mock import patch


def load_runner():
    path = Path(__file__).with_name("run-content-addressed-qualification.py")
    spec = importlib.util.spec_from_file_location("ar1686_runner", path)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class EnvironmentTests(TestCase):
    def test_child_environment_removes_credential_names_and_keeps_policy(self):
        runner = load_runner()
        with patch.dict(
            os.environ,
            {
                "OPENROUTER_API_KEY": "do-not-forward",
                "GITHUB_TOKEN": "do-not-forward",
                "PRIVATE_KEY": "do-not-forward",
                "QUALIFICATION_SAFE_VALUE": "retained",
            },
            clear=True,
        ):
            environment = runner.sanitized_environment()
        self.assertNotIn("OPENROUTER_API_KEY", environment)
        self.assertNotIn("GITHUB_TOKEN", environment)
        self.assertNotIn("PRIVATE_KEY", environment)
        self.assertEqual(environment["QUALIFICATION_SAFE_VALUE"], "retained")
        self.assertEqual(environment["ASB_TUI_NETWORK_POLICY"], "deny")
        self.assertEqual(environment["NO_PROXY"], "*")

    def test_upstream_receipt_must_be_exact_private_and_complete(self):
        runner = load_runner()
        receipt = {
            "schema_version": 1,
            "status": "qualified",
            "network": "denied",
            "credentials": "none",
            "source_revision": "a" * 40,
            "paired_heads": {"asb": "a" * 40, "asb_tui": "b" * 40},
            "runner_sha256": "c" * 64,
            "fixture_sha256": "d" * 64,
            "provider_profile_sha256": "e" * 64,
            "selection": {"agents": ["codex"], "workloads": ["original.bug-fix"]},
            "plan_run_journey": {"comparison": {"comparable": True}},
            "campaign": {"comparison": {"comparable": True}},
            "negative_identity": {"rejected": True},
            "negative_incomplete_coverage": {"rejected": True},
            "receipt_sha256": "c" * 64,
        }
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "receipt.json"
            path.write_text(json.dumps(receipt), encoding="utf-8")
            self.assertEqual(
                runner.load_upstream_receipt(path, "a" * 40, "b" * 40), receipt
            )
            receipt["network"] = "used"
            path.write_text(json.dumps(receipt), encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "credential-free and offline"):
                runner.load_upstream_receipt(path, "a" * 40, "b" * 40)


if __name__ == "__main__":
    main()
