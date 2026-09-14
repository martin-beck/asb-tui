#!/usr/bin/env python3
# Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Focused negative tests for the independent release-candidate verifier."""

import importlib.util
import json
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("verify_release", ROOT / "tools/verify-release-candidate.py")
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class ReleaseVerifierTests(unittest.TestCase):
    def test_strict_documents_accept_runtime_shape(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            bundle = Path(directory)
            manifest = {"release": "v0.1.0", "source_commit": "a" * 40, "source_tree": "b" * 40}
            (bundle / "licenses.json").write_text(json.dumps({
                "schema_version": 1, "release": "v0.1.0",
                "packages": [{"name": "demo", "version": "1.0.0", "license": "MIT"}],
            }), encoding="utf-8")
            (bundle / "provenance.json").write_text(json.dumps({
                "schema_version": 1, "release": "v0.1.0", "source_commit": "a" * 40,
                "source_tree": "b" * 40, "builder": "github-actions", "reproducible": True,
            }), encoding="utf-8")
            MODULE.validate_documents(bundle, manifest)

    def test_strict_documents_reject_unknown_provenance_field(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            bundle = Path(directory)
            manifest = {"release": "v0.1.0", "source_commit": "a" * 40, "source_tree": "b" * 40}
            (bundle / "licenses.json").write_text(
                '{"schema_version":1,"release":"v0.1.0","packages":[]}', encoding="utf-8")
            (bundle / "provenance.json").write_text(json.dumps({
                "schema_version": 1, "release": "v0.1.0", "source_commit": "a" * 40,
                "source_tree": "b" * 40, "builder": "github-actions", "reproducible": True,
                "workflow": {"name": "invalid"},
            }), encoding="utf-8")
            with self.assertRaises(SystemExit):
                MODULE.validate_documents(bundle, manifest)
    def test_missing_signed_manifest_fails_closed(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaises(SystemExit):
                MODULE.verify(Path(directory), ROOT / "provenance/allowed_signers", "martin.beck2@gmx.de", "asb-tui-bundle-v1")

    def test_missing_artifact_set_fails_before_signature_use(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            bundle = Path(directory)
            (bundle / "manifest.json").write_text('{"schema_version":1}', encoding="utf-8")
            (bundle / "manifest.json.sig").write_bytes(b"invalid")
            with self.assertRaises(SystemExit):
                MODULE.verify(bundle, ROOT / "provenance/allowed_signers", "martin.beck2@gmx.de", "asb-tui-bundle-v1")

    def test_non_string_artifact_name_is_a_bounded_failure(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            bundle = Path(directory)
            (bundle / "manifest.json").write_text(
                '{"schema_version":1,"artifacts":[{"name":[]},{"name":"source"},{"name":"licenses"},{"name":"sbom"},{"name":"provenance"}],"components":[],"release":"v0.1.0","source_commit":"' + "a" * 40 + '","source_tree":"' + "b" * 40 + '"}',
                encoding="utf-8",
            )
            (bundle / "manifest.json.sig").write_bytes(b"invalid")
            with self.assertRaises(SystemExit):
                MODULE.verify(bundle, ROOT / "provenance/allowed_signers", "martin.beck2@gmx.de", "asb-tui-bundle-v1")


if __name__ == "__main__":
    unittest.main()
