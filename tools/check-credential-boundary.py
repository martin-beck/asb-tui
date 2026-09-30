#!/usr/bin/env python3
# Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Fail-closed regression guard for the TUI credential boundary.

This is development/prototype assurance. It does not claim keychain storage,
provider authorization, or production security qualification.
"""

from pathlib import Path
import re
import sys

ROOT = Path(__file__).resolve().parents[1]
FORBIDDEN = re.compile(
    r"\b(?:api[_-]?key|access[_-]?token|password|private[_-]?key|secret|helper[_-]?path|executable[_-]?path)\b",
    re.IGNORECASE,
)


def block(source: str, name: str) -> str:
    match = re.search(rf"pub struct {name}\s*\{{(.*?)\n\}}", source, re.S)
    if not match:
        raise ValueError(f"cannot locate {name}")
    return match.group(1)


def main() -> int:
    errors: list[str] = []
    helper = (ROOT / "src/credential_helper.rs").read_text(encoding="utf-8")
    receipt = block(helper, "CredentialEnrollmentReceipt")
    for field in ("provider", "endpoint_identity_sha256", "credential_locator_sha256"):
        if not re.search(rf"pub {field}\s*:", receipt):
            errors.append(f"receipt is missing {field}")
    if FORBIDDEN.search(receipt):
        errors.append("receipt exposes a raw credential or helper locator field")
    if "deny_unknown_fields" not in helper:
        errors.append("receipt boundary is not closed against unknown fields")

    codec = (ROOT / "src/control_codec.rs").read_text(encoding="utf-8")
    enroll = block(codec, "AuthEnrollParams")
    invoke = block(codec, "AuthHelperInvokeParams")
    if FORBIDDEN.search(enroll) or FORBIDDEN.search(invoke):
        errors.append("control auth DTO accepts a raw credential or helper path")
    for field in ("endpoint_identity_sha256", "credential_locator_sha256"):
        if field not in enroll:
            errors.append(f"auth enrollment is missing {field}")
    if "profile" not in invoke or "idempotency_key" not in invoke:
        errors.append("helper invocation is missing the credential-free profile contract")

    configuration = (ROOT / "src/configuration.rs").read_text(encoding="utf-8")
    if "fn reject_secret_shapes" not in configuration:
        errors.append("configuration import lacks secret-shape rejection")
    for marker in ("token", "password", "credential", "api_key", "private_key"):
        if f'"{marker}"' not in configuration:
            errors.append(f"configuration rejection omits {marker}")

    if errors:
        print("credential boundary check failed:", file=sys.stderr)
        for error in errors:
            print(f"- {error}", file=sys.stderr)
        return 1
    print("credential boundary check passed: digest-only DTOs and secret-shape rejection are present")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
