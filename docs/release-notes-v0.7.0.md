# Echo Memory v0.7.0

## Highlights

- **Dual processing channels**: choose local transcription and analysis by default, or configure an OpenAI-compatible third-party ASR provider and text model as an enhanced path.
- **Enforced privacy gates**: third-party ASR requires completed provider settings, an API key, and a separate audio-upload consent. Third-party text analysis requires its own text-send consent. Missing configuration or consent fails before an external connection is created.
- **Speaker-aware third-party transcripts**: consume `speaker`, `speaker_label`, `speaker_id`, and `person` fields when a provider returns them. Normalize labels such as `SPEAKER_00` locally; do not infer speakers from text.
- **Single-record management**: the record context menu supports opening management, moving to another knowledge library, renaming, archiving/restoring, and deleting without requiring a prior checkbox selection.
- **Analysis quality states**: advisory quality warnings no longer force an otherwise complete analysis into an incomplete state; blocking failures still do.
- **Settings ergonomics**: long template and AI model settings pages use one stable scrolling container, and template sections can be added continuously through the documented limit with visible feedback.
- **Release quality gates**: frontend type checking/build, growth and UI tests, Rust formatting, Clippy with warnings denied, Rust tests, and production dependency auditing remain required.

## Verification status

- The deterministic frontend and Rust test suites pass locally before release packaging.
- Production npm dependencies report no known vulnerabilities when audited against the official npm registry.
- Third-party ASR and text-model integrations are covered by configuration, consent, transport-boundary, and parsing tests. They have not all been validated end to end with real commercial API credentials; this release does not claim every provider is certified.
- Environment-dependent ignored tests remain optional and require their documented local models, recordings, tokens, and isolated library.

## Distribution status

This source release is version `0.7.0`. The public artifact is `Echo-Memory_0.7.0_aarch64.dmg`, SHA-256 `caa5ea42dd1d8ff3f321e6ea8eca2db0a84f20ee802cd01125e938c262dadc92`.

- `codesign --verify --deep --strict` passes for both the packaged application and the DMG.
- `hdiutil verify` confirms the disk image is intact.
- `spctl --assess --type install` reports `accepted` with `source=Notarized Developer ID`.
- `xcrun stapler validate` passes for both the DMG and the mounted application.
- This DMG is an **Apple-signed and notarized Developer ID release**. Apple notarization submission `12853e50-c0df-4b94-93d2-62d43cbe3f18` completed with status `Accepted`.

Users can download the DMG, open it, and move `回声记忆.app` into `/Applications` normally.

---

# 回声记忆 v0.7.0

## 本版重点

- **双处理通道**：默认使用本机转写与分析，也可以配置 OpenAI-compatible 第三方 ASR 和文本模型，切换到效果优先的增强通道。
- **强制隐私门禁**：第三方转写必须完整配置服务商、地址、模型和 API Key，并单独同意上传原始音频；第三方文本分析有独立的文本发送同意。配置或同意缺失时，服务层会在建立外发连接前失败。
- **服务商说话人字段**：解析服务商真实返回的 `speaker`、`speaker_label`、`speaker_id`、`person`，本地化 `SPEAKER_00` 等标签；不根据文本猜测说话人。
- **单条记录管理**：右键菜单支持打开管理、转移知识库、重命名、归档/恢复和删除，无需先勾选。
- **分析质量状态**：建议级质量提醒不再把其余分析完整的记录标成“分析不完整”；阻断级失败仍保持未完成。
- **设置体验**：长模板列表与 AI 模型设置共用稳定滚动容器；模板栏目可连续添加到明确上限，并有自动可见与计数反馈。
- **发布质量门禁**：继续要求前端类型检查、构建、成长/UI 测试、Rust 格式、Clippy 全警告阻断、Rust 测试和生产依赖审计。

## 验证状态

- 发布打包前，本地确定性前端与 Rust 测试全部通过。
- 使用官方 npm registry 审计生产依赖时，已知漏洞为 0。
- 第三方 ASR 与文本模型已有配置、同意、传输边界和字段解析测试；尚未全部使用真实商业 API Key 完成端到端验证，因此本版不宣称所有服务商均已认证。
- 依赖真实模型、录音和 Token 的 `#[ignore]` 测试仍是可选任务，必须满足各自环境要求。

## 分发状态

本源码版本为 `0.7.0`，公开安装包为 `Echo-Memory_0.7.0_aarch64.dmg`，SHA-256 为 `caa5ea42dd1d8ff3f321e6ea8eca2db0a84f20ee802cd01125e938c262dadc92`。

- 应用程序与 DMG 均通过 `codesign --verify --deep --strict`。
- `hdiutil verify` 确认磁盘镜像完整。
- `spctl --assess --type install` 返回 `accepted`，来源为 `Notarized Developer ID`。
- `xcrun stapler validate` 对 DMG 和挂载后的应用均通过。
- 因此它明确标注为 **Apple 已签名并完成公证的 Developer ID 发布版**。公证提交 `12853e50-c0df-4b94-93d2-62d43cbe3f18` 状态为 `Accepted`。

macOS 用户下载 DMG 后可正常打开，并把 `回声记忆.app` 拖入 `/Applications`。
