#!/usr/bin/env bash
# Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
set -euo pipefail

readonly AWQ_VERSION=0.32.0
readonly AWQ_WHEEL_SHA256=bf610d925de51dfd643e47c467bd82fcad128dc43793bae1a0aeebd5b5ea4848

if [[ $# -lt 3 ]]; then
  printf '%s\n' 'usage: run-awq-shadow.sh PYTHON VERIFIED_WHEEL AWQ_ARG...' >&2
  exit 2
fi

python=$1
wheel=$2
shift 2

if [[ ! -f "$wheel" || -L "$wheel" ]]; then
  printf '%s\n' 'AWQ tool-error: wheel must be an existing regular non-symlink file' >&2
  exit 1
fi
printf '%s  %s\n' "$AWQ_WHEEL_SHA256" "$wheel" | sha256sum --check --status || {
  printf '%s\n' 'AWQ tool-error: v0.32.0 wheel digest mismatch' >&2
  exit 1
}

scratch=$(mktemp -d)
cleanup() { rm -rf -- "$scratch"; }
trap cleanup EXIT HUP INT TERM

"$python" -m venv "$scratch/venv"
"$scratch/venv/bin/python" -m pip install \
  --disable-pip-version-check --no-index --no-deps "$wheel" >/dev/null

installed_version=$(
  "$scratch/venv/bin/python" -c \
    'from importlib.metadata import version; print(version("agent-workflow-quality"))'
)
if [[ "$installed_version" != "$AWQ_VERSION" ]]; then
  printf '%s\n' 'AWQ tool-error: installed version does not match v0.32.0' >&2
  exit 1
fi

"$scratch/venv/bin/python" -m awq "$@"
