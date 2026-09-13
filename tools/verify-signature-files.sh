#!/usr/bin/env bash
# Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
set -euo pipefail

readonly PRINCIPAL=martin.beck2@gmx.de
readonly ALLOWED_SIGNERS=provenance/allowed_signers

ssh-keygen -Y verify -f "$ALLOWED_SIGNERS" -I "$PRINCIPAL" \
  -n asb-tui-release-lock -s provenance/dependencies.lock.json.sig \
  <provenance/dependencies.lock.json
ssh-keygen -Y verify -f "$ALLOWED_SIGNERS" -I "$PRINCIPAL" \
  -n asb-tui-bundle-v1 -s tests/fixtures/bundle/manifest.json.sig \
  <tests/fixtures/bundle/manifest.json
