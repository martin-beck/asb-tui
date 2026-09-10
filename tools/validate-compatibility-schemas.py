#!/usr/bin/env python3
# Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Validate compatibility fixtures against the committed closed JSON Schemas."""

from __future__ import annotations

import json
import re
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]


def same_json(left: Any, right: Any) -> bool:
    return type(left) is type(right) and left == right


def type_matches(value: Any, expected: str) -> bool:
    return {
        "array": isinstance(value, list),
        "boolean": type(value) is bool,
        "integer": type(value) is int,
        "null": value is None,
        "object": isinstance(value, dict),
        "string": isinstance(value, str),
    }[expected]


def validate(schema: dict[str, Any], value: Any, location: str = "$") -> None:
    if "anyOf" in schema:
        matches = 0
        for candidate in schema["anyOf"]:
            try:
                validate(candidate, value, location)
                matches += 1
            except ValueError:
                pass
        if matches != 1:
            raise ValueError(f"{location}: expected exactly one anyOf match")
    expected = schema.get("type")
    if expected is not None:
        types = [expected] if isinstance(expected, str) else expected
        if not any(type_matches(value, item) for item in types):
            raise ValueError(f"{location}: wrong JSON type")
    if "const" in schema and not same_json(value, schema["const"]):
        raise ValueError(f"{location}: const mismatch")
    if "enum" in schema and not any(same_json(value, item) for item in schema["enum"]):
        raise ValueError(f"{location}: enum mismatch")
    if isinstance(value, dict):
        properties = schema.get("properties", {})
        missing = set(schema.get("required", [])) - value.keys()
        if missing:
            raise ValueError(f"{location}: missing required members")
        if schema.get("additionalProperties") is False and set(value) - properties.keys():
            raise ValueError(f"{location}: additional member")
        for key, child in value.items():
            if key in properties:
                validate(properties[key], child, f"{location}.{key}")
    if isinstance(value, list):
        if len(value) < schema.get("minItems", 0) or len(value) > schema.get("maxItems", len(value)):
            raise ValueError(f"{location}: array length")
        for index, child in enumerate(value):
            validate(schema.get("items", {}), child, f"{location}[{index}]")
        if schema.get("uniqueItems") and len({json.dumps(item, sort_keys=True) for item in value}) != len(value):
            raise ValueError(f"{location}: duplicate array member")
    if isinstance(value, str):
        if len(value) < schema.get("minLength", 0) or len(value) > schema.get("maxLength", len(value)):
            raise ValueError(f"{location}: string length")
        if "pattern" in schema and re.fullmatch(schema["pattern"], value) is None:
            raise ValueError(f"{location}: pattern mismatch")
    if type(value) is int:
        if value < schema.get("minimum", value) or value > schema.get("maximum", value):
            raise ValueError(f"{location}: numeric range")


def load(relative: str) -> Any:
    return json.loads((ROOT / relative).read_text(encoding="utf-8"))


def main() -> None:
    probe_schema = load("protocol/v1/compatibility-probe.schema.json")
    report_schema = load("protocol/v1/compatibility-report.schema.json")
    validate(probe_schema, load("tests/fixtures/compatibility/compatible.json"))
    for fixture in ["mismatch.json", "malformed.json"]:
        try:
            validate(probe_schema, load(f"tests/fixtures/compatibility/{fixture}"))
        except ValueError:
            pass
        else:
            raise ValueError(f"negative compatibility fixture unexpectedly validates: {fixture}")
    for fixture in ["report-compatible.json", "report-unsupported.json"]:
        validate(report_schema, load(f"tests/fixtures/compatibility/{fixture}"))
    validate(
        load("protocol/v1/bundle-manifest.schema.json"),
        load("tests/fixtures/bundle/manifest.json"),
    )


if __name__ == "__main__":
    main()
