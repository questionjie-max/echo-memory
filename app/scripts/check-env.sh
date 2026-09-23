#!/usr/bin/env bash
# 检查真实模型/真实录音 E2E 所需的完整环境；任一缺失都显式失败。
# 用法：bash scripts/check-env.sh
set -u
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
  printf '[缺失] %s\n' "$1"
  failed=$((failed + 1))
}

check_command() {
  local name="$1"
  local command="$2"
  if command -v "$command" >/dev/null 2>&1; then
    ok "$name"
  else
    missing "$name（未找到命令：$command）"
  fi
}

check_file() {
  local name="$1"
  local path="${2:-}"
  if [ -n "$path" ] && [ -f "$path" ]; then
    ok "$name：$path"
  else
    missing "$name（要求存在文件，当前值：${path:-未设置}）"
  fi
}

check_directory() {
  local name="$1"
  local path="${2:-}"
  if [ -n "$path" ] && [ -d "$path" ]; then
    ok "$name：$path"
  else
    missing "$name（要求存在目录，当前值：${path:-未设置}）"
  fi
}

check_value() {
  local name="$1"
  local value="${2:-}"
  if [ -n "$value" ]; then
    ok "$name"
  else
    missing "$name（环境变量未设置）"
  fi
}

check_switch() {
  local name="$1"
  local value="${2:-}"
  if [ "$value" = "1" ]; then
    ok "$name=1"
  else
    missing "$name 必须显式设为 1（当前值：${value:-未设置}）"
  fi
}

echo "== 真实环境测试依赖检查（一次列全） =="
echo "-- 基础工具链 --"
check_command "Node.js" node
check_command "npm" npm
check_command "Rust 编译器 rustc" rustc
check_command "Cargo" cargo
if xcode-select -p >/dev/null 2>&1; then
  ok "Xcode Command Line Tools"
else
  missing "Xcode Command Line Tools（xcode-select -p 失败）"
fi
check_command "Clang" clang
check_command "CMake" cmake
if [ -x "$ROOT_DIR/node_modules/.bin/tauri" ]; then
  ok "Tauri CLI（npm）"
elif command -v cargo >/dev/null 2>&1 && cargo tauri --version >/dev/null 2>&1; then
  ok "Tauri CLI（cargo）"
else
  missing "Tauri CLI（既无 node_modules/.bin/tauri，也无 cargo tauri）"
fi

echo "-- 真实模型与服务 --"
check_file "Whisper 模型（ECHO_WHISPER_MODEL）" "${ECHO_WHISPER_MODEL:-}"
if [ -n "${ECHO_WHISPERX_BIN:-}" ] && [ -f "${ECHO_WHISPERX_BIN:-}" ]; then
  ok "whisperX 命令：${ECHO_WHISPERX_BIN}"
elif command -v whisperx >/dev/null 2>&1; then
  ok "whisperX 命令"
else
  missing "whisperX 命令（设置 ECHO_WHISPERX_BIN 或加入 PATH）"
fi
check_command "Ollama" ollama
check_value "HuggingFace token（ECHO_HF_TOKEN）" "${ECHO_HF_TOKEN:-}"
check_value "分析模型（ECHO_ANALYSIS_MODEL）" "${ECHO_ANALYSIS_MODEL:-}"

echo "-- 真实音频与资料库 --"
check_file "真实测试音频（ECHO_TEST_AUDIO）" "${ECHO_TEST_AUDIO:-}"
check_file "真实长录音（ECHO_BENCH_REAL 第一项）" "${ECHO_BENCH_REAL%%;*}"
check_value "真实录音语言（ECHO_BENCH_REAL_LANG）" "${ECHO_BENCH_REAL_LANG:-}"
check_file "真实 E2E 会议音频（/tmp/echo-e2e/meeting.wav）" "/tmp/echo-e2e/meeting.wav"
check_directory "资料库根目录（ECHO_LIBRARY_ROOT）" "${ECHO_LIBRARY_ROOT:-}"
check_value "测试记录 ID（ECHO_TEST_RECORD_ID）" "${ECHO_TEST_RECORD_ID:-}"
check_value "测试分析 ID（ECHO_TEST_ANALYSIS_ID）" "${ECHO_TEST_ANALYSIS_ID:-}"

echo "-- 高成本测试开关 --"
check_switch "ECHO_E2E" "${ECHO_E2E:-}"
check_switch "ECHO_BENCH" "${ECHO_BENCH:-}"
check_switch "ECHO_ABLATE" "${ECHO_ABLATE:-}"
check_switch "ECHO_BENCH_ANALYSIS" "${ECHO_BENCH_ANALYSIS:-}"
check_switch "ECHO_E2E_WHISPERX" "${ECHO_E2E_WHISPERX:-}"

echo "--------------------------"
echo "通过 $passed 项，缺失 $failed 项"
if [ "$failed" -ne 0 ]; then
  echo "环境未就绪；未静默跳过任何缺失项。"
  exit 1
fi
echo "真实环境测试依赖已就绪。"
