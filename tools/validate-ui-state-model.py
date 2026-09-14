#!/usr/bin/env python3
# Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Validate the renderer-independent UI state model (AR-1182 foundation)."""
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
MODEL = ROOT / "docs" / "ui-state-model.json"
ROLES = {"action", "editable", "selectable", "navigation", "status"}


def validate(model):
    errors = []
    if not isinstance(model, dict) or model.get("schema_version") != 1:
        return ["model: schema_version must be 1"]
    if model.get("model") != "asb-tui.ui-state":
        errors.append("model: unexpected model identifier")
    routes = model.get("routes")
    elements = model.get("elements")
    transitions = model.get("transitions")
    if not all(isinstance(value, list) and value for value in (routes, elements, transitions)):
        return errors + ["model: routes, elements, and transitions must be non-empty arrays"]
    route_ids = [item.get("id") for item in routes if isinstance(item, dict)]
    if len(route_ids) != len(set(route_ids)):
        errors.append("routes: duplicate route id")
    element_map = {}
    for item in elements:
        if not isinstance(item, dict):
            errors.append("elements: entry must be an object")
            continue
        ident = item.get("id")
        if not isinstance(ident, str) or not ident or ident in element_map:
            errors.append(f"elements: invalid or duplicate id {ident!r}")
            continue
        if item.get("role") not in ROLES:
            errors.append(f"{ident}: unknown role")
        if not isinstance(item.get("help_id"), str) or not item["help_id"]:
            errors.append(f"{ident}: missing help_id")
        element_map[ident] = item
    for route in routes:
        if not isinstance(route, dict) or route.get("id") not in route_ids:
            errors.append("routes: malformed route")
            continue
        for ident in route.get("elements", []):
            if ident not in element_map:
                errors.append(f"{route['id']}: unknown element {ident}")
            elif element_map[ident].get("route") not in (route["id"], "global"):
                errors.append(f"{ident}: element route does not match {route['id']}")
    graph = {ident: set() for ident in route_ids}
    for transition in transitions:
        if not isinstance(transition, dict):
            errors.append("transitions: entry must be an object")
            continue
        source, target = transition.get("from"), transition.get("to")
        if source not in graph or target not in graph:
            errors.append(f"transition: unknown route {source!r}->{target!r}")
        else:
            graph[source].add(target)
    initial = model.get("initial")
    if initial not in graph:
        errors.append("model: initial route is unknown")
    else:
        reached = {initial}
        pending = [initial]
        while pending:
            current = pending.pop()
            for target in graph[current]:
                if target not in reached:
                    reached.add(target)
                    pending.append(target)
        missing = sorted(set(graph) - reached)
        errors.extend(f"routes: unreachable route {ident}" for ident in missing)
    return sorted(errors)


def main():
    try:
        model = json.loads(MODEL.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        print(f"UI state model invalid: {error}", file=sys.stderr)
        return 1
    errors = validate(model)
    if errors:
        print("UI state model validation failed:", file=sys.stderr)
        print("\n".join(f"- {error}" for error in errors), file=sys.stderr)
        return 1
    print(f"UI state model valid: {len(model['routes'])} routes, {len(model['elements'])} elements")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
