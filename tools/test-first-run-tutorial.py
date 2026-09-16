#!/usr/bin/env python3
# Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Validate the first-run tutorial as a bounded offline ASB tutorial contract."""

from __future__ import annotations

import copy
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
TUTORIAL = ROOT / "docs/tutorials/first-run-agent-v1.json"
MAX_BYTES = 256 * 1024
ID = re.compile(r"^[A-Za-z][A-Za-z0-9._-]{0,63}$")
SHA256 = re.compile(r"^[0-9a-f]{64}$")
SECRET = re.compile(r"(?i)(api[_-]?key|token|password|secret|bearer|sk-[a-z0-9])")
SHELL = set(";&|<>`$(){}!\\\n\r")

COMMANDS = {
    "capabilities": [("--format", "json")],
    "provider-catalog": [()],
    "tui": [("launch",), ("status",), ("doctor",), ("remove",), ("install",),
            ("install", "--offline"), ("install", "--dry-run"), ("install", "--launch"),
            ("upgrade",), ("upgrade", "--offline"), ("upgrade", "--dry-run"),
            ("upgrade", "--launch")],
}
AUTH = {
    "status": ("--provider",),
    "enroll": ("--provider", "--endpoint-digest", "--credential-digest", "--idempotency-key"),
    "rotate": ("--provider", "--credential-digest", "--idempotency-key"),
    "revoke": ("--provider", "--idempotency-key"),
}


class Invalid(ValueError):
    """A deterministic contract violation."""


def bounded_string(value: object, name: str, maximum: int = 256) -> str:
    if not isinstance(value, str) or not value or len(value) > maximum:
        raise Invalid(f"{name} must be a bounded non-empty string")
    if SECRET.search(value):
        raise Invalid(f"{name} appears to contain secret-like material")
    return value


def safe_arg(value: object, name: str) -> str:
    text = bounded_string(value, name)
    if any(char in text for char in SHELL):
        raise Invalid(f"{name} contains shell or control syntax")
    return text


def safe_path(value: str, name: str) -> None:
    if value.startswith(("-", "/", "~")) or ".." in value.split("/") or "://" in value:
        raise Invalid(f"{name} must be repository-relative")


def validate_command(command: object, step_id: str) -> None:
    if not isinstance(command, list) or not 2 <= len(command) <= 16:
        raise Invalid(f"step {step_id} command must contain 2-16 arguments")
    args = [safe_arg(value, f"step {step_id} argument") for value in command]
    if args[0] != "asb":
        raise Invalid(f"step {step_id} must invoke symbolic asb")
    root, tail = args[1], args[2:]
    if root == "auth":
        if not tail or tail[0] not in AUTH:
            raise Invalid(f"step {step_id} has unknown auth operation")
        expected = AUTH[tail[0]]
        actual = tail[1:]
        if len(actual) != 2 * len(expected) or tuple(actual[0::2]) != expected:
            raise Invalid(f"step {step_id} auth options are missing or reordered")
        for option, value in zip(expected, actual[1::2]):
            if option.endswith("digest") and not SHA256.fullmatch(value):
                raise Invalid(f"step {step_id} has invalid {option} value")
        return
    if root == "doctor":
        if tail:
            raise Invalid(f"step {step_id} doctor accepts no arguments")
        return
    if root not in COMMANDS:
        raise Invalid(f"step {step_id} uses unknown command")
    if root == "tui":
        if tuple(tail) not in COMMANDS[root]:
            raise Invalid(f"step {step_id} has unsupported tui route/options")
        return
    if tuple(tail) not in COMMANDS[root]:
        raise Invalid(f"step {step_id} has unknown or reordered options")


def validate(document: object) -> None:
    if not isinstance(document, dict) or set(document) != {"schema_version", "tutorial_id", "title", "steps"}:
        raise Invalid("tutorial has an invalid closed top-level shape")
    if document["schema_version"] != 1 or not ID.fullmatch(bounded_string(document["tutorial_id"], "tutorial_id")):
        raise Invalid("tutorial version or ID is invalid")
    bounded_string(document["title"], "title")
    steps = document["steps"]
    if not isinstance(steps, list) or not 1 <= len(steps) <= 64:
        raise Invalid("steps must contain 1-64 entries")
    seen: set[str] = set()
    for raw in steps:
        if not isinstance(raw, dict) or not set(raw) <= {"id", "command", "expect", "references", "network", "credentials"}:
            raise Invalid("step has unknown fields")
        step_id = bounded_string(raw.get("id"), "step id")
        if not ID.fullmatch(step_id) or step_id in seen:
            raise Invalid("step IDs must be unique and stable")
        seen.add(step_id)
        validate_command(raw.get("command"), step_id)
        expect = raw.get("expect")
        if not isinstance(expect, dict) or set(expect) != {"exit_code", "stdout_shape"}:
            raise Invalid(f"step {step_id} expectation is not closed")
        if not isinstance(expect["exit_code"], int) or not 0 <= expect["exit_code"] <= 125:
            raise Invalid(f"step {step_id} exit code is invalid")
        bounded_string(expect["stdout_shape"], f"step {step_id} output shape", 64)
        if raw.get("network", "denied") != "denied" or raw.get("credentials", "none") != "none":
            raise Invalid(f"step {step_id} is not offline and credential-free")
        references = raw.get("references", [])
        if not isinstance(references, list) or len(references) > 16:
            raise Invalid(f"step {step_id} references are unbounded")
        for reference in references:
            if not isinstance(reference, dict) or set(reference) != {"path", "kind"}:
                raise Invalid(f"step {step_id} reference is not closed")
            path = bounded_string(reference["path"], "reference path")
            safe_path(path, "reference path")
            if reference["kind"] not in {"input", "output", "config", "fixture"}:
                raise Invalid(f"step {step_id} reference kind is invalid")


def main() -> int:
    if TUTORIAL.is_symlink() or not TUTORIAL.is_file() or TUTORIAL.stat().st_size > MAX_BYTES:
        print("tutorial validation failed: file is not a bounded regular file", file=sys.stderr)
        return 1
    try:
        document = json.loads(TUTORIAL.read_text(encoding="utf-8"))
        validate(document)
        bad = copy.deepcopy(document)
        bad["steps"][0]["network"] = "allowed"
        try:
            validate(bad)
        except Invalid:
            pass
        else:
            raise Invalid("negative network fixture was accepted")
        bad = copy.deepcopy(document)
        bad["steps"][0]["command"] = ["asb", "capabilities", "json", "--format"]
        try:
            validate(bad)
        except Invalid:
            pass
        else:
            raise Invalid("negative reordered-option fixture was accepted")
    except (OSError, UnicodeError, json.JSONDecodeError, Invalid) as error:
        print(f"tutorial validation failed: {error}", file=sys.stderr)
        return 1
    print(f"valid offline first-run tutorial: {TUTORIAL}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
