#!/usr/bin/env bash
# Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
set -euo pipefail

version=8.30.1
archive=gitleaks_8.30.1_linux_x64.tar.gz
digest=551f6fc83ea457d62a0d98237cbad105af8d557003051f41f3e7ca7b3f2470eb
scratch=$(mktemp -d)
cleanup() { rm -rf -- "$scratch"; }
trap cleanup EXIT HUP INT TERM
curl --fail --location --proto '=https' --tlsv1.2 \
  --output "$scratch/$archive" \
  "https://github.com/gitleaks/gitleaks/releases/download/v${version}/${archive}"
printf '%s  %s\n' "$digest" "$scratch/$archive" | sha256sum --check --status
tar -xzf "$scratch/$archive" -C "$scratch" gitleaks
"$scratch/gitleaks" git --redact --no-banner "$@" .
