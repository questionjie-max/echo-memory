# Echo Memory v0.3.0 Alpha

中文说明见本文件后半部分。

## Highlights

Theme: **open capture and first-run readiness**. Echo Memory stays a standalone, hardware-agnostic product — any recorder that can put a file on your Mac works.

- Onboarding wizard: a six-step first-run flow (privacy statement, environment check, Whisper model with **resumable download**, Ollama analysis model, inbox setup, finish). Every step is skippable; the app degrades gracefully to import/manage/search until models are configured. The wizard can be re-run from Settings.
- Audio inbox: watch any folders (Downloads, iCloud Drive, WeChat save locations — suggestions offered automatically) and optionally auto-scan USB volumes when a recorder is plugged in. New audio files are detected, stability-checked (partial files and iCloud `.icloud` placeholders are skipped), hash-deduplicated, copied into the library, and queued through the existing bounded transcription/analysis pipeline. Status is visible in Settings and in the workspace panel.
- Personal hotword vocabulary: manage terms in Settings; they are injected into the Whisper initial prompt, materially improving Chinese proper-noun recognition.
- Transcript AI correction: an optional local-LLM pass fixes homophones, punctuation, and hotword spellings after transcription. Results are written to the normalized text layer — the original transcript and manual edits are never overwritten. Available per-record ("AI 校对逐字稿") or automatically after inbox imports.
- Related records: each record detail now shows the most similar records computed from the existing local embeddings.
- Action dashboard: a new "行动" view aggregates action items and open questions across all records with completion toggles and one-click jumps back to the cited evidence.
- Whisper model downloads now resume after interruption (HTTP Range) and are guarded against duplicate concurrent downloads.

## Requirements

This is a macOS source release. It requires Node.js, Rust, local Whisper support, and optionally local Ollama for analysis. No signed installer is provided in this release.

## Privacy

The current Alpha keeps audio and transcripts local. Folder watching and USB scanning only read directories you explicitly configure. MCP is disabled by default, read-only, and does not open a public port. External AI remains opt-in, text-only, and never receives raw audio.

## Known limitations

- Watch-folder scanning is polling-based (about every 4 seconds); instant FSEvents-based notification may come later.
- Speaker diarization is not yet available; all segments are labeled "未知".
- Real-world long-recording and offline regression coverage is still being expanded.
- No cloud sync, mobile client, Windows client, team collaboration, payment system, or MCP write support is included.

---

# 回声记忆 v0.3.0 Alpha

## 主要更新

主题：**开放接入 · 开箱即用**。回声记忆保持独立产品定位，不绑定任何硬件——任何能把文件放到 Mac 上的录音设备都可以用。

- 首次启动引导：六步向导（隐私声明 → 环境体检 → Whisper 模型（**支持断点续传**）→ Ollama 分析模型 → 收件箱配置 → 完成）。每一步都可跳过；未配置模型时可先导入、管理和搜索，稍后补配。设置里可重新运行引导。
- 音频收件箱：监听任意文件夹（自动推荐下载文件夹、iCloud 云盘等），可选在插入 USB 录音设备时自动扫描。新音频文件经过写稳检测（跳过半成品文件与 iCloud 占位文件）、哈希去重后自动复制入库，进入既有的限流转写分析队列。状态在设置与工作区面板实时可见。
- 个人词汇库：在设置中维护热词，自动注入 Whisper 转写提示，显著改善中文专有名词（人名、产品名）识别率。
- 转写 AI 校对：可选的本机模型二次校对（同音字、标点断句、热词纠正）。结果写入独立文本层，原始逐字稿与手动编辑永不覆盖；支持单条记录手动触发或收件箱导入后自动执行。
- 相关记录：记录详情页基于本地向量相似度展示最相关的记录。
- 行动仪表盘：新增「行动」视图，跨记录聚合行动项与未解决问题，可标记完成并一键跳回引用的原始证据。
- Whisper 模型下载支持中断续传（HTTP Range），并防止重复并发下载。

## 环境要求

这是 macOS 源码发布，需要 Node.js、Rust、本机 Whisper 支持；使用分析功能时还需要本机 Ollama。此版本不提供签名安装包。

## 隐私边界

当前 Alpha 将音频和逐字稿留在本机。文件夹监听与 USB 扫描只读取你明确配置的目录。MCP 默认关闭、只读、不监听公网端口。外部 AI 仍为可选功能，只发送文本，永不上传原始音频。

## 已知限制

- 文件夹监听基于轻量轮询（约每 4 秒），后续可能升级为 FSEvents 实时通知。
- 暂无说话人分离，所有片段说话人为「未知」。
- 长录音与断网场景的真实回归仍在持续补充。
- 不包含云同步、手机端、Windows、团队协作、支付体系或 MCP 写入。
