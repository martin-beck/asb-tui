#!/usr/bin/env bash
# Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
set -euo pipefail

scratch=$(mktemp -d)
cleanup() { rm -rf -- "$scratch"; }
trap cleanup EXIT HUP INT TERM

install_archive() {
  local name=$1 version=$2 asset=$3 digest=$4
  local archive="$scratch/$asset" extract="$scratch/$name-extract"
  mkdir -m 700 -- "$extract"
  curl --fail --location --proto '=https' --tlsv1.2 --max-filesize 20000000 \
    --output "$archive" "https://github.com/$5/releases/download/v${version}/${asset}"
  printf '%s  %s\n' "$digest" "$archive" | sha256sum --check --status
  tar -xzf "$archive" -C "$extract"
  local binary
  binary=$(find "$extract" -type f -name "$name" -perm -u+x -print -quit)
  test -n "$binary"
  install -m 0700 -- "$binary" "$scratch/$name"
}

install_archive actionlint 1.7.12 actionlint_1.7.12_linux_amd64.tar.gz \
  8aca8db96f1b94770f1b0d72b6dddcb1ebb8123cb3712530b08cc387b349a3d8 rhysd/actionlint
install_archive zizmor 1.30.0 zizmor-x86_64-unknown-linux-gnu.tar.gz \
  ec8c95cd800845abb9bbc5f377ec7c57d2eb8e2386a00a201d3a74ee4092e5ed zizmorcore/zizmor

"$scratch/actionlint" -config-file .github/actionlint.yaml
"$scratch/zizmor" --pedantic .
