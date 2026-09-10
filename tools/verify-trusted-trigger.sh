#!/usr/bin/env bash
# Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
set -euo pipefail

if [[ $# -ne 6 ]]; then
  exit 2
fi
event=$1 repository=$2 ref=$3 expected_sha=$4 checkout_sha=$5 remote_main=$6
case "$event" in push | workflow_dispatch) ;; *) exit 1 ;; esac
test "$repository" = martin-beck/asb-tui
test "$ref" = refs/heads/main
[[ "$expected_sha" =~ ^[0-9a-f]{40}$ ]]
test "$checkout_sha" = "$expected_sha"
test "$remote_main" = "$expected_sha"
