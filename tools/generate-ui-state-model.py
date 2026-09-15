#!/usr/bin/env python3
# Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
"""Generate the canonical, reviewable UI model artifact (AR-1183).

The JSON in ``docs/ui-state-model.json`` is the authored source.  This command
normalizes it and writes the generated artifact.  CI compares the generated
bytes, so editing or omitting the generated file cannot silently pass review.
"""
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / "docs" / "ui-state-model.json"
GENERATED = ROOT / "docs" / "ui-state-model.generated.json"


def canonical(value):
    return json.dumps(value, indent=2, ensure_ascii=False) + "\n"


def main() -> int:
    try:
        model = json.loads(SOURCE.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise SystemExit(f"cannot read authored UI model: {error}")
    GENERATED.write_text(canonical(model), encoding="utf-8")
    return 0


if __name__ == "__main__":
    main()
