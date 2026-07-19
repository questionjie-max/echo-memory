#!/usr/bin/env bash
# 环境检查：确认 M0 开发所需工具链就绪。
# 用法：bash scripts/check-env.sh
set -u

pass=0
fail=0

check() {
  local name="$1"; shift
  if eval "$@" >/dev/null 2>&1; then
    echo "[OK]   $name"
    pass=$((pass + 1))
  else
    echo "[缺失] $name  ->  $*"
    fail=$((fail + 1))
  fi
}

echo "== 回声记忆 M0 环境检查 =="

check "Node.js (>=18)" "node -v"
check "npm" "npm -v"
check "Rust 工具链 rustc" "rustc --version"
check "Cargo" "cargo --version"
check "Xcode Command Line Tools" "xcode-select -p"
check "Clang (系统 WebView 编译所需)" "clang --version"
check "CMake (内嵌 Whisper 首次构建所需)" "cmake --version"

# Tauri CLI：优先 npm 脚本（@tauri-apps/cli），回退 cargo 全局
if [ -x "$(command -v npx)" ] && npm ls @tauri-apps/cli >/dev/null 2>&1; then
  check "Tauri CLI (npm)" "npx tauri --version"
else
  check "Tauri CLI (cargo 全局，可选)" "cargo tauri --version"
fi

echo "--------------------------"
echo "通过 $pass 项，缺失 $fail 项"
if [ "$fail" -ne 0 ]; then
  echo "请先补齐缺失项（见 app/README.md 的「前置依赖」）。"
  exit 1
fi
echo "环境就绪。"
