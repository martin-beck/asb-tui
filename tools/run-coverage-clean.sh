#!/usr/bin/env bash
# Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
set -euo pipefail

test -z "$(git status --porcelain)"
test -z "$(find . -maxdepth 1 -type f -name 'default_*.profraw' -print -quit)"
cargo llvm-cov --locked --all-targets --fail-under-lines 90
git diff --exit-code
test -z "$(git status --porcelain)"
test -z "$(find . -maxdepth 1 -type f -name 'default_*.profraw' -print -quit)"
