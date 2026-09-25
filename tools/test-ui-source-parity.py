#!/usr/bin/env python3
"""Focused tests for the source/model parity gate."""

import importlib.util
import json
import tempfile
import textwrap
import unittest
from pathlib import Path
from shutil import copytree, copyfile

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("ui_source_parity", ROOT / "tools/check-ui-source-parity.py")
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader
SPEC.loader.exec_module(MODULE)


class SourceParityTests(unittest.TestCase):
    def test_current_source_and_model_match(self):
        self.assertEqual(MODULE.check(ROOT), [])

    def test_action_id_extraction_is_closed(self):
        source = textwrap.dedent("""
        pub enum UiAction {
            OpenHelp,
            Quit,
        }
        impl UiAction { pub const fn id(self) -> &'static str {
            match self {
                Self::OpenHelp => \"open_help\",
                Self::Quit => \"quit\",
            }
        }
        }
        """)
        self.assertEqual(MODULE.action_ids(source), {"open_help", "quit"})

    def test_model_only_route_fails(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            copytree(ROOT / "src", root / "src")
            (root / "docs").mkdir()
            copyfile(ROOT / "docs/ui-state-model.json", root / "docs/ui-state-model.json")
            model_path = root / "docs/ui-state-model.json"
            model = json.loads(model_path.read_text(encoding="utf-8"))
            model["routes"].append({"id": "unimplemented", "elements": ["navigation"]})
            model_path.write_text(json.dumps(model), encoding="utf-8")
            self.assertTrue(any("route parity mismatch" in error for error in MODULE.check(root)))


if __name__ == "__main__":
    unittest.main()
