#!/usr/bin/env python3
# Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Check every asb-tui tutorial against the versioned ASB command grammar.

This is deliberately a parser-only gate.  It does not import or execute ASB,
the TUI, providers, benchmark commands, or network clients.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path
from typing import Any

MAX_BYTES = 256 * 1024
ID = re.compile(r"^[A-Za-z][A-Za-z0-9._-]{0,63}$")
SHA256 = re.compile(r"^[0-9a-f]{64}$")
SHELL = set(";&|<>`$(){}!\\\n\r")
SECRET = re.compile(r"(?i)(api[_-]?key|token|password|secret|bearer|sk-[a-z0-9])")

TUI_OPERATIONS = {
    "status", "doctor", "run", "record", "replay", "compare", "configure",
    "launch", "remove", "install", "upgrade",
}


class Invalid(ValueError):
    pass


def load(path: Path, label: str) -> Any:
    if path.is_symlink() or not path.is_file() or path.stat().st_size > MAX_BYTES:
        raise Invalid(f"{label} is not a bounded regular file")
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as exc:
        raise Invalid(f"{label} is not valid UTF-8 JSON") from exc


def string(value: Any, label: str, limit: int = 256) -> str:
    if not isinstance(value, str) or not value or len(value) > limit:
        raise Invalid(f"{label} must be a bounded non-empty string")
    if SECRET.search(value):
        raise Invalid(f"{label} appears to contain secret material")
    return value


def safe_arg(value: Any, label: str) -> str:
    value = string(value, label)
    if any(char in value for char in SHELL):
        raise Invalid(f"{label} contains shell or control syntax")
    return value


def path_arg(value: str, label: str) -> None:
    if value.startswith(("-", "/", "~")) or ".." in value.split("/") or "://" in value:
        raise Invalid(f"{label} must be a repository-relative reference")


def metadata(document: Any) -> dict[str, Any]:
    if not isinstance(document, dict) or set(document) != {"schema_version", "executable", "commands"}:
        raise Invalid("ASB command metadata has an invalid closed shape")
    if document["schema_version"] != 1 or document["executable"] != "asb":
        raise Invalid("ASB command metadata version or executable is unsupported")
    commands = document["commands"]
    if not isinstance(commands, dict) or not commands:
        raise Invalid("ASB command metadata has no commands")
    for name, spec in commands.items():
        if not isinstance(name, str) or not ID.fullmatch(name) or not isinstance(spec, dict):
            raise Invalid("ASB command metadata contains an invalid command")
        forms = spec.get("forms")
        minimum = spec.get("min_args")
        operations = spec.get("operations")
        if sum(value is not None for value in (forms, minimum, operations)) != 1:
            raise Invalid(f"ASB command metadata for {name} is ambiguous")
        if forms is not None and (not isinstance(forms, list) or not forms or
                                  any(not isinstance(form, list) or len(form) > 16 or
                                      any(not isinstance(token, str) or not token for token in form)
                                      for form in forms)):
            raise Invalid(f"ASB command metadata forms for {name} are invalid")
        if minimum is not None and (not isinstance(minimum, int) or not 0 < minimum <= 16):
            raise Invalid(f"ASB command metadata minimum for {name} is invalid")
        if operations is not None and (not isinstance(operations, dict) or not operations or
                                       any(not ID.fullmatch(operation) or
                                           not isinstance(options, list) or not options or
                                           any(not isinstance(option, str) or not option.startswith("--")
                                               for option in options)
                                           for operation, options in operations.items())):
            raise Invalid(f"ASB command metadata operations for {name} are invalid")
    return document


def validate_asb_command(command: Any, commands: dict[str, Any], step: str) -> None:
    if not isinstance(command, list) or not 2 <= len(command) <= 16:
        raise Invalid(f"step {step} command must be a bounded argument array")
    args = [safe_arg(value, f"step {step} command argument") for value in command]
    if args[0] != "asb":
        raise Invalid(f"step {step} must invoke the symbolic ASB executable")
    root = args[1]
    if root == "tui":
        if len(args) < 3 or args[2] not in TUI_OPERATIONS:
            raise Invalid(f"step {step} uses an unknown asb-tui operation")
        # TUI snippets are symbolic examples; preserve option order and reject
        # shell/path escapes without pretending to execute them.
        if args[2] == "configure" and (len(args) < 4 or args[3] != "agents"):
            raise Invalid(f"step {step} configure route is incomplete")
        if any(arg.startswith("--") and not re.fullmatch(r"--[a-z][a-z0-9-]*", arg) for arg in args[3:]):
            raise Invalid(f"step {step} contains an invalid TUI option")
        return
    if root not in commands:
        raise Invalid(f"step {step} uses unknown ASB command {root}")
    spec = commands[root]
    tail = args[2:]
    if "min_args" in spec:
        if len(tail) < spec["min_args"]:
            raise Invalid(f"step {step} has too few arguments for {root}")
        return
    if "operations" in spec:
        if not tail or tail[0] not in spec["operations"]:
            raise Invalid(f"step {step} uses an unknown {root} operation")
        options = spec["operations"][tail[0]]
        actual = tail[1:]
        if len(actual) != len(options) * 2 or any(actual[2 * i] != option for i, option in enumerate(options)):
            raise Invalid(f"step {step} {root} options are missing, unknown, or reordered")
        return
    for form in spec["forms"]:
        if not isinstance(form, list) or len(tail) != len(form):
            continue
        valid = True
        for value, expected in zip(tail, form):
            if expected == "PATH":
                path_arg(value, f"step {step} path")
            elif expected == "SHA256":
                valid &= bool(SHA256.fullmatch(value))
            elif expected == "AGENT":
                valid &= bool(ID.fullmatch(value))
            elif value != expected:
                valid = False
        if valid:
            return
    raise Invalid(f"step {step} arguments do not match the versioned ASB grammar")


def validate_tutorial(path: Path, product: Path, asb: Path, commands: dict[str, Any]) -> None:
    document = load(path, str(path))
    if not isinstance(document, dict) or set(document) - {"schema_version", "tutorial_id", "title", "steps", "network", "credentials"}:
        raise Invalid(f"{path.name} has an invalid closed tutorial shape")
    if document["schema_version"] != 1 or not ID.fullmatch(string(document["tutorial_id"], "tutorial_id")):
        raise Invalid(f"{path.name} has an invalid version or ID")
    string(document["title"], "tutorial title")
    if document.get("network", "denied") != "denied" or document.get("credentials", "none") != "none":
        raise Invalid(f"{path.name} is not offline and credential-free")
    steps = document["steps"]
    if not isinstance(steps, list) or not 1 <= len(steps) <= 64:
        raise Invalid(f"{path.name} must contain between 1 and 64 steps")
    seen: set[str] = set()
    for raw in steps:
        if not isinstance(raw, dict) or set(raw) - {"id", "command", "expect", "references", "network", "credentials"}:
            raise Invalid(f"{path.name} contains an unknown step field")
        step_id = string(raw.get("id"), "step ID")
        if not ID.fullmatch(step_id) or step_id in seen:
            raise Invalid(f"{path.name} contains duplicate or invalid step ID")
        seen.add(step_id)
        validate_asb_command(raw.get("command"), commands, step_id)
        expect = raw.get("expect")
        if isinstance(expect, str):
            string(expect, f"step {step_id} output shape", 64)
        elif isinstance(expect, dict) and set(expect) == {"exit_code", "stdout_shape"}:
            if not isinstance(expect["exit_code"], int) or not 0 <= expect["exit_code"] <= 125:
                raise Invalid(f"step {step_id} has an invalid exit code")
            string(expect["stdout_shape"], f"step {step_id} output shape", 64)
        else:
            raise Invalid(f"step {step_id} has an invalid expected result")
        if raw.get("network", "denied") != "denied" or raw.get("credentials", "none") != "none":
            raise Invalid(f"step {step_id} is not offline and credential-free")
        references = raw.get("references", [])
        if not isinstance(references, list) or len(references) > 16:
            raise Invalid(f"step {step_id} references are unbounded")
        for reference in references:
            if not isinstance(reference, dict) or set(reference) != {"path", "kind"}:
                raise Invalid(f"step {step_id} has an invalid reference")
            relative = string(reference["path"], "reference path")
            path_arg(relative, "reference path")
            if reference["kind"] not in {"input", "output", "config", "fixture"}:
                raise Invalid(f"step {step_id} has an invalid reference kind")
            # A reference may belong to either repository.  Resolve only
            # beneath those two checked-out roots; never consult the host.
            local = product / relative
            upstream = asb / relative
            if not (local.is_file() or upstream.is_file()):
                raise Invalid(f"step {step_id} references missing file {relative}")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--asb-root", type=Path, required=True,
                        help="checked-out immutable ASB contract tree")
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1])
    args = parser.parse_args(argv)
    try:
        asb = args.asb_root.resolve()
        product = args.root.resolve()
        metadata_path = asb / "tools/tutorials/command_metadata_v1.json"
        commands = metadata(load(metadata_path, "ASB command metadata"))["commands"]
        tutorials = sorted((product / "docs/tutorials").glob("*-v1.json"))
        if not tutorials:
            raise Invalid("no tutorial contracts were discovered")
        for tutorial in tutorials:
            validate_tutorial(tutorial, product, asb, commands)
            markdown = tutorial.with_name(tutorial.name.removesuffix("-v1.json") + ".md")
            if not markdown.is_file():
                raise Invalid(f"{tutorial.name} has no paired Markdown tutorial")
        print(f"tutorial freshness valid: {len(tutorials)} contracts; offline syntax only")
        return 0
    except (Invalid, OSError) as exc:
        print(f"tutorial freshness failed: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
