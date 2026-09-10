#!/usr/bin/env bash
# Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
set -euo pipefail

if [[ $# -ne 1 ]]; then
  printf 'usage: %s ASB_PRODUCT_ROOT\n' "$0" >&2
  exit 2
fi
product_root=$(realpath -- "$1")
test -f "$product_root/Cargo.toml"
case "$product_root" in *agent-systems-benchmark-asb-tui-separate-repository*) exit 2 ;; esac

# The core dependency graph must not acquire the separately installed frontend.
if cargo tree --locked --manifest-path "$product_root/Cargo.toml" -p asb-core |
  grep -F 'asb-tui'; then
  printf '%s\n' 'ASB core unexpectedly depends on asb-tui' >&2
  exit 1
fi
cargo test --locked --manifest-path "$product_root/Cargo.toml" -p asb-core
printf '%s\n' 'ASB core passed with no standalone TUI dependency'
