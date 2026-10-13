#!/usr/bin/env python3
# Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Fail closed when visual fixtures drift from UI ownership and formal contracts."""

import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
MANIFEST = ROOT / "docs/ui-visual-regression.json"


def load_json(path):
    return json.loads(path.read_text(encoding="utf-8"))


def validate(root=ROOT):
    errors = []
    manifest = load_json(root / "docs/ui-visual-regression.json")
    model = load_json(root / "docs/ui-state-model.json")
    generated = load_json(root / "docs/ui-state-model.generated.json")
    help_doc = load_json(root / "docs/ui-help.json")
    inventory = load_json(root / "docs/ui-module-inventory.json")
    if manifest.get("schema_version") != 1:
        errors.append("visual manifest schema_version must be 1")
    if generated != model:
        errors.append("generated formal model differs from authored model")

    dimensions = {item["id"]: (item["columns"], item["lines"]) for item in manifest["dimensions"]}
    expected_dimensions = {
        "wide": (140, 36),
        "ordinary": (100, 24),
        "narrow": (60, 16),
        "minimum": (32, 8),
    }
    if dimensions != expected_dimensions:
        errors.append(f"dimension matrix mismatch: {dimensions!r}")

    capabilities = {item["id"]: item for item in manifest["capabilities"]}
    if set(capabilities) != {"color_unicode", "mono_unicode", "ascii_plain"}:
        errors.append("capability matrix must cover color, monochrome, and ASCII")
    if capabilities.get("ascii_plain", {}).get("unicode") is not False:
        errors.append("ASCII capability must disable Unicode")

    model_routes = {item["id"] for item in model["routes"]}
    fixture_routes = {item["id"] for item in manifest["routes"]}
    if fixture_routes != model_routes:
        errors.append(
            f"route coverage mismatch: missing={sorted(model_routes-fixture_routes)} extra={sorted(fixture_routes-model_routes)}"
        )
    for route in manifest["routes"]:
        states = route.get("states", [])
        if not states or len(states) != len(set(states)):
            errors.append(f"route {route[id]} needs unique visual states")

    model_elements = {item["id"] for item in model["elements"]}
    model_help = {item["help_id"] for item in model["elements"]}
    help_ids = {item["id"] for item in help_doc["elements"]}
    if not model_help <= help_ids:
        errors.append(f"visual elements without help: {sorted(model_help-help_ids)}")
    inventory_modules = {item["path"] for item in inventory["ui_modules"]}
    owner_source = (root / "tools/validate-ui-state-model.py").read_text(encoding="utf-8")
    owner_match = re.search(r"UI_OWNERS\s*=\s*\{(.*?)\n\}", owner_source, re.S)
    if not owner_match:
        errors.append("UI_OWNERS registry is missing")
    else:
        owners = set(re.findall(r'"(src/[a-z0-9_]+\.rs)"', owner_match.group(1)))
        if owners != inventory_modules:
            errors.append(
                f"UI_OWNERS inventory drift: owner-only={sorted(owners-inventory_modules)} inventory-only={sorted(inventory_modules-owners)}"
            )

    source = (root / "src/ui.rs").read_text(encoding="utf-8")
    screen_match = re.search(r"pub enum Screen\s*\{(.*?)\n\}", source, re.S)
    if not screen_match:
        errors.append("missing executable Screen registry")
    else:
        screens = {
            line.strip().rstrip(",")
            for line in screen_match.group(1).splitlines()
            if line.strip()
        }
        expected_screens = {
            "Landing", "DevelopmentHandoff", "Wizard", "Measures",
            "Configuration", "RunControl", "Reports", "Help",
        }
        if screens != expected_screens:
            errors.append(f"Screen registry drift: {sorted(screens ^ expected_screens)}")

    action_source = (root / "src/actions.rs").read_text(encoding="utf-8")
    action_match = re.search(r"pub const fn id\(self\).*?\n    \}", action_source, re.S)
    if not action_match:
        errors.append("typed key/action registry is missing")
    else:
        actions = set(re.findall(r'Self::\w+\s*=>\s*"([^"]+)"', action_match.group(0)))
        bindings = {item["action"] for item in model["bindings"]}
        if not actions <= bindings:
            errors.append(f"typed key actions missing from formal bindings: {sorted(actions-bindings)}")

    route_states = {item["id"]: set(item["states"]) for item in manifest["routes"]}
    golden_ids = set()
    used_dimensions = set()
    used_capabilities = set()
    used_routes = set()
    forbidden = [item.casefold() for item in manifest["privacy_forbidden"]]
    for golden in manifest["goldens"]:
        gid = golden["id"]
        if gid in golden_ids:
            errors.append(f"duplicate golden id {gid}")
        golden_ids.add(gid)
        route = golden["route"]
        state = golden["state"]
        if state not in route_states.get(route, set()):
            errors.append(f"golden {gid} has unknown route/state")
        used_routes.add(route)
        used_dimensions.add(golden["dimension"])
        used_capabilities.add(golden["capability"])
        path = root / golden["file"]
        if not path.is_file():
            errors.append(f"golden {gid} file missing")
            continue
        data = path.read_text(encoding="utf-8")
        if not data or len(data.encode("utf-8")) > 32768:
            errors.append(f"golden {gid} must be nonempty and bounded")
        if any(ord(char) < 32 and char != "\n" for char in data):
            errors.append(f"golden {gid} contains terminal control bytes")
        folded = data.casefold()
        if any(secret in folded for secret in forbidden):
            errors.append(f"golden {gid} contains forbidden private material")
        for marker in golden.get("markers", []):
            if marker not in data:
                errors.append(f"golden {gid} misses semantic marker {marker!r}")
        columns, lines = dimensions.get(golden["dimension"], (0, 0))
        rows = data.rstrip("\n").split("\n")
        if len(rows) > lines or any(len(row) > columns for row in rows):
            errors.append(f"golden {gid} exceeds declared text grid")
        if golden["capability"] == "ascii_plain" and not data.isascii():
            errors.append(f"golden {gid} is not ASCII")

    if used_dimensions != set(dimensions):
        errors.append("goldens do not exercise every declared dimension")
    if used_capabilities != set(capabilities):
        errors.append("goldens do not exercise every terminal capability")
    if used_routes != model_routes - {"recent_runs"}:
        errors.append("goldens must cover every renderer route; recent_runs is covered by reports state")
    if not model_elements:
        errors.append("formal model has no visual elements")
    return errors


def main():
    errors = validate()
    if errors:
        print("UI visual parity failed:")
        print("\n".join(f"- {error}" for error in errors))
        return 1
    print("UI visual parity valid: routes, states, dimensions, capabilities, ownership, help, and goldens")
    return 0


if __name__ == "__main__":
    sys.exit(main())
