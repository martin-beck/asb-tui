#!/usr/bin/env python3
# Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Build an immutable, development-only asb-tui bundle.

The development channel intentionally has no production signature.  It is
still immutable: the requested commit and tree are checked before compilation,
the executable is addressed by its SHA-256, and the manifest is written beside
the bytes.  ASB's development installer can consume the resulting directory
without trusting a mutable path or a pre-existing checkout.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import stat
import subprocess
import tempfile
import time
import uuid
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
REPOSITORY = "https://github.com/martin-beck/asb-tui.git"
TARGETS = {"x86_64": "x86_64-unknown-linux-gnu", "aarch64": "aarch64-unknown-linux-gnu"}
MAX_EXECUTABLE_BYTES = 256 * 1024 * 1024


def run(*args: str, cwd: Path = ROOT, env: dict[str, str] | None = None) -> str:
    return subprocess.run([*args], cwd=cwd, env=env, check=True, text=True,
                          stdout=subprocess.PIPE, stderr=subprocess.PIPE).stdout.strip()


def valid_hex(value: str, length: int) -> bool:
    return len(value) == length and all(char in "0123456789abcdef" for char in value)


def digest(path: Path) -> str:
    hasher = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            hasher.update(chunk)
    return hasher.hexdigest()


def regular(path: Path) -> bool:
    try:
        return stat.S_ISREG(path.stat(follow_symlinks=False).st_mode)
    except FileNotFoundError:
        return False


def atomic_bytes(path: Path, data: bytes, mode: int) -> None:
    temporary = path.with_name(f".{path.name}.stage-{uuid.uuid4().hex}")
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL
    descriptor = os.open(temporary, flags, mode)
    try:
        with os.fdopen(descriptor, "wb") as stream:
            descriptor = -1
            stream.write(data)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
        directory = os.open(path.parent, os.O_RDONLY | getattr(os, "O_DIRECTORY", 0))
        try:
            os.fsync(directory)
        finally:
            os.close(directory)
    finally:
        if descriptor >= 0:
            os.close(descriptor)
        temporary.unlink(missing_ok=True)


def publish(output: Path, executable: Path, manifest: dict[str, object], data: bytes) -> Path:
    """Publish a version once, then atomically switch the active selector."""
    versions = output / "dev-versions"
    versions.mkdir(mode=0o700, parents=True, exist_ok=True)
    executable_digest = str(manifest["executable_sha256"])
    version = versions / executable_digest
    if version.exists() or version.is_symlink():
        if version.is_symlink() or not version.is_dir():
            raise SystemExit("content-addressed version is not a directory")
        if not regular(version / "asb-tui") or not regular(version / "manifest.json"):
            raise SystemExit("existing content-addressed version is incomplete")
        if digest(version / "asb-tui") != executable_digest:
            raise SystemExit("existing content-addressed version is substituted")
        existing = json.loads((version / "manifest.json").read_text(encoding="utf-8"))
        if existing != manifest:
            raise SystemExit("existing content-addressed manifest differs")
    else:
        stage = versions / f".stage-{os.getpid()}-{uuid.uuid4().hex}"
        stage.mkdir(mode=0o700)
        try:
            staged_executable = stage / "asb-tui"
            with executable.open("rb") as source, staged_executable.open("xb") as target:
                shutil.copyfileobj(source, target)
                target.flush()
                os.fsync(target.fileno())
            staged_executable.chmod(0o700)
            atomic_bytes(stage / "manifest.json", data, 0o600)
            os.replace(stage, version)
            directory = os.open(versions, os.O_RDONLY | getattr(os, "O_DIRECTORY", 0))
            try:
                os.fsync(directory)
            finally:
                os.close(directory)
        except Exception:
            shutil.rmtree(stage, ignore_errors=True)
            raise
    atomic_bytes(output / "active-dev.json", data, 0o600)
    return version


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--source-commit", required=True)
    parser.add_argument("--source-tree", required=True)
    parser.add_argument("--asb-source-commit", required=True)
    parser.add_argument("--asb-source-tree", required=True)
    parser.add_argument("--architecture", choices=tuple(TARGETS), default="x86_64")
    parser.add_argument("--built-unix", type=int, default=None)
    args = parser.parse_args()
    for name in ("source_commit", "source_tree", "asb_source_commit", "asb_source_tree"):
        value = getattr(args, name)
        if not valid_hex(value, 40):
            raise SystemExit(f"{name} must be lowercase 40-hex")
    if args.built_unix is not None and args.built_unix < 1:
        raise SystemExit("--built-unix must be positive")

    # Build in a detached worktree-like checkout.  This prevents local files
    # and a mutable branch checkout from influencing the artifact.
    with tempfile.TemporaryDirectory(prefix="asb-tui-dev-build-") as temporary:
        checkout = Path(temporary) / "source"
        target_dir = Path(temporary) / "target"
        # ``--no-local`` is deliberate: /tmp may be a different filesystem
        # from the checkout, where Git's local hard-link optimization fails.
        run("git", "clone", "--no-local", "--no-checkout", str(ROOT), str(checkout), cwd=ROOT)
        run("git", "-C", str(checkout), "checkout", "--detach", args.source_commit, cwd=ROOT)
        observed_commit = run("git", "-C", str(checkout), "rev-parse", "HEAD", cwd=ROOT)
        observed_tree = run("git", "-C", str(checkout), "rev-parse", "HEAD^{tree}", cwd=ROOT)
        if observed_commit != args.source_commit or observed_tree != args.source_tree:
            raise SystemExit("checked-out source identity does not match requested manifest")
        env = os.environ.copy()
        env.update(CARGO_TARGET_DIR=str(target_dir), ASB_TUI_SOURCE_COMMIT=args.source_commit,
                   ASB_TUI_SOURCE_TREE=args.source_tree)
        env["CARGO_BUILD_TARGET"] = TARGETS[args.architecture]
        run("cargo", "build", "--locked", "--release", "--bin", "asb-tui", cwd=checkout, env=env)
        executable = target_dir / TARGETS[args.architecture] / "release" / "asb-tui"
        if not regular(executable):
            raise SystemExit("cargo did not produce the release executable")
        output = args.output_dir.resolve()
        output.mkdir(parents=True, exist_ok=True)
        executable_digest = digest(executable)
        if executable.stat().st_size == 0 or executable.stat().st_size > MAX_EXECUTABLE_BYTES:
            raise SystemExit("release executable exceeds development size policy")
        built_unix = args.built_unix or int(time.time())
        manifest = {
            "schema_version": 1,
            "channel": "dev",
            "development_only": True,
            "source_repository": REPOSITORY,
            "source_ref": "refs/heads/main",
            "source_commit": args.source_commit,
            "source_tree": args.source_tree,
            "asb_source_commit": args.asb_source_commit,
            "asb_source_tree": args.asb_source_tree,
            "target": TARGETS[args.architecture],
            "executable_sha256": executable_digest,
            "executable_size": executable.stat().st_size,
            "built_unix": built_unix,
            "warnings": [
                "development_missing_authentication_allowed",
                "development_missing_signatures_allowed",
                "development_missing_key_management_allowed",
            ],
        }
        manifest_bytes = (json.dumps(manifest, indent=2, sort_keys=True) + "\n").encode()
        version = publish(output, executable, manifest, manifest_bytes)
        print(json.dumps({"bundle": str(version), "manifest": str(version / "manifest.json"),
                          "executable_sha256": executable_digest}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
