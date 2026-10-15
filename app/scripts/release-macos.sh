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
# 路径重映射：release 二进制的 panic/断点信息会记录编译时的绝对路径（本机用户名与
# 仓库位置）。发布包面向公开下载，这里统一改写成中性前缀，审计脚本会守住这条。
# 两套都要做：RUSTFLAGS 管 rustc 记录的位置；whisper.cpp 由 whisper-rs-sys 的 cmake
# 构建，其 __FILE__/调试信息路径只认 CMAKE_* 透传（该 crate 的 build.rs 会把以
# CMAKE_/WHISPER_ 开头的环境变量 define 进 cmake），CFLAGS/CXXFLAGS 传不到。
# 工程根先映射到 /project，否则 $HOME 映射后仍会留下 Desktop/SOLOPLAY 这类可识别的目录名。
# 代价是构建标志变化会触发依赖重编译（每个发布周期一次）。
export RUSTFLAGS="${RUSTFLAGS:-} --remap-path-prefix=$HOME=/build"
export CMAKE_C_FLAGS="${CMAKE_C_FLAGS:-} -ffile-prefix-map=$ROOT_DIR=/project -ffile-prefix-map=$HOME=/build"
export CMAKE_CXX_FLAGS="${CMAKE_CXX_FLAGS:-} -ffile-prefix-map=$ROOT_DIR=/project -ffile-prefix-map=$HOME=/build"
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

echo "== Auditing the built DMG before notarization =="
# 审计对象是 DMG 里的 .app：Tauri 打完 DMG 会清掉中间产物目录里的 .app，
# 而 DMG 内的才是用户真正拿到的东西。有问题直接中止，省掉一次无效公证。
audit_mount="$(mktemp -d "${TMPDIR:-/tmp}/echo-memory-audit.XXXXXX")"
if ! hdiutil attach "$dmg_path" -nobrowse -readonly -mountpoint "$audit_mount" -quiet; then
  echo "Failed to mount $dmg_path for auditing" >&2
  exit 1
fi
audited_app="$(find "$audit_mount" -maxdepth 1 -name '*.app' -print -quit 2>/dev/null)"
if [ -z "$audited_app" ]; then
  hdiutil detach "$audit_mount" -quiet || true
  echo "No .app found inside $dmg_path; cannot audit" >&2
  exit 1
fi
set +e
bash "$ROOT_DIR/scripts/audit-bundle.sh" "$audited_app"
audit_status=$?
set -e
hdiutil detach "$audit_mount" -quiet || true
rmdir "$audit_mount" 2>/dev/null || true
if [ "$audit_status" -ne 0 ]; then
  echo "Bundle audit failed; aborting before notarization" >&2
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
# 装订票贴在 DMG 上（分发单位就是 DMG），DMG 内的 .app 本身不带票——
# 这里再校验 .app 的票会必然失败（stapler 返回 65）。验收看两件事：
# 上面 spctl 是否返回 accepted/Notarized Developer ID，以及第 106 行对 DMG 的 stapler validate。

echo "== Release artifact =="
printf 'DMG: %s\n' "$dmg_path"
shasum -a 256 "$dmg_path"
echo "macOS release verification passed."
