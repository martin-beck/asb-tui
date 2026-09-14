#!/usr/bin/env python3
# Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
import copy
import importlib.util
import json
import subprocess
import unittest
from pathlib import Path

SPEC = importlib.util.spec_from_file_location("validator", "tools/validate-ui-state-model.py")
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class UiStateModelTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.model = json.loads(MODULE.MODEL.read_text(encoding="utf-8"))
        cls.inventory = json.loads(MODULE.INVENTORY.read_text(encoding="utf-8"))

    def inventory_paths(self, inventory=None):
        inventory = self.inventory if inventory is None else inventory
        return [entry["path"] for entry in inventory["ui_modules"] + inventory["exemptions"]]

    def actual_source_paths(self):
        root = Path(MODULE.ROOT)
        return sorted(path.relative_to(root).as_posix() for path in (root / "src").glob("*.rs"))

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
        self.assertTrue({"src/formal_state.rs", "src/startup.rs", "src/wizard.rs"}.issubset(MODULE.UI_OWNERS))
        for owner in MODULE.UI_OWNERS:
            errors = MODULE.validate_changes([owner])
            self.assertEqual(len(errors), 2, owner)
            self.assertTrue(any("formal model update" in error for error in errors), owner)
            self.assertTrue(any("focused formal-model test" in error for error in errors), owner)
        self.assertEqual(MODULE.validate_changes(["src/ui.rs", "docs/ui-state-model.json", "tests/ui.rs"]), [])

    def test_binding_and_parent_cycles_are_rejected(self):
        model = copy.deepcopy(self.model)
        model["bindings"].append({"action": "quit", "element": "navigation", "key": "x"})
        model["elements"][0]["parent"] = "navigation"
        model["elements"][10]["parent"] = "landing.primary"
        errors = MODULE.validate(model)
        self.assertTrue(any("duplicate binding" in error for error in errors))
        self.assertTrue(any("parent relationship contains a cycle" in error for error in errors))

    def test_module_inventory_matches_every_source_and_owner(self):
        self.assertEqual(
            MODULE.validate_module_inventory(self.inventory, self.actual_source_paths()), []
        )

    def test_module_addition_deletion_and_rename_fail_closed(self):
        actual = self.actual_source_paths()
        missing = actual.copy()
        missing.remove("src/wizard.rs")
        errors = MODULE.validate_module_inventory(self.inventory, missing)
        self.assertTrue(any("stale classification for missing module src/wizard.rs" in error for error in errors))
        added = actual + ["src/new_screen.rs"]
        errors = MODULE.validate_module_inventory(self.inventory, added)
        self.assertIn("module inventory: unclassified source module src/new_screen.rs", errors)
        renamed = ["src/renamed_screen.rs" if path == "src/wizard.rs" else path for path in actual]
        errors = MODULE.validate_module_inventory(self.inventory, renamed)
        self.assertTrue(any("unclassified source module src/renamed_screen.rs" in error for error in errors))

    def test_duplicate_and_widened_exemption_mutations_are_rejected(self):
        duplicate = copy.deepcopy(self.inventory)
        duplicate["ui_modules"].append(copy.deepcopy(duplicate["ui_modules"][0]))
        errors = MODULE.validate_module_inventory(duplicate, self.inventory_paths(duplicate))
        self.assertTrue(any("duplicate UI path" in error for error in errors))
        widened = copy.deepcopy(self.inventory)
        widened["exemptions"][0]["path"] = "../outside.rs"
        errors = MODULE.validate_module_inventory(widened, self.inventory_paths(widened))
        self.assertTrue(any("invalid or duplicate exemption path" in error for error in errors))

        weak = copy.deepcopy(self.inventory)
        weak["exemptions"][0]["reason"] = "no"
        errors = MODULE.validate_module_inventory(weak, self.inventory_paths(weak))
        self.assertTrue(any("needs meaningful reason and expiry" in error for error in errors))

        bad_expiry = copy.deepcopy(self.inventory)
        bad_expiry["exemptions"][0]["expires"] = "not-a-date"
        errors = MODULE.validate_module_inventory(bad_expiry, self.inventory_paths(bad_expiry))
        self.assertTrue(any("has invalid expiry" in error for error in errors))

    def test_owner_drift_is_rejected(self):
        owners = set(MODULE.UI_OWNERS)
        owners.remove("src/wizard.rs")
        errors = MODULE.validate_module_inventory(self.inventory, self.inventory_paths(), owners)
        self.assertIn("module inventory: UI modules and UI_OWNERS differ", errors)


if __name__ == "__main__":
    unittest.main()
