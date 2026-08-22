# Echo Memory v0.2.0 Alpha

中文说明见本文件后半部分。

## Highlights

- Reliability: background heavy jobs (Whisper transcription, Ollama analysis, and knowledge indexing) now run through a process-wide concurrency limiter. Importing many recordings at once queues local-model work instead of launching it all simultaneously and starving the machine. The limit defaults to 2 and can be tuned with `ECHO_MAX_HEAVY_JOBS` (1–8).
- Configuration: the local Ollama address is now resolved in one place. `OLLAMA_HOST` is honored consistently by analysis, knowledge indexing, and model downloads (a scheme-less value such as `127.0.0.1:11434` is accepted and normalized).
- Resilience: an application-wide error boundary keeps the window usable with a recovery path if a rendering error occurs, instead of showing a blank screen. Library data is untouched either way.
- Security hardening: release builds no longer probe the build machine's `target/` directory when locating the bundled MCP executable.
- Code health: removed the unused demo views and the template boilerplate commands (`greet`, `app_info`, `generate_id`) from the IPC surface; deduplicated time-formatting and processing-status helpers shared across components.
- Tests: new unit tests for the concurrency limiter and Ollama address resolution; the full suite (typecheck, frontend tests, `cargo fmt --check`, `cargo test --features mcp-bin`) passes.

## Requirements

This is a macOS source release. It requires Node.js, Rust, local Whisper support, and optionally local Ollama for analysis. No signed installer is provided in this release.

## Privacy

The current Alpha keeps audio and transcripts local. MCP is disabled by default, read-only, and does not open a public port. External AI remains opt-in, text-only, and never receives raw audio.

## Known limitations

- A local Whisper model must be available for real transcription.
- A local Ollama service and model are required for analysis.
- Real-world long-recording and offline regression coverage is still being expanded.
- No cloud sync, mobile client, Windows client, team collaboration, payment system, or MCP write support is included.

---

# 回声记忆 v0.2.0 Alpha

## 主要更新

- 稳定性：Whisper 转写、Ollama 分析与知识索引等后台重任务统一经过进程级并发闸门。批量导入多条录音时，本机模型任务会排队执行，不再同时全部启动拖垮整机。默认上限 2，可用环境变量 `ECHO_MAX_HEAVY_JOBS`（1–8）调整。
- 配置一致性：本机 Ollama 地址改为统一解析，分析、知识索引、模型下载都遵循 `OLLAMA_HOST`（允许 `127.0.0.1:11434` 这类省略 scheme 的写法，会自动归一化）。
- 容错性：新增全应用错误兜底。界面渲染出错时展示可恢复的提示页，而不是白屏；资料库数据不受影响。
- 安全加固：release 构建不再探测编译机的 `target/` 目录来定位随包 MCP 可执行文件。
- 代码健康：删除未使用的演示视图与模板样板命令（`greet`、`app_info`、`generate_id`），收窄 IPC 暴露面；抽取组件间重复的时间格式化与处理状态判断工具。
- 测试：新增并发闸门与 Ollama 地址解析的单元测试；完整验证命令（typecheck、前端测试、`cargo fmt --check`、`cargo test --features mcp-bin`）全部通过。

## 环境要求

这是 macOS 源码发布，需要 Node.js、Rust、本机 Whisper 支持；使用分析功能时还需要本机 Ollama。此版本不提供签名安装包。

## 隐私边界

当前 Alpha 将音频和逐字稿留在本机。MCP 默认关闭、只读、不监听公网端口。外部 AI 仍为可选功能，只发送文本，永不上传原始音频。

## 已知限制

- 真实转写需要可用的本机 Whisper 模型。
- 分析需要本机运行 Ollama 服务与模型。
- 长录音与断网场景的真实回归仍在持续补充。
- 不包含云同步、手机端、Windows、团队协作、支付体系或 MCP 写入。
