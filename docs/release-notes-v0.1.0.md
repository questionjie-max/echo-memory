# Echo Memory v0.1.0 Alpha

中文说明见本文件后半部分。

## Highlights

- Local audio import for MP3, M4A, and WAV.
- Local timestamped transcription, playback seeking, and non-destructive transcript edits.
- Local structured analysis with source-backed decisions and action items.
- Project knowledge libraries and local full-text search.
- User-enabled, local stdio, read-only MCP access for AI tools.

## Requirements

This is a macOS source release. It requires Node.js, Rust, local Whisper support, and optionally local Ollama for analysis. No signed installer is provided in this release.

## Privacy

The current Alpha keeps audio and transcripts local. MCP is disabled by default, read-only, and does not open a public port.

## Known limitations

- A local Whisper model must be available for real transcription.
- A local Ollama service and model are required for analysis.
- Real-world long-recording and offline regression coverage is still being expanded.
- No cloud sync, mobile client, Windows client, team collaboration, payment system, or MCP write support is included.

---

# 回声记忆 v0.1.0 Alpha

## 主要能力

- 本地导入 MP3、M4A、WAV。
- 本地生成带时间点的逐字稿，支持播放定位与非破坏性编辑。
- 本地结构化分析，并为决策与待办保留原文出处。
- 项目知识库与本地全文搜索。
- 用户主动开启的本机 stdio、只读 MCP，供 AI 工具查询历史上下文。

## 环境要求

这是 macOS 源码发布，需要 Node.js、Rust、本机 Whisper 支持；使用分析功能时还需要本机 Ollama。此版本不提供签名安装包。

## 隐私边界

当前 Alpha 将音频和逐字稿留在本机。MCP 默认关闭、只读，且不监听公网端口。

## 已知限制

- 真实转写需要可用的本机 Whisper 模型。
- 分析需要本机运行 Ollama 服务与模型。
- 长录音与断网场景的真实回归仍在持续补充。
- 不包含云同步、手机端、Windows、团队协作、支付体系或 MCP 写入。
