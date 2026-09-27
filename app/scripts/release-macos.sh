#!/usr/bin/env bash
# Build, notarize, staple, and verify one host-architecture macOS DMG.
set -euo pipefail
export LC_ALL=C

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR" || exit 1

bash "$ROOT_DIR/scripts/check-macos-signing.sh"

build_marker="$(mktemp "${TMPDIR:-/tmp}/echo-memory-release.XXXXXX")"
mount_dir="$(mktemp -d "${TMPDIR:-/tmp}/echo-memory-dmg.XXXXXX")"
device=""

cleanup() {
  if [ -n "$device" ]; then
    hdiutil detach "$device" -quiet >/dev/null 2>&1 || true
  fi
  rmdir "$mount_dir" >/dev/null 2>&1 || true
  rm -f "$build_marker"
}
trap cleanup EXIT

version="$(node -p "require('./src-tauri/tauri.conf.json').version")"

echo "== Building signed and notarized macOS DMG =="
"$ROOT_DIR/node_modules/.bin/tauri" build --bundles dmg --ci

bundle_dir="$ROOT_DIR/src-tauri/target/release/bundle/dmg"
dmg_path=""
while IFS= read -r candidate; do
  if [ -z "$dmg_path" ] || [ "$candidate" -nt "$dmg_path" ]; then
    dmg_path="$candidate"
  fi
done < <(
  find "$bundle_dir" -maxdepth 1 -type f -name "*_${version}_*.dmg" -newer "$build_marker" -print 2>/dev/null
)

if [ -z "$dmg_path" ]; then
  echo "No newly built $version DMG found in $bundle_dir" >&2
  exit 1
fi

echo "== Notarizing and stapling DMG =="
if [ -n "${NOTARYTOOL_KEYCHAIN_PROFILE:-}" ]; then
  xcrun notarytool submit "$dmg_path" \
    --keychain-profile "$NOTARYTOOL_KEYCHAIN_PROFILE" \
    --wait
elif [ -n "${APPLE_ID:-}" ] && [ -n "${APPLE_PASSWORD:-}" ] && [ -n "${APPLE_TEAM_ID:-}" ]; then
  xcrun notarytool submit "$dmg_path" \
    --apple-id "$APPLE_ID" \
    --password "$APPLE_PASSWORD" \
    --team-id "$APPLE_TEAM_ID" \
    --wait
elif [ -n "${APPLE_API_ISSUER:-}" ] && [ -n "${APPLE_API_KEY:-}" ] && [ -n "${APPLE_API_KEY_PATH:-}" ]; then
  xcrun notarytool submit "$dmg_path" \
    --key "$APPLE_API_KEY_PATH" \
    --key-id "$APPLE_API_KEY" \
    --issuer "$APPLE_API_ISSUER" \
    --wait
else
  echo "No notarization credentials available for the DMG." >&2
  exit 1
fi
xcrun stapler staple "$dmg_path"

echo "== Verifying DMG image and signature =="
hdiutil verify "$dmg_path"
codesign --verify --deep --strict --verbose=2 "$dmg_path"
spctl --assess --type install --verbose=4 "$dmg_path"
xcrun stapler validate "$dmg_path"

attach_output="$(hdiutil attach -readonly -nobrowse -noautoopen -mountpoint "$mount_dir" "$dmg_path")"
printf '%s\n' "$attach_output"
device="$(printf '%s\n' "$attach_output" | awk '/^\/dev\// { print $1; exit }')"
if [ -z "$device" ]; then
  echo "Could not determine the mounted DMG device" >&2
  exit 1
fi

app_path=""
app_count=0
while IFS= read -r candidate; do
  app_path="$candidate"
  app_count=$((app_count + 1))
done < <(find "$mount_dir" -mindepth 1 -maxdepth 1 -type d -name "*.app" -print)

if [ "$app_count" -ne 1 ]; then
  echo "Expected exactly one top-level .app in the DMG, found $app_count" >&2
  exit 1
fi

echo "== Verifying mounted application =="
codesign --verify --deep --strict --verbose=2 "$app_path"
spctl --assess --type execute --verbose=4 "$app_path"
xcrun stapler validate "$app_path"

echo "== Release artifact =="
printf 'DMG: %s\n' "$dmg_path"
shasum -a 256 "$dmg_path"
echo "macOS release verification passed."
