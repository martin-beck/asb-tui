#!/usr/bin/env python3
# Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Unit checks for the AR-1686 qualification process boundary."""

from __future__ import annotations

import importlib.util
import os
from pathlib import Path
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


if __name__ == "__main__":
    main()
