#!/usr/bin/env python3
# Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Validate the credential-free development/mock journey contract (AR-1330)."""

import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CONTRACT = ROOT / "docs/qualification/development-journey-v1.json"
EXPECTED_STEPS = ["enroll", "select", "mock-capture", "offline-replay", "comparison"]
EXPECTED_WARNINGS = {"missing-auth", "missing-signature-validation", "missing-key-management"}


def fail(message: str) -> None:
    raise SystemExit(f"development journey invalid: {message}")


def main() -> int:
    try:
        document = json.loads(CONTRACT.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        fail(str(error))
    if document.get("schema_version") != 1:
        fail("schema_version must be 1")
    if document.get("journey_id") != "asb-tui-development-journey":
        fail("journey_id is not stable")
    if document.get("classification") != "development/mock":
        fail("journey must be classified development/mock")
    if document.get("network") != "denied" or document.get("credentials") != "none":
        fail("journey must deny network and credentials")
    for field in ("asb_revision", "asb_tui_revision"):
        if document.get(field) != "runtime-provided":
            fail(f"{field} must require exact runtime evidence")
    warnings = document.get("warning_cases")
    if not isinstance(warnings, list) or {item.get("id") for item in warnings} != EXPECTED_WARNINGS:
        fail("warning cases must cover auth, signature, and key-management absence")
    if any(item.get("continue") is not True or not item.get("warning") for item in warnings):
        fail("every warning case must be visible and non-gating")
    steps = document.get("steps")
    if not isinstance(steps, list) or [item.get("id") for item in steps] != EXPECTED_STEPS:
        fail("journey steps are incomplete or out of order")
    if any(item.get("continue_on_warning") is not True for item in steps):
        fail("all development steps must continue on warning")
    if any(not re.fullmatch(r"[a-z0-9_-]+", item.get("action", "")) for item in steps):
        fail("step actions must be stable identifiers")
    print(f"development/mock journey contract valid: {len(steps)} steps, {len(warnings)} warning cases")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
