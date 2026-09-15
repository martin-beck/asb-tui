#!/usr/bin/env python3
# Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Unit tests for the AR-1037 help-catalog gate."""

import importlib.util
import json
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("validate_ui_help", ROOT / "tools" / "validate-ui-help.py")
assert SPEC and SPEC.loader
VALIDATOR = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(VALIDATOR)


class UiHelpValidationTests(unittest.TestCase):
    def setUp(self) -> None:
        self.document = json.loads((ROOT / "docs" / "ui-help.json").read_text(encoding="utf-8"))

    def test_repository_catalog_is_valid_and_covers_registered_actions(self) -> None:
        self.assertEqual(VALIDATOR.validate_document(self.document, VALIDATOR._action_ids()), [])

    def test_missing_action_is_reported_deterministically(self) -> None:
        self.document["elements"] = [entry for entry in self.document["elements"] if entry["id"] != "action.quit"]
        errors = VALIDATOR.validate_document(
            self.document, VALIDATOR._action_ids()
        )
        self.assertEqual(errors, ["action.quit: registered action has no help entry"])

    def test_duplicate_and_placeholder_text_are_rejected(self) -> None:
        entry = next(item for item in self.document["elements"] if item["id"] == "screen.landing")
        duplicate = dict(entry)
        self.document["elements"].append(duplicate)
        entry["help"]["summary"] = "TODO"
        errors = VALIDATOR.validate_document(self.document, set())
        self.assertIn("screen.landing: duplicate id", errors)
        self.assertIn("screen.landing: help.summary must be 24-240 characters without padding", errors)
        self.assertIn("screen.landing: help.summary must contain at least four words", errors)
        self.assertIn("screen.landing: help.summary contains placeholder language", errors)

    def test_unknown_fields_and_empty_help_are_rejected(self) -> None:
        entry = next(item for item in self.document["elements"] if item["id"] == "screen.landing")
        entry["help"]["extra"] = "not permitted"
        errors = VALIDATOR.validate_document(self.document, set())
        self.assertEqual(errors, ["screen.landing: help must contain exactly summary and usage"])

    def test_unknown_action_and_top_level_fields_fail_closed(self) -> None:
        self.document["elements"].append(
            {
                "id": "action.not_registered",
                "route": "global",
                "kind": "action",
                "label": "Unknown action",
                "help": {
                    "summary": "Describe an action that the registry does not expose.",
                    "usage": "This entry must be removed or registered before publication.",
                },
            }
        )
        self.document["unexpected"] = True
        errors = VALIDATOR.validate_document(self.document, VALIDATOR._action_ids())
        self.assertIn("action.not_registered: help entry does not match a registered action", errors)
        self.assertIn("catalog: unknown top-level fields: unexpected", errors)

    def test_private_material_is_rejected_even_when_prose_is_long_enough(self) -> None:
        entry = next(item for item in self.document["elements"] if item["id"] == "action.cancel_run")
        entry["help"]["usage"] = (
            "Use the documented workflow; never copy /home/martin/private/token "
            "or an access_token: secret-value into public help."
        )
        errors = VALIDATOR.validate_document(self.document, set())
        self.assertIn(
            "action.cancel_run: help.usage contains private or secret material", errors
        )

    def test_formal_model_elements_have_authoritative_catalog_entries(self) -> None:
        model = json.loads((ROOT / "docs" / "ui-state-model.json").read_text(encoding="utf-8"))
        documented = {entry["id"] for entry in self.document["elements"]}
        model_ids = {entry["help_id"] for entry in model["elements"]}
        self.assertEqual(model_ids - documented, set())

    def test_action_registry_discovery_fails_closed(self) -> None:
        original = VALIDATOR.ACTION_SOURCE
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "actions.rs"
            path.write_text("pub enum UiAction {}", encoding="utf-8")
            VALIDATOR.ACTION_SOURCE = path
            with self.assertRaisesRegex(ValueError, "cannot locate UiAction::ALL registry"):
                VALIDATOR._action_ids()
        VALIDATOR.ACTION_SOURCE = original


if __name__ == "__main__":
    unittest.main()
