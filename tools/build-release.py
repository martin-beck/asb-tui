#!/usr/bin/env python3
# Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Build deterministic, unsigned release-candidate artifacts for AR-1027.

This tool deliberately does not modify ``release/channel-status.json`` and does
not publish anything.  Promotion remains a separate, reviewed operation.
"""

from __future__ import annotations

import argparse
import gzip
import hashlib
import json
import os
import shutil
import subprocess
import tarfile
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
ARTIFACTS = (("asb-tui", "executable"), ("source", "source"),
             ("licenses", "license_report"), ("sbom", "sbom"),
             ("provenance", "provenance"))


def run(command: list[str], *, cwd: Path = ROOT, env: dict[str, str] | None = None) -> str:
    result = subprocess.run(command, cwd=cwd, env=env, check=True, text=True,
                            stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    return result.stdout.strip()


def git(*args: str) -> str:
    return run(["git", *args])


def digest(path: Path) -> str:
    hasher = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            hasher.update(chunk)
    return hasher.hexdigest()


def identity(value: str, name: str, length: int = 40) -> str:
    if len(value) != length or any(char not in "0123456789abcdef" for char in value):
        raise ValueError(f"{name} must be lowercase hexadecimal ({length} characters)")
    return value


def write_json(path: Path, value: object) -> None:
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def source_archive(output: Path, source_date_epoch: int) -> None:
    # GNU tar/gzip are avoided: tarfile gives stable member ordering, metadata,
    # and gzip timestamp independent of the checkout filesystem.
    with output.open("wb") as stream:
        with gzip.GzipFile(filename="", fileobj=stream, mode="wb", mtime=source_date_epoch,
                           compresslevel=9) as compressed:
            with tarfile.open(fileobj=compressed, mode="w", format=tarfile.PAX_FORMAT) as archive:
                paths = sorted(ROOT.rglob("*"))
                for path in paths:
                    relative = path.relative_to(ROOT)
                    if (".git" in relative.parts or "target" in relative.parts
                            or "__pycache__" in relative.parts or ".pytest_cache" in relative.parts):
                        continue
                    info = archive.gettarinfo(str(path), arcname=Path("asb-tui") / relative)
                    info.uid = info.gid = 0
                    info.uname = info.gname = ""
                    info.mtime = source_date_epoch
                    if info.isreg():
                        with path.open("rb") as source:
                            archive.addfile(info, source)
                    else:
                        archive.addfile(info)


def license_report() -> dict[str, object]:
    lock = run(["cargo", "metadata", "--locked", "--offline", "--format-version", "1"])
    metadata = json.loads(lock)
    packages = [{"name": item["name"], "version": item["version"],
                 "license": item.get("license") or "NOASSERTION"}
                for item in metadata["packages"]]
    return {"schema_version": 1, "generator": "asb-tui/tools/build-release.py",
            "packages": sorted(packages, key=lambda item: (item["name"], item["version"]))}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--release", required=True)
    parser.add_argument("--asb-version", required=True)
    parser.add_argument("--protocol-version", type=int, required=True)
    parser.add_argument("--coordinator-version", required=True)
    parser.add_argument("--coordinator-commit", required=True)
    parser.add_argument("--quality-version", required=True)
    parser.add_argument("--quality-commit", required=True)
    parser.add_argument("--coordinator-tree", required=True)
    parser.add_argument("--quality-tree", required=True)
    parser.add_argument("--coordinator-artifact-sha256", required=True)
    parser.add_argument("--quality-artifact-sha256", required=True)
    parser.add_argument("--issued-unix", type=int, required=True)
    parser.add_argument("--expires-unix", type=int, required=True)
    parser.add_argument("--source-commit", default=os.environ.get("ASB_TUI_SOURCE_COMMIT"))
    parser.add_argument("--source-tree", default=os.environ.get("ASB_TUI_SOURCE_TREE"))
    parser.add_argument("--architecture", choices=("x86_64", "aarch64"), default="x86_64")
    parser.add_argument("--signing-key", type=Path)
    args = parser.parse_args()
    if not args.release.startswith("v") or args.release.count(".") != 2:
        raise SystemExit("--release must be a semantic version such as v0.1.0")
    source_commit = identity(args.source_commit or git("rev-parse", "HEAD"), "source commit")
    source_tree = identity(args.source_tree or git("rev-parse", "HEAD^{tree}"), "source tree")
    for name, value in (("coordinator commit", args.coordinator_commit),
                        ("coordinator tree", args.coordinator_tree),
                        ("quality commit", args.quality_commit),
                        ("quality tree", args.quality_tree),
                        ("coordinator artifact digest", args.coordinator_artifact_sha256),
                        ("quality artifact digest", args.quality_artifact_sha256)):
        identity(value, name, 64 if "digest" in name else 40)
    if args.issued_unix < 1 or args.expires_unix <= args.issued_unix:
        raise SystemExit("release validity interval is invalid")
    output = args.output_dir.resolve()
    output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="asb-tui-release-") as temporary:
        staging = Path(temporary)
        executable = staging / "asb-tui"
        env = os.environ.copy()
        env.update(ASB_TUI_SOURCE_COMMIT=source_commit, ASB_TUI_SOURCE_TREE=source_tree)
        target_dir = staging / "target"
        env["CARGO_TARGET_DIR"] = str(target_dir)
        run(["cargo", "build", "--locked", "--offline", "--release"], env=env)
        built = target_dir / "release" / "asb-tui"
        if not built.is_file():
            raise SystemExit("cargo did not produce the asb-tui executable")
        shutil.copy2(built, executable)
        executable.chmod(0o755)
        source_archive(staging / "source.tar.gz", args.issued_unix)
        write_json(staging / "licenses.json", license_report())
        sbom_path = ROOT / "provenance/sbom.spdx.json"
        original_sbom = sbom_path.read_bytes()
        try:
            run(["python3", "tools/generate-rust-sbom.py"], env=env)
            shutil.copy2(sbom_path, staging / "sbom.spdx.json")
        finally:
            sbom_path.write_bytes(original_sbom)
        bundle = "asb-tui-v1-linux-" + args.architecture
        compatibility = {"bundle": bundle, "architecture": args.architecture,
                         "asb_version": args.asb_version, "protocol_version": args.protocol_version,
                         "coordinator_version": args.coordinator_version,
                         "coordinator_commit": args.coordinator_commit,
                         "quality_version": args.quality_version, "quality_commit": args.quality_commit}
        components = [
            {"name": "asb-tui", "version": args.release, "commit": source_commit,
             "tree": source_tree, "artifact_sha256": digest(executable)},
            {"name": "agent-workflow-coordinator", "version": args.coordinator_version,
             "commit": args.coordinator_commit, "tree": args.coordinator_tree,
             "artifact_sha256": args.coordinator_artifact_sha256},
            {"name": "agent-workflow-quality", "version": args.quality_version,
             "commit": args.quality_commit, "tree": args.quality_tree,
             "artifact_sha256": args.quality_artifact_sha256},
        ]
        artifact_files = {"asb-tui": executable, "source": staging / "source.tar.gz",
                          "licenses": staging / "licenses.json", "sbom": staging / "sbom.spdx.json"}
        provenance = {"schema_version": 1, "release": args.release, "source_commit": source_commit,
                      "source_tree": source_tree, "workflow": {"name": "local-release-candidate"},
                      "artifacts": []}
        for name, _kind in ARTIFACTS[:-1]:
            path = artifact_files[name]
            provenance["artifacts"].append({"name": name, "size": path.stat().st_size,
                                            "sha256": digest(path)})
        write_json(staging / "provenance.json", provenance)
        artifact_files["provenance"] = staging / "provenance.json"
        manifest_artifacts = []
        for name, kind in ARTIFACTS:
            path = artifact_files[name]
            manifest_artifacts.append({"name": name, "kind": kind,
                "url": f"https://github.com/martin-beck/asb-tui/releases/download/{args.release}/{path.name}",
                "size": path.stat().st_size, "sha256": digest(path)})
        manifest = {"schema_version": 1, "release": args.release,
                    "source_commit": source_commit, "source_tree": source_tree,
                    "issued_unix": args.issued_unix, "expires_unix": args.expires_unix,
                    "compatibility": compatibility, "components": components,
                    "artifacts": manifest_artifacts}
        write_json(staging / "manifest.json", manifest)
        for name, _kind in ARTIFACTS:
            shutil.copy2(artifact_files[name], output / artifact_files[name].name)
        shutil.copy2(staging / "manifest.json", output / "manifest.json")
        if args.signing_key:
            signature = run(["ssh-keygen", "-Y", "sign", "-n", "asb-tui-bundle-v1",
                             "-f", str(args.signing_key), str(staging / "manifest.json")])
            del signature
            shutil.copy2(staging / "manifest.json.sig", output / "manifest.json.sig")
    print(output)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
