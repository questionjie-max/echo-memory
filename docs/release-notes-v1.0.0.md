# Echo Memory v1.0.0

> The release that completes the capture side: in-app local microphone recording, a first-run wizard that no longer walls analysis setup behind an Ollama install, and the fixes found in a dedicated pre-release audit.

## Highlights

- **In-app local recording.** One click on the home screen ("开始记录") records your Mac's microphone as 48 kHz mono WAV, with pause/resume, a live input meter, and an explicit discard (with confirmation). When you stop, the audio is sealed into the library automatically, enters the transcription and analysis queue automatically, and shows up in the workspace — no manual import. If the app quits mid-recording, the unsealed session survives: at next launch it is offered for recovery or discard, never silently lost. Permission handling is explicit: a denied microphone shows where to fix it in System Settings instead of failing silently.
- **A slimmer first-run wizard, with analysis setup deferred until it matters.** The wizard is now 5 steps; the "analysis model" step that demanded a pre-installed Ollama is gone, and the environment check reports Ollama neutrally instead of blocking. Analysis is configured the first time you actually use it, through a panel with two tabs: "本地（Ollama 一键准备）" checks installation, service, and models and pulls what is missing with visible progress; "云端（填 Key 即用）" ships preset OpenAI-compatible endpoints for 智谱, DeepSeek, 阿里通义千问, 月之暗面 Kimi, MiniMax, 硅基流动, and OpenAI — pick a provider, fill in your API key, fetch the model list, and choose from it; manual model entry always works as a fallback. The panel can be closed and re-opened from Settings, does not re-pop within the same session, and resumes the action you were trying to run once setup completes. One boundary is stated in the tab itself: cloud analysis covers understanding only — knowledge indexing (embeddings) still requires local Ollama.
- **Six P1 findings from a dedicated pre-release audit, fixed.** Symptom-level: the recording UI could stay stuck at "正在记录" after stopping; a failed stop could leave the UI disagreeing with the backend; the one-click prepare button could stick at "正在准备…" forever; a background download could make local setup report readiness prematurely; switching cloud providers mid-fetch could pair one provider's model list with another's endpoint; and the in-app confirmation dialog could, in edge cases, be skipped without an answer — a destructive action confirmed unasked. All six are covered by the test suite.
- **Deeper transcription-fallback honesty.** When the third-party transcription channel is selected but incompletely configured (missing key or audio-upload consent), imports now fall back to the built-in Whisper *and say so once*, pointing to Settings — instead of switching silently. Channels that were never configured keep the previous silent fallback.

## Verification status

本版在发布前通过了完整的回归验证（后端与前端测试套件、生产构建、以及沙盒环境下的真机走查：5 步向导、录音卡片与权限提示、分析设置面板两个选项卡、AI 模型设置页）。有两项留待真实凭据完成：系统设置授权麦克风后的完整录音链路，以及用真实 API Key 拉取真实模型列表（各家 `/models` 支持度不一，手动填写为兜底）。

## Distribution status

This source release is version `1.0.0`. The public artifact is `Echo-Memory_1.0.0_aarch64.dmg` (14.7 MB), SHA-256 `1fd49746ee3053c7b326ce7ac494b223edbd0b26614d61288e1f92a9a826f3c3`.

- `codesign --verify --deep --strict` passes for the application inside the DMG.
- `hdiutil verify` confirms the disk image checksum is VALID.
- `spctl --assess --type execute` reports `accepted` with `source=Notarized Developer ID` for the application.
- This DMG is an **Apple-signed and notarized Developer ID release**. Notarization submission `9aace8dc-6131-40a3-9228-f7f917f08b23` completed with status `Accepted`, and the ticket is stapled to the DMG (`xcrun stapler validate` passes).
- A pre-notarization approval gate audited the built bundle (sourcemaps, test/dev leftovers, developer machine paths, dev endpoints, version consistency across config/Cargo/package/Info.plist) before submission.

Users can download the DMG, open it, and move `回声记忆.app` into `/Applications` normally.

---

# 回声记忆 v1.0.0

> 补齐采集侧的版本：应用内本地麦克风录音、不再把分析设置挡在 Ollama 安装后面的首次启动向导，以及一次专门的前置审计所发现的全部修复。

## 本版重点

- **应用内本地录音**。首页点一下「开始记录」就以 48kHz 单声道 WAV 录制本机麦克风，支持暂停/继续、实时输入电平和带确认的放弃；停止后音频自动封存入库、自动进入转写与分析队列，记录直接出现在工作区——无需手动导入。应用中途退出时未封存的录音会保留：下次启动给出「恢复或放弃」，绝不无声丢失。权限处理明确：麦克风被拒时会指出在系统设置的哪里解决，而不是静默失败。
- **首次启动向导瘦身，分析设置延后到真正要用时**。向导精简为 5 步；原本要求「先装好 Ollama」的分析模型步骤移除，环境体检对 Ollama 只做中性提示、不再拦截。分析能力在你第一次真正使用时引导配置：面板提供「本地（Ollama 一键准备）」——检查安装、服务与模型并带进度拉取缺失项——和「云端（填 Key 即用）」——预置智谱、DeepSeek、阿里通义千问、月之暗面 Kimi、MiniMax、硅基流动、OpenAI 的 OpenAI 兼容地址，选服务商、填 API Key、拉取模型列表即可选择；手填模型永远可用。面板可关闭并从设置重新打开，同一会话内不再自动弹出，配置完成后自动继续你原本要做的动作。选项卡内如实注明：云端只覆盖分析，知识索引（向量嵌入）仍需本机 Ollama。
- **专项前置审计发现的六项 P1 问题全部修复**。现象级描述：停止录音后界面可能停在「正在记录」；停止失败后界面可能与后端状态不一致；一键准备按钮可能永远停在「正在准备…」；后台下载可能让本地配置误报就绪；切换云端服务商可能把一家的模型列表配到另一家的地址；应用内确认弹窗在极端情况下可能不被回答就默认确认——破坏性操作被静默执行。六项均已进入测试套件。
- **转写回退更诚实**。当第三方转写通道被选中但未配置完整（缺 Key 或未确认音频上传说明）时，导入会回退到内置 Whisper **并明确提示一次**、指向设置补配——而不是悄悄切换；从未配置过的通道保持原有的静默回退。

## 验证状态

本版在发布前通过了完整的回归验证（后端与前端测试套件、生产构建，以及沙盒环境下的真机走查：5 步向导、录音卡片与权限提示、分析设置面板两个选项卡、AI 模型设置页）。有两项留待真实凭据完成：系统设置授权麦克风后的完整录音链路，以及用真实 API Key 拉取真实模型列表（各家 `/models` 支持度不一，手动填写为兜底）。

## 分发状态

本次源码发布版本为 `1.0.0`。公开产物为 `Echo-Memory_1.0.0_aarch64.dmg`（14.7 MB），SHA-256 `1fd49746ee3053c7b326ce7ac494b223edbd0b26614d61288e1f92a9a826f3c3`。

- `codesign --verify --deep --strict` 对 DMG 内应用校验通过。
- `hdiutil verify` 确认磁盘镜像校验和为 VALID。
- `spctl --assess --type execute` 对应用报告 `accepted`，来源 `source=Notarized Developer ID`。
- 本 DMG 为 **Apple 已签名并已公证的 Developer ID 发布物**。公证提交 `9aace8dc-6131-40a3-9228-f7f917f08b23` 状态 `Accepted`，装订票已钉入 DMG（`xcrun stapler validate` 通过）。
- 公证提交前，构建产物审计门（sourcemap、测试/开发残留、开发机路径、开发端点、config/Cargo/package/Info.plist 版本一致性）已通过。

用户下载 DMG 后正常打开、把 `回声记忆.app` 拖入 `/Applications` 即可使用。
