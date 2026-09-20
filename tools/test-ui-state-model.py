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
        cls.inventory = json.loads(MODULE.INVENTORY.read_text(encoding="utf-8"))

    def inventory_paths(self, inventory=None):
        inventory = self.inventory if inventory is None else inventory
        return [entry["path"] for entry in inventory["ui_modules"] + inventory["exemptions"]]

    def actual_source_paths(self):
        root = Path(MODULE.ROOT)
        return sorted(path.relative_to(root).as_posix() for path in (root / "src").glob("*.rs"))

    def test_current_model_is_valid(self):
        self.assertEqual(MODULE.validate(self.model), [])

    def test_startup_controller_state_is_formally_declared(self):
        self.assertIn("startup_wizard_opened", self.model["state_fields"])

    def test_readiness_routes_are_closed_before_wizard_selection(self):
        route_ids = {route["id"] for route in self.model["routes"]}
        self.assertIn("landing", route_ids)
        self.assertIn("wizard", route_ids)
        self.assertIn("open_wizard", {item["event"] for item in self.model["transitions"]})

    def test_resize_layout_state_is_formally_declared(self):
        self.assertIn("responsive_layout", self.model["state_fields"])

    def test_contextual_help_state_is_formally_declared(self):
        self.assertIn("contextual_help", self.model["state_fields"])

    def test_measurement_catalog_projection_is_formally_declared(self):
        self.assertIn("measurement_catalog", self.model["state_fields"])

    def test_configuration_persistence_state_is_formally_declared(self):
        self.assertIn("configuration_persistence", self.model["state_fields"])

    def test_configuration_save_binding_and_transition_are_formally_declared(self):
        bindings = {(item["action"], item["element"], item["key"]) for item in self.model["bindings"]}
        self.assertIn(("focus_configuration", "configuration.entry", "Enter"), bindings)
        self.assertIn(("save_configuration", "configuration.entry", "Ctrl-S"), bindings)
        transitions = {(item["event"], item["from"], item["to"]) for item in self.model["transitions"]}
        self.assertIn(("save_configuration", "configuration", "configuration"), transitions)
        save = next(item for item in self.model["transitions"] if item["event"] == "save_configuration")
        self.assertEqual(save["effects"], ["configuration_persisted", "focus_reset"])
        entry_help = next(item["text"] for item in self.model["help"] if item["id"] == "configuration.entry")
        self.assertIn("local configuration store", entry_help)

    def test_wizard_catalog_state_is_formally_declared(self):
        self.assertIn("wizard_catalog", self.model["state_fields"])

    def test_live_setup_catalog_projection_is_formally_declared(self):
        self.assertIn("live_setup_catalog", self.model["state_fields"])
        self.assertIn("wizard_catalog", self.model["state_fields"])

    def test_wizard_completion_state_is_formally_declared(self):
        self.assertIn("wizard_completion", self.model["state_fields"])
        transitions = {(item["event"], item["from"], item["to"]) for item in self.model["transitions"]}
        self.assertIn(("complete_wizard", "wizard", "landing"), transitions)

    def test_wizard_editing_help_and_completion_bindings_are_formally_declared(self):
        bindings = {(item["action"], item["element"], item["key"]) for item in self.model["bindings"]}
        self.assertIn(("wizard_edit", "wizard.agent", "Character"), bindings)
        self.assertIn(("wizard_backspace", "wizard.agent", "Backspace"), bindings)
        self.assertIn(("wizard_help", "wizard.agent", "?"), bindings)
        self.assertIn(("complete_wizard", "wizard.review", "Enter"), bindings)

    def test_workspace_wizard_route_and_bindings_are_formally_declared(self):
        routes = {route["id"]: route for route in self.model["routes"]}
        self.assertIn("wizard", routes)
        self.assertTrue(
            {"wizard.agent", "wizard.provider", "wizard.review"}.issubset(
                routes["wizard"]["elements"]
            )
        )
        bindings = {(item["action"], item["key"]) for item in self.model["bindings"]}
        self.assertTrue(
            {
                ("open_wizard", "w"),
                ("wizard_next", "Enter"),
                ("complete_wizard", "Enter"),
                ("wizard_back", "Esc"),
                ("cancel_wizard", "q"),
            }.issubset(bindings)
        )
        transitions = {
            (item["event"], item["from"], item["to"])
            for item in self.model["transitions"]
        }
        self.assertTrue(
            {
                ("open_wizard", "landing", "wizard"),
                ("wizard_next", "wizard", "wizard"),
                ("wizard_back", "wizard", "wizard"),
                ("cancel_wizard", "wizard", "landing"),
                ("complete_wizard", "wizard", "landing"),
            }.issubset(transitions)
        )

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

    def test_module_inventory_matches_every_source_and_owner(self):
        self.assertEqual(MODULE.validate_module_inventory(self.inventory, self.actual_source_paths()), [])

    def test_module_inventory_rejects_drift_and_weak_exemptions(self):
        actual = self.actual_source_paths()
        missing = actual.copy()
        missing.remove("src/wizard.rs")
        errors = MODULE.validate_module_inventory(self.inventory, missing)
        self.assertTrue(any("stale classification" in error for error in errors))
        errors = MODULE.validate_module_inventory(self.inventory, actual + ["src/new_screen.rs"])
        self.assertIn("module inventory: unclassified source module src/new_screen.rs", errors)
        weak = copy.deepcopy(self.inventory)
        weak["exemptions"][0]["reason"] = "no"
        errors = MODULE.validate_module_inventory(weak, self.inventory_paths(weak))
        self.assertTrue(any("meaningful reason" in error for error in errors))

    def test_module_inventory_must_match_ui_owners(self):
        owners = set(MODULE.UI_OWNERS)
        owners.remove("src/wizard.rs")
        errors = MODULE.validate_module_inventory(self.inventory, self.inventory_paths(), owners)
        self.assertIn("module inventory: UI modules and UI_OWNERS differ", errors)


if __name__ == "__main__":
    unittest.main()
