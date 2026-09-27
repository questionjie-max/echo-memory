#!/usr/bin/env bash
# Sign bundled helper binaries before Tauri creates the notarization archive.
set -euo pipefail

if [[ "$(uname -s)" != "Darwin" ]]; then
  exit 0
fi

if [[ -z "${APPLE_SIGNING_IDENTITY:-}" ]]; then
  echo "Skipping release resource signing: APPLE_SIGNING_IDENTITY is not set."
  exit 0
fi

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
resources_dir="$root_dir/src-tauri/resources"
binaries=(
  "$resources_dir/ffmpeg"
  "$resources_dir/echo-memory-mcp"
)

for binary in "${binaries[@]}"; do
  if [[ ! -f "$binary" ]]; then
    echo "Missing release resource: $binary" >&2
    exit 1
  fi

  codesign --force --options runtime --timestamp \
    --sign "$APPLE_SIGNING_IDENTITY" "$binary"
done

echo "Signed release resource binaries with $APPLE_SIGNING_IDENTITY."
