#!/usr/bin/env python3
# Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Validate the fail-closed public release-channel declaration."""

import json
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
STATUS = ROOT / "release" / "channel-status.json"
REQUIRED = {
    "asb_external_protocol",
    "asb_lifecycle_router",
    "interactive_ui",
    "x86_64_release_qualification",
    "aarch64_release_qualification",
    "signed_release_bundle",
    "hosted_exact_release_ci",
    "trusted_exact_release_ci",
}


def main() -> None:
    value = json.loads(STATUS.read_text(encoding="utf-8"))
    assert value["schema_version"] == 1
    assert value["trust_policy_version"] == 1
    requirements = value["promotion_requirements"]
    assert set(requirements) == REQUIRED
    assert all(isinstance(item, bool) for item in requirements.values())
    fully_verified = all(requirements.values())
    if value["classification"] == "verified_extension":
        assert fully_verified
        assert value["installable"] is True
        assert value["release_artifacts_published"] is True
    else:
        assert value["classification"] == "source_only_unverified"
        assert value["installable"] is False
        assert value["release_artifacts_published"] is False
        assert not fully_verified

    forbidden_dynamic_identity = {
        "source_commit",
        "source_tree",
        "executable_sha256",
        "manifest_sha256",
        "verified_bundles",
    }
    assert forbidden_dynamic_identity.isdisjoint(value)

    release_docs = (ROOT / "docs" / "RELEASE_CHANNELS.md").read_text(encoding="utf-8")
    matrix = (ROOT / "docs" / "CAPABILITY_MATRIX.md").read_text(encoding="utf-8")
    for phrase in (
        "signed annotated version tag",
        "Rollback and cleanup",
        "fail closed",
        "ASB_TUI_SOURCE_COMMIT",
        "ASB_TUI_SOURCE_TREE",
        "real release-compiled executable",
    ):
        assert phrase in release_docs
    for phrase in ("Top-level `asb tui` routing", "Ratatui/Crossterm", "No supported public workflow"):
        assert phrase in matrix


if __name__ == "__main__":
    main()
