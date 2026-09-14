#!/usr/bin/env python3
# Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
import copy
import importlib.util
import json
import unittest

SPEC = importlib.util.spec_from_file_location("validator", "tools/validate-ui-state-model.py")
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


if __name__ == "__main__":
    unittest.main()
