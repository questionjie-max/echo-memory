# Echo Memory v1.1.0

> The release that completes the value side: system-audio capture, a real-time meeting minutes loop (live transcript → incremental minutes → post-hoc minutes → confirm-into-memory), and the plumbing that makes confirmed minutes visible to your Agent.

## Highlights

- **System-audio capture (功能一：系统内录).** The home recording card now offers three sources: 麦克风 / 系统声音 / 双路. "系统声音" records everything your Mac plays (the remote side of an online meeting) via ScreenCaptureKit with self-process exclusion (echo-safe); "双路" records microphone and system audio simultaneously and merges them into one stereo file (left = mic, right = system) aligned by the measured start offset, which is persisted per session for verification. Privacy is enforced at the API contract level: recording system audio requires an explicit confirmation dialog that states exactly who will be captured and where the audio stays, plus a backend `confirmed` gate — the frontend cannot bypass it. Permission denial fails loudly with System Settings guidance and never silently falls back.
- **Real-time transcript (块2).** While recording, a sliding-window local Whisper pass streams timestamped text into the recording card (final sentences + a provisional tail), with silence gating (RMS), a window cap, and continuation merging across windows reusing the full pipeline's overlap logic. Measured end-to-end latency on this machine: ≈3.2s with large-v3-turbo.
- **Post-hoc structured minutes (块3).** When transcription completes, minutes (topics / conclusions / action items / glossary) generate automatically in the background — generation failure never blocks the recording, it lands in a retryable failed state. Every conclusion and action item carries a verbatim quote validated against the transcript (fail-closed: unverifiable items are dropped and counted), so 100% of them link back to the source moment.
- **Confirm-into-memory (块4).** Minutes items have two states, provisional/confirmed. Only confirmed items enter the memory layer: MCP `get_my_context` exposes them (with record title, timestamp window, and verbatim quote) and the black-box probe of all seven MCP tools confirmed unconfirmed items are invisible on every exit path. Re-generation resets items to provisional — confirmation must target current content.
- **Live incremental minutes (块5).** During recording, a throttled engine (≥45s and ≥2 new finalized sentences) merges new speech into a growing draft shown as "实时纪要（暂定，以事后为准）". Cross-session buffer pollution and failure storms (retest loops, unbounded feed JSON) are explicitly guarded and unit-tested; the post-hoc minutes replace the live draft wholesale when you stop.
- **Speaker diarization as an experimental switch (块6).** whisperX stays off by default and is now explicitly labelled 实验 with an honest "runs entirely on this Mac, audio never leaves the machine" note; a contract test pins the default engine.
- **Terminology deepening (块7).** Minutes glossaries feed the hotword library and a new variant-mapping table automatically; searching a misspelled variant (substring, ≥2 chars) OR-expands to the canonical term and sibling variants, so wrong spellings still find the canonical transcript — in the app and over MCP.
- **Dual-source merge (块8).** Stop-time streaming merge with per-session offset persistence; cross-source mutual exclusion closes the parallel-capture gap; start/stop failures roll back or mark both sessions recoverable — never a stuck state, never silent capture.
- **Export (块9).** Markdown/TXT export now includes the minutes section (same content_json as the panel — 所见即所导), and a new SRT subtitle export renders per-segment timestamps with speaker prefixes.

## Known limitations

- 纪要质量取决于所选本机模型：实测 qwen2.5:7b 对纪要提取产出偏稀疏（15 句语音 → 3 议题 1 待办），管线 fail-closed 无编造。建议在设置中选择更强的分析模型。
- 实时文字实测延迟 ≈3.2s（1s 节拍 + 本机大模型窗口解码），略超 <3s 目标——如实记为实测值。
- 双路录音的暂停/恢复由编排器同拍驱动，两路各自的暂停操作差为亚秒级，计入对齐误差；转写管线预处理会把立体声混为单声道（文件层面始终保留分声道）。
- 变体搜索扩展为子串匹配（≥2 字），极短英文缩写可能带来额外召回（原查询仍参与，只加 OR 不加过滤）。

## Verification status

- Backend: `cargo test` 19 个测试目标全过（lib 257，块 1-9 净增 ~60）；前端 `vitest` 133 过；`tsc`/vite build/`cargo build` 全过。
- Real-machine E2E (automatable parts): system capture captured a played 440Hz tone (322,560 frames @48kHz, RMS≈11991); live transcript on a real speech clip (9 ticks, 9 finalized sentences, 0.90 reference coverage); live incremental minutes with real qwen2.5:7b structured output (3 topics + 1 action, timeline monotonic).
- Adversarial review per block: 每块独立对抗审核（多轮打回-修复-复审），块4 含 MCP 全工具黑盒探针（未确认内容全出口 0 泄露）。

## Not verified (needs a human)

- 首次系统声/双路录音的 TCC 权限弹窗需人手点（含真实麦克风+屏幕授权后的完整双路链路）。
- 真实会议中的实时文字/实时纪要/暂停继续体验，以及真机说话人分离效果（需 HuggingFace token 并接受 pyannote 协议）。
- 设置页 MCP 说明需补一句：「确认门槛只保护纪要条目层，逐字稿原文对 MCP 本来就可读（v1.0 by design）」。
- 发布物签名/公证（本版 DMG 为本机构建，见下）。

## Distribution status

This source release is version `1.1.0`. The DMG artifact is built locally on an Apple Silicon Mac; whether it is signed/notarized depends on the build environment used tonight and must be re-verified by a human before public distribution (v1.0.0 的签名公证流程需要 APPLE_SIGNING_IDENTITY 与 NOTARYTOOL_KEYCHAIN_PROFILE）。

---

# 回声记忆 v1.1.0

> 补齐价值侧的版本：系统内录、实时纪要闭环（实时文字 → 增量纪要 → 事后纪要 → 确认入记忆），以及让已确认纪要能被 Agent 读到的全部管道。

## 本版重点

- **系统内录（功能一）**。首页录音卡片提供三种来源：麦克风 / 系统声音 / 双路。「系统声音」通过 ScreenCaptureKit 录下这台 Mac 正在播放的所有声音（线上会议的远端），并排除自身进程避免回声；「双路」同时录麦克风与系统声，停止时按实测启动偏移合并为一个立体声文件（左=麦克风、右=系统声），偏移毫秒逐会话入库可查。隐私红线落在 API 契约上：录系统声必须先过显性告知确认框（讲清会录到谁的声音、音频只存本机、入记忆前还要确认），后端 `confirmed` 参数硬校验，前端绕不过去；权限被拒时明确报错并指引系统设置，绝不静默降级。
- **实时文字（块2）**。录音中，滑窗本机 Whisper 把带时间戳的文字流式送进录音卡片（定稿句 + 暂定尾），带静音门控（RMS）、窗口上限与跨窗接续合并（复用全量管线的重叠逻辑）。本机实测端到端延迟 ≈3.2s（large-v3-turbo）。
- **事后结构化纪要（块3）**。转写完成后后台自动生成纪要（议题/结论/待办/术语表）——生成失败绝不阻塞录音，落为可重试的 failed 状态。每条结论与待办都带逐字引文并经逐字稿校验（fail-closed：对不上出处的条目直接丢弃并计数），100% 可点回原文时间戳。
- **确认入记忆（块4）**。纪要条目分暂定/已确认两态；只有已确认条目进入记忆层：MCP `get_my_context` 暴露它们（含记录标题、时间窗、逐字引文），且对全部 7 个 MCP 工具的黑盒探针证实未确认内容在所有出口都不可见。重新生成会把条目重置回暂定——确认必须针对当下内容。
- **实时增量纪要（块5）**。录音中，节流引擎（≥45 秒且 ≥2 句新定稿）把新内容合并进生长中的草稿，以「实时纪要（暂定，以事后为准）」呈现；跨会话缓冲污染与失败风暴（重试循环、回喂 JSON 无上限）都有显式防护与单测；停止后事后纪要整体覆盖增量草稿。
- **说话人分离实验开关（块6）**。whisperX 保持默认关，并显式标注「实验」、如实说明「全部在这台 Mac 上运行，音频不出本机」；契约测试锁定默认引擎。
- **术语深化（块7）**。纪要术语表自动沉淀进热词库与新的变体映射表；搜索错写变体（子串、≥2 字）会 OR 扩展到规范写法与同词其它变体，错写也能命中规范原文——应用内与 MCP 同链路受益。
- **双路合并（块8）**。停止时流式合并并逐会话持久化偏移；跨源互斥收口并行采集缺口；启动/停止失败要么回滚两路、要么把两路标为可恢复——绝不卡死、绝不静默采集。
- **导出（块9）**。Markdown/TXT 导出带会议纪要段（与面板同一 content_json——所见即所导），新增 SRT 字幕导出（逐段时间戳 + 说话人前缀）。

## 已知局限

- 纪要质量取决于所选本机模型：实测 qwen2.5:7b 产出偏稀疏（15 句语音 → 3 议题 1 待办），管线 fail-closed 无编造。建议在设置中换更强的分析模型。
- 实时文字实测延迟 ≈3.2s（1 秒节拍 + 本机大模型窗口解码），略超 <3s 目标——如实记为实测值。
- 双路录音暂停/恢复由编排器同拍驱动，两路操作差为亚秒级，计入对齐误差；转写预处理会把立体声混为单声道（文件层面始终保留分声道）。
- 变体搜索扩展为子串匹配（≥2 字），极短英文缩写可能带来额外召回（只加 OR 不加过滤）。

## 验证状态

- 后端 `cargo test` 19 个测试目标全过（lib 257，块 1-9 净增约 60 项）；前端 vitest 133 过；tsc、vite 构建、cargo 构建全过。
- 真机可自动化部分：系统声采集实测录到播放的 440Hz 测试音（322,560 帧 @48kHz，RMS≈11991）；实时文字在真实语音片段上 9 拍 9 句定稿、参考覆盖率 0.90；实时增量纪要用真实 qwen2.5:7b 结构化输出（3 议题 + 1 待办，时间轴单调）。
- 每块独立对抗审核（多轮打回-修复-复审），块4 含 MCP 全工具黑盒探针（未确认内容全出口 0 泄露）。

## 未验证项（需要人做）

- 首次系统声/双路录音的 TCC 权限弹窗需人手点（含真实麦克风+屏幕授权后的完整双路链路）。
- 真实会议中的实时文字/实时纪要/暂停继续体验，以及真机说话人分离效果（需 HuggingFace token 并接受 pyannote 协议）。
- 设置页 MCP 说明补一句：「确认门槛只保护纪要条目层，逐字稿原文对 MCP 本来就可读（v1.0 by design）」。
- 发布物签名/公证（本版 DMG 为本机构建，见下）。

## 分发状态

源码版本 `1.1.0`。DMG 在 Apple Silicon Mac 本机构建；是否签名/公证取决于当晚构建环境，公开发布前必须由人重新核验（v1.0.0 的签名公证流程需要 APPLE_SIGNING_IDENTITY 与 NOTARYTOOL_KEYCHAIN_PROFILE）。
