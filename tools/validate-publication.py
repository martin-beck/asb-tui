#!/usr/bin/env python3
# Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Validate public repository license and source-header policy."""

from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
COPYRIGHT = "Copyright (c) Huawei Technologies Co., Ltd. 2026"
LICENSE_COPYRIGHT = "Copyright (c) 2026 Huawei Technologies Co., Ltd."
SPDX = "SPDX-License-Identifier: MIT"
SUFFIXES = {".md", ".py", ".rs", ".sh", ".yaml", ".yml"}


def main() -> None:
    license_text = (ROOT / "LICENSE").read_text(encoding="utf-8")
    assert license_text.startswith("MIT License\n")
    assert LICENSE_COPYRIGHT in license_text

    missing = []
    for path in sorted(ROOT.rglob("*")):
        if not path.is_file() or path.suffix not in SUFFIXES:
            continue
        if ".git" in path.parts or "target" in path.parts:
            continue
        prefix = "\n".join(path.read_text(encoding="utf-8").splitlines()[:5])
        if COPYRIGHT not in prefix or SPDX not in prefix:
            missing.append(str(path.relative_to(ROOT)))
    assert not missing, f"missing copyright or SPDX headers: {missing}"


if __name__ == "__main__":
    main()
