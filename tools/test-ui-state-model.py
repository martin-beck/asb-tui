#!/usr/bin/env python3
# Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
import copy
import importlib.util
import json
from pathlib import Path
import subprocess
import unittest

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location(
    "validator", ROOT / "tools" / "validate-ui-state-model.py"
)
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class UiStateModelTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.model = json.loads(MODULE.MODEL.read_text(encoding="utf-8"))

    def test_current_model_is_valid(self):
        self.assertEqual(MODULE.validate(self.model), [])

    def test_unknown_element_is_rejected(self):
        model = copy.deepcopy(self.model)
        model["routes"][0]["elements"].append("landing.missing")
        self.assertTrue(any("unknown element" in error for error in MODULE.validate(model)))

    def test_unreachable_route_is_rejected(self):
        model = copy.deepcopy(self.model)
        model["routes"].append({"id": "orphan", "elements": ["navigation"]})
        self.assertIn("routes: unreachable route orphan", MODULE.validate(model))

    def test_duplicate_element_is_rejected(self):
        model = copy.deepcopy(self.model)
        model["elements"].append(copy.deepcopy(model["elements"][0]))
        self.assertTrue(any("duplicate id" in error for error in MODULE.validate(model)))

    def test_missing_help_and_focus_metadata_are_rejected(self):
        model = copy.deepcopy(self.model)
        model["elements"][0]["help_id"] = "missing"
        model["elements"][1].pop("focusable")
        errors = MODULE.validate(model)
        self.assertIn("landing.primary: help_id does not reference a help entry", errors)
        self.assertIn("measures.search: focusable must be boolean", errors)

    def test_weak_help_prose_and_escape_path_are_rejected(self):
        model = copy.deepcopy(self.model)
        model["help"][0]["text"] = "..."
        model["transitions"] = [entry for entry in model["transitions"] if entry["from"] != "help"]
        errors = MODULE.validate(model)
        self.assertIn("help.screen.landing: text must be meaningful prose (at least four words)", errors)
        self.assertIn("routes: help has no escape path to landing", errors)

    def test_generated_artifact_matches_authored_model(self):
        generated = json.loads(MODULE.GENERATED.read_text(encoding="utf-8"))
        self.assertEqual(MODULE.canonical(generated), MODULE.canonical(self.model))

    def test_change_ownership_requires_model_and_focused_tests(self):
        self.assertTrue({"src/startup.rs", "src/wizard.rs"}.issubset(MODULE.UI_OWNERS))
        for owner in MODULE.UI_OWNERS:
            errors = MODULE.validate_changes([owner])
            self.assertEqual(len(errors), 2, owner)
            self.assertTrue(any("formal model update" in error for error in errors), owner)
            self.assertTrue(any("focused formal-model test" in error for error in errors), owner)
        self.assertEqual(MODULE.validate_changes(["src/ui.rs", "docs/ui-state-model.json", "tests/ui.rs"]), [])

    def test_malformed_route_and_transition_ids_fail_closed(self):
        model = copy.deepcopy(self.model)
        model["routes"].append({"id": {"bad": "id"}, "elements": ["navigation"]})
        model["transitions"].append({"from": {"bad": "id"}, "to": "landing", "event": "bad"})
        errors = MODULE.validate(model)
        self.assertTrue(any("invalid route id" in error for error in errors))
        self.assertTrue(any("route ids must be strings" in error for error in errors))

    def test_malformed_route_elements_fail_closed(self):
        model = copy.deepcopy(self.model)
        model["routes"][0]["elements"] = {"bad": "shape"}
        errors = MODULE.validate(model)
        self.assertTrue(any("elements must be a non-empty array" in error for error in errors))

    def test_binding_and_parent_cycles_are_rejected(self):
        model = copy.deepcopy(self.model)
        model["bindings"].append({"action": "quit", "element": "navigation", "key": "x"})
        model["elements"][0]["parent"] = "navigation"
        next_element = next(item for item in model["elements"] if item["id"] == "navigation")
        next_element["parent"] = "landing.primary"
        errors = MODULE.validate(model)
        self.assertTrue(any("duplicate binding" in error for error in errors))
        self.assertTrue(any("parent relationship contains a cycle" in error for error in errors))


if __name__ == "__main__":
    unittest.main()
