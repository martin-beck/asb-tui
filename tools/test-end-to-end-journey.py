#!/usr/bin/env python3
# Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Validate the AR-1338 credential-free end-to-end journey contract."""

import json
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CONTRACT = ROOT / "docs/qualification/end-to-end-journey-v1.json"

EXPECTED_STAGES = [
    "install",
    "launch",
    "wizard",
    "benchmark_selection",
    "materialization",
    "preflight",
    "launch_run",
    "live_statistics",
    "final_results",
    "history_comparison",
]
EXPECTED_RECOVERIES = {
    "version": "refresh_version",
    "catalog": "refresh_catalog",
    "bundle": "repair_bundle",
    "protocol": "retry_broker",
}


def fail(message: str) -> None:
    raise SystemExit(f"end-to-end journey invalid: {message}")


def main() -> int:
    try:
        document = json.loads(CONTRACT.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        fail(str(error))
    if document.get("schema_version") != 1:
        fail("schema_version must be 1")
    if document.get("journey_id") != "asb-tui-end-to-end-development-journey":
        fail("journey_id is not stable")
    if document.get("classification") != "development/mock":
        fail("journey must be classified development/mock")
    if document.get("network") != "denied" or document.get("credentials") != "none":
        fail("network and credentials must remain denied")
    if document.get("source_sha") != "runtime-provided":
        fail("source SHA must be recorded at qualification time")
    stages = document.get("stages")
    if not isinstance(stages, list) or [stage.get("id") for stage in stages] != EXPECTED_STAGES:
        fail("stages are incomplete or out of order")
    for stage in stages:
        if not stage.get("command") or not stage.get("evidence"):
            fail("every stage needs command and evidence")
        if not re.fullmatch(r"[a-z0-9_ -]+", stage["id"]):
            fail("stage identifiers must be stable")
    recoveries = document.get("mismatch_recoveries")
    if not isinstance(recoveries, list) or {
        item.get("id"): item.get("recovery") for item in recoveries
    } != EXPECTED_RECOVERIES:
        fail("mismatch recovery choices are incomplete")
    print(f"end-to-end development/mock contract valid: {len(stages)} stages, {len(recoveries)} recoveries")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
