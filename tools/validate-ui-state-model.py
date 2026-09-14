#!/usr/bin/env python3
# Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Fail-closed validation and change-ownership gate for the TUI model."""
import argparse
import datetime
import json
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
MODEL = ROOT / "docs" / "ui-state-model.json"
GENERATED = ROOT / "docs" / "ui-state-model.generated.json"
ROLES = {"action", "editable", "selectable", "navigation", "status"}
LAYOUTS = {"wide", "compact", "tiny"}
UI_OWNERS = {
    # State, projection, interaction, and terminal modules are the complete
    # standalone UI surface. Backend protocol and lifecycle modules are
    # intentionally excluded to avoid requiring model churn for unrelated
    # implementation changes.
    "src/actions.rs", "src/app.rs", "src/configuration.rs", "src/help.rs",
    "src/landing.rs", "src/live_projection.rs", "src/renderer.rs",
    "src/reports.rs", "src/runtime.rs", "src/selection.rs", "src/shell.rs",
    "src/startup.rs", "src/terminal.rs", "src/ui.rs", "src/visual.rs", "src/wizard.rs",
}
MODEL_FILES = {"docs/ui-state-model.json", "docs/ui-state-model.generated.json", "tools/generate-ui-state-model.py", "tools/validate-ui-state-model.py", "tools/test-ui-state-model.py"}
INVENTORY = ROOT / "docs" / "ui-module-inventory.json"
INVENTORY_FILE = "docs/ui-module-inventory.json"
INVENTORY_KINDS = {"state", "interaction", "projection", "rendering", "terminal", "startup"}
PATH_RE = re.compile(r"^src/[a-z0-9_]+\.rs$")


def validate(model):
    errors = []
    if not isinstance(model, dict) or model.get("schema_version") != 1:
        return ["model: schema_version must be 1"]
    if model.get("model") != "asb-tui.ui-state":
        errors.append("model: unexpected model identifier")
    routes, elements, transitions = (model.get(key) for key in ("routes", "elements", "transitions"))
    if not all(isinstance(value, list) and value for value in (routes, elements, transitions)):
        return errors + ["model: routes, elements, and transitions must be non-empty arrays"]
    if set(model.get("layouts", [])) != LAYOUTS:
        errors.append("model: layouts must declare exactly wide, compact, and tiny")
    fields = model.get("state_fields")
    if not isinstance(fields, list) or not fields or any(not isinstance(field, str) or not field for field in fields):
        errors.append("model: state_fields must be non-empty strings")
    route_ids = [item.get("id") for item in routes if isinstance(item, dict)]
    if len(route_ids) != len(set(route_ids)) or any(not isinstance(item, str) or not item for item in route_ids):
        errors.append("routes: duplicate or invalid route id")
    route_set, element_map = set(route_ids), {}
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
        for field in ("focusable", "hoverable"):
            if not isinstance(item.get(field), bool):
                errors.append(f"{ident}: {field} must be boolean")
        parent = item.get("parent")
        if parent is not None and (not isinstance(parent, str) or not parent):
            errors.append(f"{ident}: parent must be null or an element id")
        element_map[ident] = item
    for ident, item in element_map.items():
        parent = item.get("parent")
        if parent is not None and parent not in element_map:
            errors.append(f"{ident}: unknown parent {parent}")
        if parent == ident:
            errors.append(f"{ident}: element cannot parent itself")
    for ident in element_map:
        seen = set()
        current = ident
        while current is not None:
            if current in seen:
                errors.append(f"{ident}: parent relationship contains a cycle")
                break
            seen.add(current)
            current = element_map.get(current, {}).get("parent")
    help_map = {}
    entries = model.get("help")
    if not isinstance(entries, list) or not entries:
        errors.append("help: non-empty array is required")
    else:
        for entry in entries:
            ident = entry.get("id") if isinstance(entry, dict) else None
            prose = entry.get("text") if isinstance(entry, dict) else None
            if not isinstance(ident, str) or not ident or ident in help_map:
                errors.append(f"help: invalid or duplicate id {ident!r}")
                continue
            if not isinstance(prose, str) or len(prose.split()) < 4 or prose.strip().endswith("..."):
                errors.append(f"help.{ident}: text must be meaningful prose (at least four words)")
            help_map[ident] = entry
    for ident, item in element_map.items():
        if item.get("help_id") not in help_map:
            errors.append(f"{ident}: help_id does not reference a help entry")
    bindings = model.get("bindings")
    binding_ids = set()
    if not isinstance(bindings, list) or not bindings:
        errors.append("bindings: non-empty array is required")
    else:
        for binding in bindings:
            action = binding.get("action") if isinstance(binding, dict) else None
            element = binding.get("element") if isinstance(binding, dict) else None
            key = binding.get("key") if isinstance(binding, dict) else None
            if (not isinstance(action, str) or not action or action in binding_ids or
                    not isinstance(element, str) or element not in element_map or
                    not isinstance(key, str) or not key):
                errors.append(f"bindings: invalid or duplicate binding {binding!r}")
            else:
                binding_ids.add(action)
    listed = []
    for route in routes:
        if not isinstance(route, dict) or route.get("id") not in route_set:
            errors.append("routes: malformed route")
            continue
        route_id, route_elements = route["id"], route.get("elements")
        if not isinstance(route_elements, list) or not route_elements:
            errors.append(f"{route_id}: elements must be a non-empty array")
            continue
        for ident in route_elements:
            listed.append(ident)
            if ident not in element_map:
                errors.append(f"{route_id}: unknown element {ident}")
            elif element_map[ident].get("route") not in (route_id, "global"):
                errors.append(f"{ident}: element route does not match {route_id}")
    for ident in sorted(set(element_map) - set(listed)):
        errors.append(f"elements: {ident} is not visible on any route")
    graph, reverse = {ident: set() for ident in route_ids}, {ident: set() for ident in route_ids}
    for transition in transitions:
        if not isinstance(transition, dict):
            errors.append("transitions: entry must be an object")
            continue
        source, target, event = (transition.get(key) for key in ("from", "to", "event"))
        if not isinstance(event, str) or not event:
            errors.append("transition: event must be a non-empty stable id")
        if source not in graph or target not in graph:
            errors.append(f"transition: unknown route {source!r}->{target!r}")
        else:
            graph[source].add(target)
            reverse[target].add(source)
    initial = model.get("initial")
    if initial not in graph:
        errors.append("model: initial route is unknown")
    else:
        def closure(edges, start):
            reached, pending = {start}, [start]
            while pending:
                current = pending.pop()
                for target in edges[current]:
                    if target not in reached:
                        reached.add(target)
                        pending.append(target)
            return reached
        reached = closure(graph, initial)
        errors.extend(f"routes: unreachable route {ident}" for ident in sorted(set(graph) - reached))
        can_return = closure(reverse, initial)
        errors.extend(f"routes: {ident} has no escape path to {initial}" for ident in sorted(set(graph) - can_return))
    return sorted(errors)


def canonical(value):
    return json.dumps(value, indent=2, ensure_ascii=False) + "\n"


def changed_files(base, head):
    result = subprocess.run(["git", "diff", "--name-only", f"{base}..{head}"], cwd=ROOT, check=True, capture_output=True, text=True)
    return [line for line in result.stdout.splitlines() if line]


def validate_changes(files):
    changed, touched = set(files), sorted(set(files) & UI_OWNERS)
    if not touched:
        return []
    errors = []
    if not changed & MODEL_FILES:
        errors.append("UI files changed without a formal model update: " + ", ".join(touched))
    if "tools/test-ui-state-model.py" not in changed and not any(path.startswith("tests/") for path in changed):
        errors.append("UI files changed without a focused formal-model test update: " + ", ".join(touched))
    return errors


def validate_module_inventory(inventory, actual_paths, owners=None):
    """Compare the checked-in UI classification with the exact source tree."""
    owners = UI_OWNERS if owners is None else set(owners)
    errors = []
    if not isinstance(inventory, dict) or inventory.get("schema_version") != 1:
        return ["module inventory: schema_version must be 1"]
    ui_entries = inventory.get("ui_modules")
    exemptions = inventory.get("exemptions")
    if not isinstance(ui_entries, list) or not ui_entries:
        errors.append("module inventory: ui_modules must be a non-empty array")
        ui_entries = []
    if not isinstance(exemptions, list):
        errors.append("module inventory: exemptions must be an array")
        exemptions = []
    ui_paths, exemption_paths = [], []
    for entry in ui_entries:
        path = entry.get("path") if isinstance(entry, dict) else None
        kind = entry.get("kind") if isinstance(entry, dict) else None
        if path in ui_paths or not isinstance(path, str) or not PATH_RE.fullmatch(path):
            errors.append(f"module inventory: invalid or duplicate UI path {path!r}")
            continue
        if kind not in INVENTORY_KINDS:
            errors.append(f"module inventory: unknown kind for {path!r}")
        ui_paths.append(path)
    today = datetime.date.today()
    for entry in exemptions:
        path = entry.get("path") if isinstance(entry, dict) else None
        reason = entry.get("reason") if isinstance(entry, dict) else None
        expires = entry.get("expires") if isinstance(entry, dict) else None
        if path in exemption_paths or not isinstance(path, str) or not PATH_RE.fullmatch(path):
            errors.append(f"module inventory: invalid or duplicate exemption path {path!r}")
            continue
        if not isinstance(reason, str) or len(reason.split()) < 4 or not isinstance(expires, str):
            errors.append(f"module inventory: exemption {path!r} needs meaningful reason and expiry")
        else:
            try:
                if datetime.date.fromisoformat(expires) < today:
                    errors.append(f"module inventory: exemption {path!r} is expired")
            except ValueError:
                errors.append(f"module inventory: exemption {path!r} has invalid expiry")
        exemption_paths.append(path)
    if set(ui_paths) & set(exemption_paths):
        errors.append("module inventory: a path cannot be both UI and exempt")
    actual = set(actual_paths)
    if any(not isinstance(path, str) or not PATH_RE.fullmatch(path) for path in actual):
        errors.append("module inventory: actual source path is outside src/*.rs")
    classified = set(ui_paths) | set(exemption_paths)
    errors.extend(f"module inventory: unclassified source module {path}" for path in sorted(actual - classified))
    errors.extend(f"module inventory: stale classification for missing module {path}" for path in sorted(classified - actual))
    if set(ui_paths) != owners:
        errors.append("module inventory: UI modules and UI_OWNERS differ")
    return sorted(errors)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base", help="base revision for UI ownership checks")
    parser.add_argument("--head", default="HEAD", help="head revision for UI ownership checks")
    args = parser.parse_args()
    try:
        model = json.loads(MODEL.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        print(f"UI state model invalid: {error}", file=sys.stderr)
        return 1
    errors = validate(model)
    try:
        inventory = json.loads(INVENTORY.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        errors.append(f"module inventory invalid: {error}")
    else:
        actual_paths = [path.relative_to(ROOT).as_posix() for path in (ROOT / "src").glob("*.rs")]
        errors.extend(validate_module_inventory(inventory, actual_paths))
    if not GENERATED.exists():
        errors.append("generated model artifact is missing; run tools/generate-ui-state-model.py")
    else:
        try:
            generated = json.loads(GENERATED.read_text(encoding="utf-8"))
            if canonical(generated) != canonical(model):
                errors.append("generated model artifact is stale; run tools/generate-ui-state-model.py")
        except (OSError, json.JSONDecodeError) as error:
            errors.append(f"generated model artifact is invalid: {error}")
    if args.base:
        try:
            files = changed_files(args.base, args.head)
            errors.extend(validate_changes(files))
            source_changes = sorted(path for path in files if path.startswith("src/") and path.endswith(".rs"))
            if source_changes and INVENTORY_FILE not in files:
                errors.append("UI module changed without updating module inventory: " + ", ".join(source_changes))
        except (OSError, subprocess.CalledProcessError) as error:
            errors.append(f"cannot inspect changed UI files: {error}")
    if errors:
        print("UI state model validation failed:", file=sys.stderr)
        print("\n".join(f"- {error}" for error in sorted(errors)), file=sys.stderr)
        return 1
    print(f"UI state model valid: {len(model['routes'])} routes, {len(model['elements'])} elements")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
