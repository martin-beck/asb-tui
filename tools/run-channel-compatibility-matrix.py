#!/usr/bin/env python3
"""Run the AR-1676 channel matrix against a disposable TUI executable.

Every command is attached to a fresh PTY and a private install/config root.
The runner never contacts a provider: the lifecycle cases use only local state
and an intentionally missing local repository for the rollback case.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import pty
import shutil
import subprocess
import tempfile
from pathlib import Path


def invoke(binary: Path, args: list[str], env: dict[str, str]) -> tuple[int, dict]:
    master, slave = pty.openpty()
    try:
        process = subprocess.Popen(
            [str(binary), *args],
            stdin=slave,
            stdout=slave,
            stderr=slave,
            env=env,
            close_fds=True,
        )
    finally:
        os.close(slave)
    output = bytearray()
    try:
        while True:
            try:
                block = os.read(master, 65536)
            except OSError:
                break
            if not block:
                break
            output.extend(block)
    finally:
        os.close(master)
    code = process.wait()
    lines = [line.strip() for line in output.decode("utf-8", "replace").splitlines()]
    for line in reversed(lines):
        try:
            return code, json.loads(line)
        except json.JSONDecodeError:
            continue
    raise AssertionError(f"no JSON response for {' '.join(args)}: {output!r}")


def check(name: str, code: int, response: dict, *, exit_code: int, result: str, channel: str = "dev") -> dict:
    assert code == exit_code, (name, code, response)
    assert response.get("code") == result, (name, response)
    assert response.get("channel") == channel, (name, response)
    return {"name": name, "exit_code": code, "response_code": result, "channel": channel}


def git_value(checkout: Path, revision: str) -> str:
    return subprocess.check_output(
        ["git", "-C", str(checkout), "rev-parse", revision], text=True
    ).strip()


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("binary", type=Path)
    parser.add_argument("--receipt", type=Path, required=True)
    parser.add_argument("--asb-head", help="exact ASB commit; defaults to the paired checkout HEAD")
    parser.add_argument(
        "--asb-checkout",
        type=Path,
        default=Path(os.environ.get("ASB_SOURCE_CHECKOUT", "/srv/data/projects/agent-systems-benchmark")),
        help="ASB checkout used to derive the exact commit and tree",
    )
    args = parser.parse_args()
    binary = args.binary.resolve()
    tui_checkout = Path(__file__).parents[1].resolve()
    asb_checkout = args.asb_checkout.resolve()
    asb_head = args.asb_head or git_value(asb_checkout, "HEAD")
    asb_tree = git_value(asb_checkout, f"{asb_head}^{{tree}}")
    tui_head = git_value(tui_checkout, "HEAD")
    tui_tree = git_value(tui_checkout, "HEAD^{tree}")
    manifest = tui_checkout / "release" / "channel-status.json"
    manifest_sha256 = hashlib.sha256(manifest.read_bytes()).hexdigest()
    root = Path(tempfile.mkdtemp(prefix="asb-tui-ar1676-"))
    try:
        install = root / "install"
        config = root / "config"
        state = config / "asb-tui" / "channel.json"
        env = os.environ.copy()
        env.update(
            {
                "ASB_TUI_DEV_INSTALL_ROOT": str(install),
                "ASB_TUI_CHANNEL_STATE": str(state),
                # A missing local repository makes the rollback case bounded
                # and proves that no network fallback is attempted.
                "ASB_TUI_DEV_REPOSITORY": str(root / "missing-local-repository"),
                "ASB_TUI_DEV_REF": "never-used",
                "ASB_TUI_NETWORK_POLICY": "deny",
                "HTTP_PROXY": "http://127.0.0.1:1",
                "HTTPS_PROXY": "http://127.0.0.1:1",
                "ALL_PROXY": "http://127.0.0.1:1",
                "NO_PROXY": "*",
            }
        )
        cases = []
        code, response = invoke(binary, ["tui", "status", "--json"], env)
        cases.append(check("omitted-dev", code, response, exit_code=3, result="development_not_installed"))
        code, response = invoke(binary, ["tui", "status", "--channel", "dev", "--json"], env)
        cases.append(check("explicit-dev", code, response, exit_code=3, result="development_not_installed"))
        for channel in ("stable", "nightly", "experimental"):
            code, response = invoke(binary, ["tui", "status", "--channel", channel, "--json"], env)
            cases.append(check(f"future-{channel}", code, response, exit_code=3, result=f"{channel}_channel_unavailable", channel=channel))

        # Explicit selection survives a fresh process, while explicit dev can
        # safely restore the available channel.
        code, response = invoke(binary, ["tui", "status", "--channel", "stable", "--json"], env)
        cases.append(check("persist-stable-selection", code, response, exit_code=3, result="stable_channel_unavailable", channel="stable"))
        code, response = invoke(binary, ["tui", "status", "--json"], env)
        cases.append(check("persisted-stable-selection", code, response, exit_code=3, result="stable_channel_unavailable", channel="stable"))
        code, response = invoke(binary, ["tui", "status", "--channel", "dev", "--json"], env)
        cases.append(check("restore-dev-selection", code, response, exit_code=3, result="development_not_installed"))

        install.mkdir(mode=0o700, parents=True, exist_ok=True)
        executable = install / "asb-tui"
        executable.write_bytes(b"candidate")
        provenance = {
            "schema_version": 1,
            "channel": "dev",
            "development_only": True,
            "source_commit": "a" * 40,
            "source_tree": "b" * 40,
            "executable_sha256": hashlib.sha256(b"candidate").hexdigest(),
            "source_repository": "local-fixture",
        }
        (install / "provenance.json").write_text(json.dumps(provenance), encoding="utf-8")
        before = (executable.read_bytes(), (install / "provenance.json").read_bytes())
        code, response = invoke(binary, ["tui", "upgrade", "--channel", "dev", "--json"], env)
        cases.append(check("rollback-preserves-channel", code, response, exit_code=3, result="development_command_failed"))
        assert (executable.read_bytes(), (install / "provenance.json").read_bytes()) == before
        code, response = invoke(binary, ["tui", "remove", "--channel", "dev", "--json"], env)
        cases.append(check("remove", code, response, exit_code=0, result="development_removed"))
        assert not executable.exists() and not (install / "provenance.json").exists()

        install.mkdir(mode=0o700, parents=True, exist_ok=True)
        (install / "asb-tui").write_bytes(b"candidate")
        (install / "provenance.json").write_text(
            json.dumps(
                {
                    "schema_version": 99,
                    "channel": "dev",
                    "development_only": True,
                    "source_commit": "a" * 40,
                    "source_tree": "b" * 40,
                    "executable_sha256": hashlib.sha256(b"wrong").hexdigest(),
                    "source_repository": "local-fixture",
                }
            ),
            encoding="utf-8",
        )
        code, response = invoke(binary, ["tui", "status", "--channel", "dev", "--json"], env)
        cases.append(check("malformed-manifest", code, response, exit_code=3, result="development_installation_invalid"))
        receipt = {
            "schema_version": 1,
            "ar": "AR-1676",
            "head": tui_head,
            "network": "denied",
            "credentials": "none",
            "cases": cases,
            "provenance": {
                "tui_commit": tui_head,
                "tui_tree": tui_tree,
                "asb_commit": asb_head,
                "asb_tree": asb_tree,
                "manifest": "release/channel-status.json",
                "manifest_sha256": manifest_sha256,
                "executable_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
            },
        }
        args.receipt.parent.mkdir(parents=True, exist_ok=True)
        args.receipt.write_text(json.dumps(receipt, indent=2) + "\n", encoding="utf-8")
        print(json.dumps(receipt, sort_keys=True))
        return 0
    finally:
        shutil.rmtree(root, ignore_errors=True)


if __name__ == "__main__":
    raise SystemExit(main())
