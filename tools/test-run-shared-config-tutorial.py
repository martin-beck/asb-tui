#!/usr/bin/env python3
# Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Validate AR-1222 tutorials and lifecycle fixtures without executing commands."""

from __future__ import annotations

import copy
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
TUTORIALS = [
    ROOT / "docs/tutorials/benchmark-run-v1.json",
    ROOT / "docs/tutorials/shared-agent-configuration-v1.json",
]
ID = re.compile(r"^[a-z][a-z0-9-]{0,63}$")
SYNTHETIC = re.compile(r"^synthetic-[a-z0-9-]+$")
SHELL = set(";&|<>`$(){}!\\\n\r")
SECRET = re.compile(r"(?i)(api[_-]?key|token|password|secret|bearer|credential)")


class Invalid(ValueError):
    pass


def string(value: object, name: str, maximum: int = 256) -> str:
    if not isinstance(value, str) or not value or len(value) > maximum:
        raise Invalid(f"{name} must be a bounded non-empty string")
    if any(char in value for char in SHELL) or SECRET.search(value):
        raise Invalid(f"{name} contains unsafe syntax or secret-like text")
    return value


def command(value: object, step_id: str) -> None:
    if not isinstance(value, list) or not 2 <= len(value) <= 16:
        raise Invalid(f"step {step_id} command must contain 2-16 arguments")
    args = [string(item, f"step {step_id} argument") for item in value]
    if args[:2] != ["asb", "tui"]:
        raise Invalid(f"step {step_id} must use the symbolic asb tui route")
    option_args = args[3:]
    if args[2] == "run":
        allowed = {
            ("--benchmark", "synthetic-benchmark", "--agent", "synthetic-agent"),
            ("--benchmark", "synthetic-benchmark", "--agent", "synthetic-agent", "--confirm"),
            ("--status", "synthetic-run-001"),
            ("--cancel", "synthetic-run-001"),
            ("--recover", "synthetic-run-001"),
        }
    elif args[2:4] == ["configure", "agents"]:
        option_args = args[4:]
        allowed = {
            ("--agents", "synthetic-agent-a,synthetic-agent-b"),
            ("--agents", "synthetic-agent-a,synthetic-agent-b", "--provider", "synthetic-provider",
             "--model", "synthetic-model", "--benchmark", "synthetic-benchmark"),
            ("--agents", "synthetic-agent-a,synthetic-agent-b", "--provider", "synthetic-provider",
             "--model", "synthetic-model", "--benchmark", "synthetic-benchmark", "--apply"),
        }
    else:
        raise Invalid(f"step {step_id} has an unknown route")
    if tuple(option_args) not in allowed:
        raise Invalid(f"step {step_id} has unsupported or reordered options")
    for arg in option_args:
        if arg.startswith("--"):
            continue
        if "," in arg:
            if any(not SYNTHETIC.fullmatch(identity) for identity in arg.split(",")):
                raise Invalid(f"step {step_id} has a non-synthetic agent selection")
        elif arg.startswith("synthetic-"):
            if not SYNTHETIC.fullmatch(arg):
                raise Invalid(f"step {step_id} has a non-synthetic identity")
        else:
            raise Invalid(f"step {step_id} has a non-synthetic identity")


def validate_tutorial(path: Path) -> dict:
    document = json.loads(path.read_text(encoding="utf-8"))
    validate_tutorial_document(document, path.name)
    return document


def validate_tutorial_document(document: dict, label: str = "tutorial") -> None:
    if set(document) != {"schema_version", "tutorial_id", "title", "steps"}:
        raise Invalid(f"{label} has an invalid closed top-level shape")
    if document["schema_version"] != 1 or not ID.fullmatch(string(document["tutorial_id"], "tutorial ID")):
        raise Invalid(f"{label} has an invalid version or ID")
    string(document["title"], "tutorial title")
    steps = document["steps"]
    if not isinstance(steps, list) or not steps:
        raise Invalid(f"{label} must contain steps")
    seen = set()
    for step in steps:
        if set(step) != {"id", "command", "expect", "references", "network", "credentials"}:
            raise Invalid(f"{label} has an invalid step shape")
        step_id = string(step["id"], "step ID")
        if not ID.fullmatch(step_id) or step_id in seen:
            raise Invalid(f"{label} has duplicate or invalid step IDs")
        seen.add(step_id)
        command(step["command"], step_id)
        expect = step["expect"]
        if set(expect) != {"exit_code", "stdout_shape"} or not isinstance(expect["exit_code"], int):
            raise Invalid(f"{label} has an invalid expectation")
        string(expect["stdout_shape"], "stdout shape", 64)
        if step["network"] != "denied" or step["credentials"] != "none":
            raise Invalid(f"{label} must be offline and credential-free")
        for reference in step["references"]:
            if set(reference) != {"path", "kind"} or not isinstance(reference["path"], str):
                raise Invalid(f"{label} has an invalid reference")
            ref = reference["path"]
            if ref.startswith(("/", "~")) or ".." in ref.split("/") or reference["kind"] not in {"input", "output", "fixture"}:
                raise Invalid(f"{label} has an unsafe reference")
    return document


def fixtures() -> None:
    run_required = {"schema_version", "state", "run_id", "benchmark_id", "agent_id", "completed_measures", "total_measures", "checkpoint", "explanation"}
    run_states = set()
    for path in sorted((ROOT / "tests/fixtures/tutorial/benchmark-run").glob("*.json")):
        value = json.loads(path.read_text(encoding="utf-8"))
        if set(value) != run_required or value["schema_version"] != 1:
            raise Invalid(f"invalid benchmark fixture: {path.name}")
        if value["state"] not in {"running", "cancelled", "recovered"} or not all(SYNTHETIC.fullmatch(value[k]) for k in ("run_id", "benchmark_id", "agent_id")):
            raise Invalid(f"invalid benchmark fixture values: {path.name}")
        if not isinstance(value["explanation"], str) or len(value["explanation"]) < 20:
            raise Invalid(f"benchmark fixture explanation is not meaningful: {path.name}")
        run_states.add(value["state"])
    if run_states != {"running", "cancelled", "recovered"}:
        raise Invalid("benchmark fixtures must cover running, cancelled, and recovered states")

    config_required = {"schema_version", "state", "selected_agents", "configuration", "changed_agents", "explanation"}
    config_states = set()
    for path in sorted((ROOT / "tests/fixtures/tutorial/shared-agent-configuration").glob("*.json")):
        value = json.loads(path.read_text(encoding="utf-8"))
        if set(value) != config_required and set(value) != config_required | {"incompatible_agents"}:
            raise Invalid(f"invalid shared-config fixture: {path.name}")
        if value["schema_version"] != 1 or value["state"] not in {"applied", "refused"}:
            raise Invalid(f"invalid shared-config state: {path.name}")
        if value["selected_agents"] != ["synthetic-agent-a", "synthetic-agent-b"]:
            raise Invalid(f"shared-config selection is not stable: {path.name}")
        config = value["configuration"]
        if set(config) != {"provider", "model", "benchmark"} or any(set(item) != {"value", "origin"} for item in config.values()):
            raise Invalid(f"configuration origins are incomplete: {path.name}")
        if any(item["origin"] not in {"explicit", "saved", "default"} or not SYNTHETIC.fullmatch(item["value"]) for item in config.values()):
            raise Invalid(f"configuration origin/value is invalid: {path.name}")
        if value["state"] == "applied" and value["changed_agents"] != 2:
            raise Invalid("atomic success must change all selected agents")
        if value["state"] == "refused":
            if value["changed_agents"] != 0 or not value.get("incompatible_agents"):
                raise Invalid("atomic incompatibility must leave every agent unchanged")
        if not isinstance(value["explanation"], str) or len(value["explanation"]) < 20:
            raise Invalid(f"shared-config explanation is not meaningful: {path.name}")
        config_states.add(value["state"])
    if config_states != {"applied", "refused"}:
        raise Invalid("shared-config fixtures must cover applied and refused states")


def main() -> int:
    try:
        documents = [validate_tutorial(path) for path in TUTORIALS]
        if {doc["tutorial_id"] for doc in documents} != {"asb-tui-benchmark-run", "asb-tui-shared-agent-configuration"}:
            raise Invalid("tutorial IDs do not cover both AR-1222 journeys")
        bad = copy.deepcopy(documents[0])
        bad["steps"][0]["network"] = "allowed"
        try:
            validate_tutorial_document(bad, "negative tutorial")
        except Invalid:
            pass
        else:
            raise Invalid("negative network case was accepted")
        fixtures()
    except (OSError, UnicodeError, json.JSONDecodeError, Invalid) as error:
        print(f"AR-1222 tutorial validation failed: {error}", file=sys.stderr)
        return 1
    print("AR-1222 tutorials and offline lifecycle fixtures valid")
    return 0

if __name__ == "__main__":
    raise SystemExit(main())
