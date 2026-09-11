#!/bin/sh
# Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
set -eu

root=$(
  unset CDPATH
  cd -- "$(dirname -- "$0")/.."
  pwd
)
scratch=$(mktemp -d "${TMPDIR:-/tmp}/asb-tui-promoted-self-test.XXXXXX")
cleanup() {
  rm -rf -- "$scratch"
}
trap cleanup EXIT HUP INT TERM

mkdir "$scratch/source"
tar --exclude=.git --exclude=target -C "$root" -cf - . | tar -C "$scratch/source" -xf -
cp "$root/tests/fixtures/verified-channel-status.json" "$scratch/source/release/channel-status.json"

source_commit=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
source_tree=bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb
ASB_TUI_SOURCE_COMMIT=$source_commit \
  ASB_TUI_SOURCE_TREE=$source_tree \
  CARGO_TARGET_DIR="$scratch/target" \
  cargo build --locked --offline --manifest-path "$scratch/source/Cargo.toml"

mkdir -m 700 "$scratch/config" "$scratch/cache"
response="$scratch/response.json"
command="stty cols 80 rows 24; XDG_CONFIG_HOME='$scratch/config' XDG_CACHE_HOME='$scratch/cache' TERM=xterm-256color '$scratch/target/debug/asb-tui' lifecycle-self-test --release v0.1.0 --asb-version 0.1.0 --protocol-version 1 --format json >'$response'"
/usr/bin/timeout --signal=KILL 20s /usr/bin/script -q -e -c "$command" /dev/null >/dev/null

jq -e \
  --arg commit "$source_commit" \
  --arg tree "$source_tree" \
  '.schema_version == 1 and .classification == "verified_extension" and .release == "v0.1.0" and .source_commit == $commit and .source_tree == $tree and .asb_version == "0.1.0" and .protocol_version == 1 and .ready == true' \
  "$response" >/dev/null
printf '%s\n' "promoted executable self-test: ready"
