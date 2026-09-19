# 转写质量基准

用 macOS `say` 合成、文本真值精确已知的语音片段，量化内嵌 Whisper 引擎的转写质量
（CER 字错误率、时间轴单调性、耗时），并作为后续调参与回归的统一标尺。

## 目录结构

- `clips/` — 音频（m4a）与对应真值文本（`.txt`）。`03-dialogue` 另有说话人轮次真值（txt 内 `名字：内容` 逐行）。
- `manifest.json` — 每条片段的时长、SHA-256、语言、真值、备注。
- `generate.sh` — 重新生成全部音频（`say` → 系统 ffmpeg → m4a）。声音可用环境变量覆盖（`ZH_FEMALE`/`ZH_MALE`/`EN_VOICE`）。
- `results/` — 基准跑器的 JSON 明细（每次运行一个文件，按时间戳 + 模型哈希命名）。
- `RESULTS.md` — 汇总表（人工维护结论）。
- `../app/src-tauri/tests/transcription_benchmark_test.rs` — 基准跑器。

## 跑法

```bash
# 前置：本机有一个 Whisper 模型（默认会找真实库 models/ 下最大的 .bin，
# 也可用 ECHO_WHISPER_MODEL 显式指定路径）
ECHO_BENCH=1 cargo test --features mcp-bin --test transcription_benchmark -- --nocapture

# 附加真实录音稳定性对照（无真值，不算 CER，只记录转写与耗时）：
ECHO_BENCH=1 ECHO_BENCH_REAL="/path/a.m4a;/path/b.wav" cargo test --features mcp-bin --test transcription_benchmark -- --nocapture
```

## 片段设计

| id | 时长 | 内容 | 目的 |
|---|---|---|---|
| 01-short | ~25s | 中文短句，含数字与日期 | 基础准确率 |
| 02-medium | ~83s | 中文多主题，专有名词 + 易混词 | 词汇难度（单块内） |
| 03-dialogue | ~50s | 双人对话 8 轮（男女声交替） | whisperX 说话人分离真值 |
| 04-noisy | ~83s | 02 + 粉噪声（强度 0.18） | 抗噪 |
| 05-english | ~36s | 英文 | 语言处理 |

CER 比较口径：双方去除标点与空白，只保留文字/数字/字母，再算字级编辑距离。
