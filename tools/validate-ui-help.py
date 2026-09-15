#!/usr/bin/env python3
# Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Validate the extensible, user-facing UI help catalog (AR-1037).

The JSON document is the authoring format.  This checker deliberately also
checks the closed action registry, so adding an action without adding help is
a deterministic CI failure.  Screen-specific widget inventories can be
extended without changing this validator; each entry is checked uniformly.
"""

import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CATALOG = ROOT / "docs" / "ui-help.json"
ACTION_SOURCE = ROOT / "src" / "actions.rs"
ROUTES = {"landing", "configuration", "measurement_selection", "run_control", "recent_runs", "reports", "help", "global"}
KINDS = {"screen", "action", "widget", "dialog", "status", "editable", "selectable"}
BAD_PHRASES = ("todo", "tbd", "lorem ipsum", "placeholder", "help text", "coming soon")
TOP_LEVEL_KEYS = {"schema_version", "catalog", "description", "elements"}
# Help is public documentation.  Reject values that would leak a host path or
# look like a credential even when they happen to satisfy the prose checks.
PRIVATE_MATERIAL = re.compile(
    r"(?:^|[\s=(])/(?:home|srv|tmp|run|etc|root)(?:[/\s)]|$)"
    r"|(?:gh[pousr]_|github_pat_|BEGIN (?:OPENSSH|RSA|EC) PRIVATE KEY)"
    r"|(?:api[_ -]?key|access[_ -]?token|secret)\s*[:=]",
    re.IGNORECASE,
)
ID_RE = re.compile(r"^[a-z][a-z0-9]*(?:[._-][a-z0-9]+)+$")


def _action_ids() -> set[str]:
    source = ACTION_SOURCE.read_text(encoding="utf-8")
    match = re.search(r"pub const ALL: \[Self; \d+\] = \[(.*?)\];", source, re.S)
    if not match:
        raise ValueError("cannot locate UiAction::ALL registry")
    variants = re.findall(r"Self::([A-Za-z][A-Za-z0-9_]*)", match.group(1))
    # Rust's action id conversion is intentionally mirrored here and checked
    # against the manifest.  A changed registry must update its documentation.
    return {"action." + re.sub(r"(?<!^)([A-Z])", r"_\1", variant).lower() for variant in variants}


def _meaningful(value: object, label: str, field: str, errors: list[str], element_id: str) -> None:
    if not isinstance(value, str):
        errors.append(f"{element_id}: help.{field} must be a string")
        return
    text = " ".join(value.split())
    if text != value or not (24 <= len(text) <= 240):
        errors.append(f"{element_id}: help.{field} must be 24-240 characters without padding")
    if len(text.split()) < 4:
        errors.append(f"{element_id}: help.{field} must contain at least four words")
    if text.casefold() == label.casefold():
        errors.append(f"{element_id}: help.{field} must explain the element, not repeat its label")
    lowered = text.casefold()
    if any(phrase in lowered for phrase in BAD_PHRASES):
        errors.append(f"{element_id}: help.{field} contains placeholder language")
    if PRIVATE_MATERIAL.search(text):
        errors.append(f"{element_id}: help.{field} contains private or secret material")


def validate_document(document: object, action_ids: set[str]) -> list[str]:
    errors: list[str] = []
    if not isinstance(document, dict):
        return ["catalog: top level must be an object"]
    unknown_keys = set(document) - TOP_LEVEL_KEYS
    if unknown_keys:
        errors.append(
            "catalog: unknown top-level fields: " + ",".join(sorted(unknown_keys))
        )
    if document.get("schema_version") != 1:
        errors.append("catalog: schema_version must be 1")
    if document.get("catalog") != "asb-tui.ui-help":
        errors.append("catalog: catalog must be asb-tui.ui-help")
    elements = document.get("elements")
    if not isinstance(elements, list) or not elements:
        return errors + ["catalog: elements must be a non-empty array"]
    seen: set[str] = set()
    for index, element in enumerate(elements):
        prefix = f"elements[{index}]"
        if not isinstance(element, dict):
            errors.append(f"{prefix}: entry must be an object")
            continue
        element_id = element.get("id")
        if not isinstance(element_id, str) or not (element_id == "navigation" or ID_RE.fullmatch(element_id)):
            errors.append(f"{prefix}: id must match dotted stable-id syntax")
            continue
        if element_id in seen:
            errors.append(f"{element_id}: duplicate id")
        seen.add(element_id)
        route, kind, label = element.get("route"), element.get("kind"), element.get("label")
        if route not in ROUTES:
            errors.append(f"{element_id}: route must be one of {','.join(sorted(ROUTES))}")
        if kind not in KINDS:
            errors.append(f"{element_id}: kind must be one of {','.join(sorted(KINDS))}")
        if element_id.startswith("action.") and kind != "action":
            errors.append(f"{element_id}: action ids must use kind action")
        if not isinstance(label, str) or not (1 <= len(label) <= 80) or not label.strip():
            errors.append(f"{element_id}: label must be a non-empty string of at most 80 characters")
        help_value = element.get("help")
        if not isinstance(help_value, dict):
            errors.append(f"{element_id}: help must be an object")
            continue
        if set(help_value) != {"summary", "usage"}:
            errors.append(f"{element_id}: help must contain exactly summary and usage")
        _meaningful(help_value.get("summary"), label if isinstance(label, str) else "", "summary", errors, element_id)
        _meaningful(help_value.get("usage"), label if isinstance(label, str) else "", "usage", errors, element_id)
    if action_ids:
        documented = {item.get("id") for item in elements if isinstance(item, dict)}
        for action_id in sorted(action_ids - documented):
            errors.append(f"{action_id}: registered action has no help entry")
        for action_id in sorted(
            item for item in documented if isinstance(item, str) and item.startswith("action.")
            and item not in action_ids
        ):
            errors.append(f"{action_id}: help entry does not match a registered action")
    return sorted(errors)


def main() -> int:
    try:
        document = json.loads(CATALOG.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        print(f"catalog: cannot read valid JSON: {error}", file=sys.stderr)
        return 1
    try:
        action_ids = _action_ids()
    except (OSError, ValueError) as error:
        print(f"action registry: cannot discover registered actions: {error}", file=sys.stderr)
        return 1
    errors = validate_document(document, action_ids)
    if errors:
        print("UI help catalog validation failed:", file=sys.stderr)
        print("\n".join(f"- {error}" for error in errors), file=sys.stderr)
        return 1
    print(f"UI help catalog valid: {len(document['elements'])} elements")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
