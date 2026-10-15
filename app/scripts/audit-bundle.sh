#!/usr/bin/env bash
# 发布产物审计：防「开发残留随包发出」这类回归。
#
# 反面教材来自得到大脑逆向报告（2026-10-14）：正式包内夹带了
# .claude/settings.local.json（开发机路径）、内部测试源码、开发域白名单与明文 HTTP 上报。
# 本脚本把同类问题变成可执行的发布门禁。
#
# 用法：bash scripts/audit-bundle.sh <目标目录>
#   目标可以是 app/dist（前端产物）或已构建的 *.app（发布包）。
#   有问题时输出明细并以非零码退出；干净时打印 ✓ 并退出 0。
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
target="${1:-}"
if [ -z "$target" ] || [ ! -e "$target" ]; then
  echo "用法：bash scripts/audit-bundle.sh <dist 或 *.app>" >&2
  exit 2
fi
target="$(cd "$(dirname "$target")" && pwd)/$(basename "$target")"

findings=0
fail() { echo "  ✗ $1"; findings=$((findings + 1)); }
pass() { echo "  ✓ $1"; }

echo "== 审计目标：$target =="

# 1) 前端不得带 sourcemap（源码泄漏面）
map_files="$(find "$target" -name '*.map' -type f -print 2>/dev/null || true)"
if [ -n "$map_files" ]; then
  fail "发现 sourcemap："
  echo "$map_files" | sed 's/^/      /'
else
  pass "无 sourcemap"
fi

# 2) 测试与开发残留不得随包
leftovers="$(find "$target" \
  \( -name '*.test.*' -o -name '*.spec.*' -o -name '*.tsbuildinfo' -o -name '.env*' \
     -o -name '.DS_Store' -o -name '.claude*' -o -name 'node_modules' \) \
  -print 2>/dev/null || true)"
if [ -n "$leftovers" ]; then
  fail "发现测试/开发残留："
  echo "$leftovers" | sed 's/^/      /'
else
  pass "无测试/开发残留"
fi

# 3) 开发机路径串（开发机绝对路径、仓库位置）。文本产物严格；Mach-O 二进制单独用
#    strings 检查——release 构建已用 --remap-path-prefix 消除这类路径，这里守住回归。
user_marker="/Users/$(whoami)"
path_hits="$(grep -rla --binary-files=without-match \
  -e "$user_marker" -e "Desktop/SOLOPLAY" "$target" 2>/dev/null || true)"
binary_hits=""
while IFS= read -r binary; do
  # 用 grep -c 而不是 grep -q：-q 会提前退出让 strings 收到 SIGPIPE，
  # 叠加 pipefail 后整个管道变成非零，检查会永远"通过"（假绿）。
  marker_count="$(strings -a "$binary" 2>/dev/null | grep -c -e "$user_marker" -e "Desktop/SOLOPLAY" || true)"
  if [ "${marker_count:-0}" -gt 0 ]; then
    binary_hits+="$binary （${marker_count} 处）"$'\n'
  fi
done < <(find "$target" -type f \( -perm -111 -o -name '*.dylib' -o -name '*.node' \) -print 2>/dev/null || true)
if [ -n "$path_hits" ] || [ -n "$binary_hits" ]; then
  fail "发现开发机路径串（$user_marker / Desktop/SOLOPLAY）："
  [ -n "$path_hits" ] && echo "$path_hits" | sed 's/^/      /'
  [ -n "$binary_hits" ] && echo "$binary_hits" | sed 's/^/      /'
else
  pass "无本机路径串"
fi

# 4) 开发端点不得出现在文本产物里。
#    二进制的 devUrl（http://localhost:1420）来自内嵌的 Tauri 配置，属预期，不在检查范围。
endpoint_hits="$(grep -rla --binary-files=without-match \
  -e "localhost:1420" -e "127.0.0.1:1420" \
  -e "localhost:5173" -e "localhost:5174" -e "127.0.0.1:5173" -e "127.0.0.1:5174" \
  "$target" 2>/dev/null || true)"
if [ -n "$endpoint_hits" ]; then
  fail "发现开发端点残留："
  echo "$endpoint_hits" | sed 's/^/      /'
else
  pass "文本产物无开发端点残留"
fi

# 5) 生产 CSP 不得为开发热更新保留 localhost ws 通道（它们应只出现在 devCsp 里）。
#    这是配置源检查：内嵌配置会让 devCsp 里合法出现 ws://localhost。
csp_value="$(node -p "require('$ROOT_DIR/src-tauri/tauri.conf.json').app.security.csp" 2>/dev/null || echo "")"
case "$csp_value" in
  *"ws://localhost"*|*"ws://127.0.0.1"*)
    fail "tauri.conf.json 的 csp 仍含开发 ws 通道（应移到 devCsp）"
    ;;
  *)
    pass "生产 csp 无开发 ws 通道"
    ;;
esac

# 6) 版本号一致性：tauri.conf.json = Cargo.toml = package.json
config_version="$(node -p "require('$ROOT_DIR/src-tauri/tauri.conf.json').version")"
cargo_version="$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT_DIR/src-tauri/Cargo.toml" | head -1)"
pkg_version="$(node -p "require('$ROOT_DIR/package.json').version")"
if [ "$config_version" = "$cargo_version" ] && [ "$config_version" = "$pkg_version" ]; then
  pass "版本号一致（${config_version}）"
else
  fail "版本号不一致：tauri.conf=$config_version cargo=$cargo_version package=$pkg_version"
fi

# 7) .app 追加检查：Info.plist 版本一致 + entitlements 摘要（人工过目最小权限）
case "$target" in
  *.app)
    plist_version="$(defaults read "$target/Contents/Info.plist" CFBundleShortVersionString 2>/dev/null || echo "?")"
    if [ "$plist_version" = "$config_version" ]; then
      pass "Info.plist 版本一致（${plist_version}）"
    else
      fail "Info.plist 版本 $plist_version 与配置版本 $config_version 不一致"
    fi
    echo "  · entitlements（人工过目，最小权限原则）："
    codesign -d --entitlements - "$target" 2>/dev/null | sed 's/^/      /' || echo "      （未签名或无法读取）"
    ;;
esac

echo
if [ "$findings" -gt 0 ]; then
  echo "审计未通过：$findings 处问题，先修复再发布。" >&2
  exit 1
fi
echo "审计通过：产物干净。"
