#!/usr/bin/env python3
# Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
import copy
import importlib.util
import json
from pathlib import Path
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

    def test_malformed_route_id_is_rejected_without_crashing(self):
        model = copy.deepcopy(self.model)
        model["routes"].append({"id": {"not": "hashable"}, "elements": []})
        errors = MODULE.validate(model)
        self.assertTrue(any("invalid route id" in error for error in errors))

    def test_malformed_transition_ids_are_rejected_without_crashing(self):
        model = copy.deepcopy(self.model)
        model["transitions"].append({"from": {"bad": "id"}, "to": "landing"})
        errors = MODULE.validate(model)
        self.assertTrue(any("route ids must be strings" in error for error in errors))

    def test_non_array_route_elements_are_rejected(self):
        model = copy.deepcopy(self.model)
        model["routes"][0]["elements"] = {"not": "an array"}
        errors = MODULE.validate(model)
        self.assertTrue(any("elements must be an array" in error for error in errors))


if __name__ == "__main__":
    unittest.main()
