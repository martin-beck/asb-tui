#!/usr/bin/env python3
# Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Check that the authored UI model covers the executable source registries."""

import argparse
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def snake(value):
    return re.sub(r"(?<!^)([A-Z])", r"_\1", value).lower()


def enum_variants(source, enum_name):
    match = re.search(rf"pub enum {enum_name}\s*\{{(.*?)\n\}}", source, re.S)
    if not match:
        raise ValueError(f"missing enum {enum_name}")
    return {
        line.strip().split("(", 1)[0].split("{", 1)[0].rstrip(",")
        for line in match.group(1).splitlines()
        if line.strip() and not line.lstrip().startswith("#")
    }


def action_ids(source):
    variants = enum_variants(source, "UiAction")
    match = re.search(r"pub const fn id\(self\).*?\n    \}", source, re.S)
    if not match:
        raise ValueError("missing UiAction::id")
    ids = dict(re.findall(r"Self::(\w+)\s*=>\s*\"([^\"]+)\"", match.group(0)))
    if set(ids) != variants:
        raise ValueError("UiAction variants and IDs differ")
    return set(ids.values())


def source_ids(root):
    sources = {path: path.read_text(encoding="utf-8") for path in (root / "src").glob("*.rs")}
    all_source = "\n".join(sources.values())
    routes = {snake(item) for item in enum_variants(sources[root / "src/shell.rs"], "Route")}
    routes |= {snake(item) for item in enum_variants(sources[root / "src/wizard.rs"], "StartupRoute")}
    actions = action_ids(sources[root / "src/actions.rs"])
    rendered_match = re.search(
        r'RENDERED_ELEMENT_BINDINGS.*?=\s*&\[(.*?)\]',
        sources[root / "src/formal_state.rs"],
        re.S,
    )
    if not rendered_match:
        raise ValueError("missing rendered element bindings")
    rendered = set(rendered_match.group(1).replace('"', "").split(","))
    return routes, actions, all_source, {item.strip() for item in rendered if item.strip()}


def check(root=ROOT):
    model = json.loads((root / "docs/ui-state-model.json").read_text(encoding="utf-8"))
    routes, actions, source, rendered = source_ids(root)
    errors = []
    model_routes = {item["id"] for item in model["routes"]}
    if routes != model_routes:
        errors.append(f"route parity mismatch: source-only={sorted(routes - model_routes)} model-only={sorted(model_routes - routes)}")
    model_actions = {item["action"] for item in model["bindings"]}
    if not actions <= model_actions:
        errors.append(f"action parity mismatch: source-only={sorted(actions - model_actions)}")
    element_ids = {item["id"] for item in model["elements"]}
    source_elements = {item for item in element_ids if item in source} | rendered
    if source_elements != element_ids:
        errors.append(f"element parity mismatch: model-only={sorted(element_ids - source_elements)}")
    transition_ids = {item["event"] for item in model["transitions"]}
    missing_events = sorted(item for item in transition_ids if item not in source)
    if missing_events:
        errors.append(f"transition parity mismatch: model-only={missing_events}")
    return errors


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, default=ROOT)
    args = parser.parse_args()
    errors = check(args.root)
    if errors:
        print("UI source parity failed:")
        print("\n".join(f"- {error}" for error in errors))
        return 1
    print("UI source parity valid")
    return 0


if __name__ == "__main__":
    sys.exit(main())
