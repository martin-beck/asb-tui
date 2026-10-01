#!/usr/bin/env bash
# Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
set -euo pipefail

test -z "$(git status --porcelain)"
test -z "$(find . -maxdepth 1 -type f -name 'default_*.profraw' -print -quit)"
coverage_dir="$(mktemp -d "${TMPDIR:-/tmp}/asb-tui-coverage.XXXXXX")"
trap 'rm -rf "$coverage_dir"' EXIT
LLVM_PROFILE_FILE="$coverage_dir/%p-%m.profraw" \
  cargo llvm-cov --locked --all-targets --fail-under-lines 90
# Child-process boundary tests deliberately exercise the LLVM default when
# they clear the profile environment. Remove only those generated root files
# before the clean-tree assertions below.
find . -maxdepth 1 -type f -name 'default_*.profraw' -delete
git diff --exit-code
test -z "$(git status --porcelain)"
test -z "$(find . -maxdepth 1 -type f -name 'default_*.profraw' -print -quit)"
