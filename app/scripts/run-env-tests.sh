#!/usr/bin/env bash
# 完整环境检查通过后，按独立 Cargo target 运行全部 ignored 真实环境测试。
# 用法：bash scripts/run-env-tests.sh
set -euo pipefail
export LC_ALL=C

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
bash "$ROOT_DIR/scripts/check-env.sh"

TARGETS=(
  whisperx_e2e_test
  analysis_benchmark_test
  chunk_level_ablation_test
  long_recording_ablation_test
  transcription_benchmark_test
  realworld_e2e_test
  analysis_long_transcript_e2e_test
  transcript_quality_e2e_test
  analysis_e2e_test
  transcription_e2e_test
)

cd "$ROOT_DIR/src-tauri"
for target in "${TARGETS[@]}"; do
  echo "== 运行 ignored 测试 target：$target =="
  cargo test --features mcp-bin --test "$target" -- --ignored --nocapture
done
