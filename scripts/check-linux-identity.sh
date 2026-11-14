#!/usr/bin/env bash
set -euo pipefail

# 使用独立的临时凭据库，不读写运行器已有身份；口令只用于本次 CI 测试。
identity_test_root="$(mktemp -d)"
trap 'rm -rf "$identity_test_root"' EXIT
export XDG_DATA_HOME="$identity_test_root/data"
export XDG_CONFIG_HOME="$identity_test_root/config"
mkdir -p "$XDG_DATA_HOME" "$XDG_CONFIG_HOME"
printf '%s' 'file-trans-ci-only' | gnome-keyring-daemon --unlock --components=secrets
identity_test_first="$(node scripts/rust.mjs run --quiet --locked -p file-trans-identity --example identity_probe -- "$identity_test_root/identity")"
identity_test_second="$(node scripts/rust.mjs run --quiet --locked -p file-trans-identity --example identity_probe -- "$identity_test_root/identity")"
test "$identity_test_first" = "$identity_test_second"
test "${#identity_test_first}" = 64
printf 'Linux Secret Service: identity preserved across processes\n'
