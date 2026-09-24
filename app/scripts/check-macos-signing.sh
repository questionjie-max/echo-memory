#!/usr/bin/env bash
# Check every local prerequisite before producing a signed macOS release.
set -uo pipefail
export LC_ALL=C

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR" || exit 1

passed=0
failed=0

ok() {
  printf '[OK]   %s\n' "$1"
  passed=$((passed + 1))
}

missing() {
  printf '[MISSING] %s\n' "$1"
  failed=$((failed + 1))
}

check_command() {
  local name="$1"
  local command_name="$2"
  if command -v "$command_name" >/dev/null 2>&1; then
    ok "$name"
  else
    missing "$name (command not found: $command_name)"
  fi
}

echo "== macOS signing preflight =="
echo "-- Platform and toolchain --"
if [ "$(uname -s)" = "Darwin" ]; then
  ok "macOS"
else
  missing "macOS (current OS: $(uname -s))"
fi

check_command "Node.js" node
check_command "npx" npx
check_command "security" security
check_command "codesign" codesign
check_command "spctl" spctl
check_command "xcrun" xcrun
check_command "hdiutil" hdiutil
check_command "SHA-256 tool" shasum

if command -v xcode-select >/dev/null 2>&1 && xcode-select -p >/dev/null 2>&1; then
  ok "Xcode Command Line Tools"
else
  missing "Xcode Command Line Tools (xcode-select -p failed)"
fi

if [ -x "$ROOT_DIR/node_modules/.bin/tauri" ]; then
  ok "Tauri CLI (local npm dependency)"
else
  missing "Tauri CLI (run npm ci before releasing)"
fi

echo "-- Developer ID signing identity --"
signing_identity="${APPLE_SIGNING_IDENTITY:-}"
if [ -z "$signing_identity" ]; then
  missing "APPLE_SIGNING_IDENTITY"
else
  identities="$(security find-identity -v -p codesigning 2>/dev/null || true)"
  identity_line="$(printf '%s\n' "$identities" | grep -F "\"$signing_identity\"" || true)"
  if [ -z "$identity_line" ]; then
    missing "Valid codesigning identity matching APPLE_SIGNING_IDENTITY"
  elif printf '%s\n' "$identity_line" | grep -F "Developer ID Application:" >/dev/null 2>&1; then
    ok "Developer ID Application identity: $signing_identity"
  else
    missing "Developer ID Application identity (the matching identity has another type)"
  fi
fi

echo "-- Apple notarization credentials --"
apple_id_ready=1
apple_id_missing=""
for name in APPLE_ID APPLE_PASSWORD APPLE_TEAM_ID; do
  if [ -z "${!name:-}" ]; then
    apple_id_ready=0
    apple_id_missing="$apple_id_missing $name"
  fi
done

api_key_ready=1
api_key_missing=""
for name in APPLE_API_ISSUER APPLE_API_KEY APPLE_API_KEY_PATH; do
  if [ -z "${!name:-}" ]; then
    api_key_ready=0
    api_key_missing="$api_key_missing $name"
  fi
done

if [ -n "${APPLE_API_KEY_PATH:-}" ] && [ ! -f "${APPLE_API_KEY_PATH}" ]; then
  api_key_ready=0
  api_key_missing="$api_key_missing APPLE_API_KEY_PATH(file)"
fi

if [ "$apple_id_ready" -eq 1 ]; then
  ok "Apple ID app-specific password credentials"
fi
if [ "$api_key_ready" -eq 1 ]; then
  ok "App Store Connect API key credentials"
fi

if [ "$apple_id_ready" -eq 0 ] && [ -n "$apple_id_missing" ]; then
  if [ "$apple_id_missing" = " APPLE_ID APPLE_PASSWORD APPLE_TEAM_ID" ]; then
    # This alternative is reported only when neither complete mode was found.
    :
  else
    missing "Apple ID mode is incomplete:$apple_id_missing"
  fi
fi

if [ "$api_key_ready" -eq 0 ] && [ -n "$api_key_missing" ]; then
  if [ "$api_key_missing" = " APPLE_API_ISSUER APPLE_API_KEY APPLE_API_KEY_PATH" ]; then
    :
  else
    missing "API key mode is incomplete:$api_key_missing"
  fi
fi

if [ "$apple_id_ready" -eq 0 ] && [ "$api_key_ready" -eq 0 ]; then
  missing "Notarization credentials (set APPLE_ID + APPLE_PASSWORD + APPLE_TEAM_ID, or APPLE_API_ISSUER + APPLE_API_KEY + APPLE_API_KEY_PATH)"
fi

echo "--------------------------"
echo "Passed $passed checks; missing $failed checks"
if [ "$failed" -ne 0 ]; then
  echo "Signing preflight failed; the release command will not start."
  exit 1
fi
echo "macOS signing prerequisites are ready."
