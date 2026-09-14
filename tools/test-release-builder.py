#!/usr/bin/env python3
# Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Focused deterministic tests for the AR-1027 release-candidate builder."""

import hashlib
import importlib.util
import tempfile
import unittest
from unittest.mock import patch
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("build_release", ROOT / "tools/build-release.py")
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class ReleaseBuilderTests(unittest.TestCase):
    def test_source_archive_is_byte_for_byte_reproducible(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            first = Path(directory) / "one.tar.gz"
            second = Path(directory) / "two.tar.gz"
            MODULE.source_archive(first, 1_700_000_000)
            MODULE.source_archive(second, 1_700_000_000)
            self.assertEqual(first.read_bytes(), second.read_bytes())
            self.assertEqual(hashlib.sha256(first.read_bytes()).digest(),
                             hashlib.sha256(second.read_bytes()).digest())

    def test_identity_rejects_non_lowercase_or_wrong_length(self) -> None:
        self.assertEqual(MODULE.identity("a" * 40, "commit"), "a" * 40)
        with self.assertRaises(ValueError):
            MODULE.identity("A" * 40, "commit")
        with self.assertRaises(ValueError):
            MODULE.identity("a" * 39, "commit")

    def test_architecture_maps_to_target_specific_output_directory(self) -> None:
        self.assertEqual(MODULE.target_triple("x86_64"), "x86_64-unknown-linux-gnu")
        self.assertEqual(MODULE.target_triple("aarch64"), "aarch64-unknown-linux-gnu")
        with self.assertRaises(ValueError):
            MODULE.target_triple("riscv64")

    def test_license_document_matches_strict_runtime_shape(self) -> None:
        metadata = {"packages": [{"name": "demo", "version": "1.0.0", "license": "MIT"}]}
        with patch.object(MODULE, "run", return_value=__import__("json").dumps(metadata)):
            document = MODULE.license_report("v0.1.0")
        self.assertEqual(set(document), {"schema_version", "release", "packages"})
        self.assertEqual(document["schema_version"], 1)
        self.assertEqual(document["release"], "v0.1.0")

    def test_provenance_document_matches_strict_runtime_shape(self) -> None:
        document = MODULE.provenance_report("v0.1.0", "a" * 40, "b" * 40)
        self.assertEqual(set(document), {"schema_version", "release", "source_commit",
                                         "source_tree", "builder", "reproducible"})
        self.assertEqual(document["builder"], "github-actions")
        self.assertTrue(document["reproducible"])


if __name__ == "__main__":
    unittest.main()
