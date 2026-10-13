#!/usr/bin/env python3
# Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Focused failure-path tests for the visual parity gate."""

import importlib.util
import json
import shutil
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location(
    "ui_visual_parity", ROOT / "tools/check-ui-visual-parity.py"
)
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


def fixture_root():
    temporary = tempfile.TemporaryDirectory()
    root = Path(temporary.name)
    (root / "docs").mkdir()
    (root / "src").mkdir()
    (root / "tests/goldens").mkdir(parents=True)
    for name in [
        "ui-visual-regression.json",
        "ui-state-model.json",
        "ui-state-model.generated.json",
        "ui-help.json",
        "ui-module-inventory.json",
    ]:
        shutil.copy2(ROOT / "docs" / name, root / "docs" / name)
    shutil.copy2(ROOT / "src/ui.rs", root / "src/ui.rs")
    shutil.copy2(ROOT / "src/actions.rs", root / "src/actions.rs")
    (root / "tools").mkdir()
    shutil.copy2(
        ROOT / "tools/validate-ui-state-model.py",
        root / "tools/validate-ui-state-model.py",
    )
    shutil.copytree(
        ROOT / "tests/goldens/visual", root / "tests/goldens/visual"
    )
    return temporary, root


class VisualParityTests(unittest.TestCase):
    def test_repository_fixture_is_valid(self):
        self.assertEqual(MODULE.validate(ROOT), [])

    def test_missing_route_and_dimension_metadata_fail_closed(self):
        temporary, root = fixture_root()
        self.addCleanup(temporary.cleanup)
        path = root / "docs/ui-visual-regression.json"
        document = json.loads(path.read_text())
        document["routes"].pop()
        document["dimensions"][0]["columns"] = 139
        path.write_text(json.dumps(document))
        errors = MODULE.validate(root)
        self.assertTrue(any("route coverage mismatch" in error for error in errors))
        self.assertTrue(any("dimension matrix mismatch" in error for error in errors))

    def test_absent_golden_and_unknown_state_fail_closed(self):
        temporary, root = fixture_root()
        self.addCleanup(temporary.cleanup)
        path = root / "docs/ui-visual-regression.json"
        document = json.loads(path.read_text())
        document["goldens"][0]["state"] = "invented"
        missing = root / document["goldens"][1]["file"]
        missing.unlink()
        path.write_text(json.dumps(document))
        errors = MODULE.validate(root)
        self.assertTrue(any("unknown route/state" in error for error in errors))
        self.assertTrue(any("file missing" in error for error in errors))

    def test_private_and_control_material_fail_closed(self):
        temporary, root = fixture_root()
        self.addCleanup(temporary.cleanup)
        manifest = json.loads(
            (root / "docs/ui-visual-regression.json").read_text()
        )
        path = root / manifest["goldens"][0]["file"]
        path.write_text("/home/private\n\x1b[31msecret\n")
        errors = MODULE.validate(root)
        self.assertTrue(any("private material" in error for error in errors))
        self.assertTrue(any("control bytes" in error for error in errors))


if __name__ == "__main__":
    unittest.main()
