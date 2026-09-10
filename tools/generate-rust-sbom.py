#!/usr/bin/env python3
# Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Generate or verify the deterministic SPDX inventory for Cargo.lock."""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import tomllib
from pathlib import Path
from urllib.parse import quote

ROOT = Path(__file__).resolve().parents[1]
OUTPUT = ROOT / "provenance" / "sbom.spdx.json"
MAX_LOCK_BYTES = 4 * 1024 * 1024
MAX_METADATA_BYTES = 16 * 1024 * 1024
MAX_PACKAGES = 512


def spdx_id(name: str, version: str) -> str:
    value = re.sub(r"[^A-Za-z0-9.-]", "-", f"{name}-{version}")
    return f"SPDXRef-{value}"


def cargo_metadata() -> dict:
    command = ["cargo", "metadata", "--locked", "--offline", "--format-version", "1"]
    result = subprocess.run(
        command,
        cwd=ROOT,
        check=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        timeout=60,
    )
    if len(result.stdout.encode("utf-8")) > MAX_METADATA_BYTES:
        raise SystemExit("cargo metadata exceeds the deterministic size bound")
    return json.loads(result.stdout)


def document() -> dict:
    lock_path = ROOT / "Cargo.lock"
    if lock_path.stat().st_size > MAX_LOCK_BYTES:
        raise SystemExit("Cargo.lock exceeds the deterministic size bound")
    lock = tomllib.loads(lock_path.read_text(encoding="utf-8"))
    if not 1 <= len(lock["package"]) <= MAX_PACKAGES:
        raise SystemExit("Cargo.lock package count is outside the reviewed bound")
    metadata = cargo_metadata()
    metadata_packages = {
        (package["name"], package["version"]): package
        for package in metadata["packages"]
    }
    packages = []
    for package in sorted(lock["package"], key=lambda item: (item["name"], item["version"])):
        name = package["name"]
        version = package["version"]
        metadata_package = metadata_packages[(name, version)]
        entry = {
            "SPDXID": "SPDXRef-asb-tui" if name == "asb-tui" else spdx_id(name, version),
            "copyrightText": (
                "Copyright (c) 2026 Huawei Technologies Co., Ltd."
                if name == "asb-tui"
                else "NOASSERTION"
            ),
            "downloadLocation": "NOASSERTION",
            "filesAnalyzed": False,
            "licenseConcluded": "NOASSERTION" if name != "asb-tui" else "MIT",
            "licenseDeclared": metadata_package.get("license") or "NOASSERTION",
            "name": name,
            "versionInfo": version,
        }
        checksum = package.get("checksum")
        if checksum:
            entry["checksums"] = [{"algorithm": "SHA256", "checksumValue": checksum}]
            entry["downloadLocation"] = (
                "https://crates.io/api/v1/crates/"
                f"{quote(name)}/{quote(version)}/download"
            )
        elif name != "asb-tui":
            raise SystemExit(f"non-registry package is forbidden: {name} {version}")
        packages.append(entry)

    root_package = next(
        package for package in metadata["packages"] if package["name"] == "asb-tui"
    )
    root_node = next(node for node in metadata["resolve"]["nodes"] if node["id"] == root_package["id"])
    id_by_package_id = {
        package["id"]: (
            "SPDXRef-asb-tui"
            if package["name"] == "asb-tui"
            else spdx_id(package["name"], package["version"])
        )
        for package in metadata["packages"]
    }
    relationships = [
        {
            "relatedSpdxElement": "SPDXRef-asb-tui",
            "relationshipType": "DESCRIBES",
            "spdxElementId": "SPDXRef-DOCUMENT",
        }
    ]
    relationships.extend(
        {
            "relatedSpdxElement": id_by_package_id[dependency["pkg"]],
            "relationshipType": "DEPENDS_ON",
            "spdxElementId": "SPDXRef-asb-tui",
        }
        for dependency in sorted(root_node["deps"], key=lambda item: item["pkg"])
    )
    return {
        "SPDXID": "SPDXRef-DOCUMENT",
        "creationInfo": {
            "created": "2026-09-10T00:00:00Z",
            "creators": ["Organization: Huawei Technologies Co., Ltd."],
            "licenseListVersion": "3.27.0",
        },
        "dataLicense": "CC0-1.0",
        "documentNamespace": "https://github.com/martin-beck/asb-tui/sbom/0.1.0",
        "name": "asb-tui-0.1.0",
        "packages": packages,
        "relationships": relationships,
        "spdxVersion": "SPDX-2.3",
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true")
    arguments = parser.parse_args()
    rendered = json.dumps(document(), indent=2, sort_keys=True) + "\n"
    if arguments.check:
        if OUTPUT.read_text(encoding="utf-8") != rendered:
            raise SystemExit("provenance/sbom.spdx.json is stale")
        return 0
    OUTPUT.write_text(rendered, encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
