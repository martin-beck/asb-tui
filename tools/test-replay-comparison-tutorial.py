#!/usr/bin/env python3
# Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Validate AR-1223 tutorials and synthetic record/comparison reports offline."""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
TUTORIALS = [ROOT / "docs/tutorials/llm-record-replay-v1.json", ROOT / "docs/tutorials/multi-agent-comparison-v1.json"]
ID = re.compile(r"^[a-z][a-z0-9-]{0,63}$")
SYNTHETIC = re.compile(r"^synthetic-[a-z0-9-]+$")
DIGEST = re.compile(r"^sha256:synthetic-[a-z0-9-]+$")
SHELL = set(";&|<>`$(){}!\\\n\r")
SECRET = re.compile(r"(?i)(api[_-]?key|token|password|secret|bearer|credential)")


class Invalid(ValueError):
    pass


def bounded(value: object, name: str, maximum: int = 256) -> str:
    if not isinstance(value, str) or not value or len(value) > maximum:
        raise Invalid(f"{name} must be a bounded non-empty string")
    if any(char in value for char in SHELL) or SECRET.search(value):
        raise Invalid(f"{name} contains unsafe syntax or secret-like text")
    return value


def synthetic(value: object, name: str) -> str:
    value = bounded(value, name)
    if not SYNTHETIC.fullmatch(value):
        raise Invalid(f"{name} must be synthetic")
    return value


def digest(value: object, name: str) -> str:
    value = bounded(value, name)
    if not DIGEST.fullmatch(value):
        raise Invalid(f"{name} must be a synthetic digest")
    return value


def command(value: object, step_id: str) -> None:
    if not isinstance(value, list) or not 2 <= len(value) <= 16:
        raise Invalid(f"step {step_id} command must contain 2-16 arguments")
    args = [bounded(item, f"step {step_id} argument") for item in value]
    if args[:3] != ["asb", "tui", args[2]]:
        raise Invalid(f"step {step_id} must use the symbolic asb tui route")
    route = args[2]
    allowed = {
        "record": {
            ("--run", "synthetic-run-001", "--output", "synthetic-cassette-001"),
            ("--inspect", "synthetic-cassette-001"),
        },
        "replay": {
            ("--cassette", "synthetic-cassette-001", "--benchmark", "synthetic-benchmark", "--agent", "synthetic-agent"),
            ("--cassette", "synthetic-cassette-001", "--benchmark", "synthetic-benchmark", "--agent", "synthetic-agent", "--show-failed"),
        },
        "compare": {
            ("--benchmark", "synthetic-benchmark", "--runs", "synthetic-run-a,synthetic-run-b"),
            ("--benchmark", "synthetic-benchmark", "--runs", "synthetic-run-a,synthetic-run-b", "--report"),
            ("--benchmark", "synthetic-benchmark", "--runs", "synthetic-run-a,synthetic-run-c", "--report"),
        },
    }
    if route not in allowed or tuple(args[3:]) not in allowed[route]:
        raise Invalid(f"step {step_id} has unsupported or reordered options")
    for arg in args[3:]:
        if not arg.startswith("--") and not SYNTHETIC.fullmatch(arg) and "," not in arg:
            raise Invalid(f"step {step_id} has a non-synthetic identity")
        if "," in arg and any(not SYNTHETIC.fullmatch(item) for item in arg.split(",")):
            raise Invalid(f"step {step_id} has a non-synthetic selection")


def validate_tutorial(path: Path) -> dict:
    document = json.loads(path.read_text(encoding="utf-8"))
    if set(document) != {"schema_version", "tutorial_id", "title", "steps"}:
        raise Invalid(f"{path.name} has an invalid closed top-level shape")
    if document["schema_version"] != 1 or not ID.fullmatch(bounded(document["tutorial_id"], "tutorial ID")):
        raise Invalid(f"{path.name} has an invalid version or ID")
    bounded(document["title"], "tutorial title")
    if not isinstance(document["steps"], list) or not document["steps"]:
        raise Invalid(f"{path.name} must contain steps")
    seen = set()
    for step in document["steps"]:
        if set(step) != {"id", "command", "expect", "references", "network", "credentials"}:
            raise Invalid(f"{path.name} has an invalid step shape")
        step_id = bounded(step["id"], "step ID")
        if not ID.fullmatch(step_id) or step_id in seen:
            raise Invalid(f"{path.name} has duplicate or invalid step IDs")
        seen.add(step_id)
        command(step["command"], step_id)
        expect = step["expect"]
        if set(expect) != {"exit_code", "stdout_shape"} or not isinstance(expect["exit_code"], int):
            raise Invalid(f"{path.name} has an invalid expectation")
        bounded(expect["stdout_shape"], "stdout shape", 64)
        if step["network"] != "denied" or step["credentials"] != "none":
            raise Invalid(f"{path.name} must be offline and credential-free")
        if not isinstance(step["references"], list) or not step["references"]:
            raise Invalid(f"{path.name} needs an explicit fixture or contract reference")
        for reference in step["references"]:
            if set(reference) != {"path", "kind"} or not isinstance(reference["path"], str):
                raise Invalid(f"{path.name} has an invalid reference")
            if reference["path"].startswith(("/", "~")) or ".." in reference["path"].split("/") or reference["kind"] not in {"input", "fixture", "output"}:
                raise Invalid(f"{path.name} has an unsafe reference")
    return document


def validate_fixtures() -> None:
    replay = ROOT / "tests/fixtures/tutorial/llm-record-replay"
    required_recorded = {"schema_version", "state", "cassette_id", "benchmark_id", "benchmark_digest", "run_id", "generation", "response_count", "content_digest", "credential_references", "explanation"}
    required_result = {"schema_version", "state", "cassette_id", "benchmark_id", "run_id", "source_run_id", "generation", "completed_measures", "total_measures", "network_used", "explanation"}
    values = {}
    for path in sorted(replay.glob("*.json")):
        value = json.loads(path.read_text(encoding="utf-8"))
        values[value["state"]] = value
        if value["schema_version"] != 1 or not isinstance(value["explanation"], str) or len(value["explanation"]) < 20:
            raise Invalid(f"invalid replay fixture: {path.name}")
        for key in ("cassette_id", "benchmark_id", "run_id"):
            synthetic(value[key], key)
        if value["state"] == "recorded":
            if set(value) != required_recorded or not all(isinstance(value[k], int) and value[k] >= 0 for k in ("generation", "response_count")):
                raise Invalid(f"invalid recording fixture: {path.name}")
            digest(value["benchmark_digest"], "benchmark digest")
            digest(value["content_digest"], "content digest")
            if value["credential_references"] != []:
                raise Invalid("recorded fixture contains credential references")
        elif value["state"] in {"replayed", "failed"}:
            expected = required_result | ({"failure_retained"} if value["state"] == "failed" else set())
            if set(value) != expected or not isinstance(value["network_used"], bool) or value["network_used"]:
                raise Invalid(f"invalid replay result: {path.name}")
            synthetic(value["source_run_id"], "source run ID")
            if value["state"] == "failed" and value["failure_retained"] is not True:
                raise Invalid("failed replay must be retained")
        else:
            raise Invalid(f"unknown replay state: {path.name}")
    if set(values) != {"recorded", "replayed", "failed"}:
        raise Invalid("replay fixtures must cover recording, success, and retained failure")
    comparable = json.loads((ROOT / "tests/fixtures/tutorial/multi-agent-comparison/comparable.json").read_text(encoding="utf-8"))
    incomparable = json.loads((ROOT / "tests/fixtures/tutorial/multi-agent-comparison/incomparable.json").read_text(encoding="utf-8"))
    if set(comparable) != {"schema_version", "state", "benchmark_id", "benchmark_digest", "measure_set_digest", "revision", "generation", "runs", "compared_measures", "explanation"} or comparable["state"] != "comparable":
        raise Invalid("invalid comparable report shape")
    for key in ("benchmark_id", "revision"):
        synthetic(comparable[key], key)
    for key in ("benchmark_digest", "measure_set_digest"):
        digest(comparable[key], key)
    if len(comparable["runs"]) < 2 or any(set(run) != {"run_id", "agent_id", "passed_measures", "failed_measures"} for run in comparable["runs"]):
        raise Invalid("comparison must retain every selected run")
    if set(incomparable) != {"schema_version", "state", "benchmark_id", "runs", "mismatch", "ranked", "explanation"} or incomparable["state"] != "incomparable" or incomparable["ranked"] is not False:
        raise Invalid("incomparable report must refuse ranking")
    if incomparable["mismatch"] != "measure_set_digest" or len(incomparable["runs"]) < 2:
        raise Invalid("incomparable report lacks conservative mismatch")


def main() -> int:
    try:
        documents = [validate_tutorial(path) for path in TUTORIALS]
        if {doc["tutorial_id"] for doc in documents} != {"asb-tui-llm-record-replay", "asb-tui-multi-agent-comparison"}:
            raise Invalid("tutorial IDs do not cover both journeys")
        validate_fixtures()
    except (OSError, UnicodeError, json.JSONDecodeError, KeyError, TypeError, Invalid) as error:
        print(f"AR-1223 tutorial validation failed: {error}", file=sys.stderr)
        return 1
    print("AR-1223 record/replay and comparison tutorials valid offline")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
