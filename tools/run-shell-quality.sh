#!/usr/bin/env bash
# Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
set -euo pipefail

scratch=$(mktemp -d)
cleanup() { rm -rf -- "$scratch"; }
trap cleanup EXIT HUP INT TERM
curl --fail --location --proto '=https' --tlsv1.2 --max-filesize 20000000 \
  --output "$scratch/shellcheck.tar.xz" \
  https://github.com/koalaman/shellcheck/releases/download/v0.11.0/shellcheck-v0.11.0.linux.x86_64.tar.xz
printf '%s  %s\n' 8c3be12b05d5c177a04c29e3c78ce89ac86f1595681cab149b65b97c4e227198 \
  "$scratch/shellcheck.tar.xz" | sha256sum --check --status
tar -xJOf "$scratch/shellcheck.tar.xz" shellcheck-v0.11.0/shellcheck >"$scratch/shellcheck"
chmod 0700 "$scratch/shellcheck"

curl --fail --location --proto '=https' --tlsv1.2 --max-filesize 20000000 \
  --output "$scratch/shfmt" \
  https://github.com/mvdan/sh/releases/download/v3.14.1/shfmt_v3.14.1_linux_amd64
printf '%s  %s\n' 76e77641faa025814b77f153b29796b8e6fa2fca03e0c76a691608b86c7ea7bf \
  "$scratch/shfmt" | sha256sum --check --status
chmod 0700 "$scratch/shfmt"

"$scratch/shellcheck" tools/*.sh
"$scratch/shfmt" -d -i 2 -ci tools/*.sh
