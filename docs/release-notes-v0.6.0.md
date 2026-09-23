# Echo Memory v0.6.0

## Highlights

- **Quality gates**: CI now runs frontend type checking and builds, growth/UI tests, Rust formatting, Clippy with warnings denied, Rust tests, and secret scanning. Local releases use the same deterministic gate.
- **Reliable bulk operations**: batch moves run in one database transaction and roll back together; batch deletion atomically removes database records, then performs best-effort file cleanup after commit and reports file failures separately.
- **Server-side privacy gate**: external AI snapshot generation, chat, and availability checks share one consent check. Calls that send data externally cannot start without a recorded privacy consent timestamp.
- **Library-scoped playback**: audio playback permissions follow the active library root instead of granting access to unrelated directories.
- **Event-driven refresh**: knowledge index updates are event-driven, with short fallback polling only while indexing or processing is active; idle views no longer poll continuously.
- **Regression coverage**: direct tests now cover record details, workspace bulk actions, knowledge chat, project navigation, assistant dock, growth view, onboarding, and event refresh behavior.
- **Monotonic transcription timeline**: chunked transcription now normalizes segment order and prevents end timestamps from moving backwards after overlap continuation.

## Verification

- Deterministic test baseline: TypeScript typecheck and frontend build pass; growth tests 9/9; UI tests 81/81; Rust format and Clippy checks pass; Rust tests execute 155 deterministic cases successfully.
- Release bundle baseline: frontend and release MCP binary build successfully; production npm dependencies report zero known vulnerabilities.
- macOS DMG baseline: `回声记忆_0.6.0_aarch64.dmg` passes `hdiutil verify` and launches successfully; SHA-256: `f463c75cfaf96b5c36e047db3845d4416c38763291d623d4f435a79897f29478`.
- Real long-recording regression: `benchmarks/results/20260923T183736Z-39422170.json` passed with Chinese language selection, 292 segments, a monotonic timeline, `accepted: true`, and no transcription error.
- Documentation status: known P0 product failures and the repeatable real long-recording regression are resolved; the archived JSON result remains the release evidence.

## Known boundaries

- Environment-dependent ignored tests still require a local Whisper model, real recordings, Ollama/model settings, Hugging Face credentials where applicable, and an isolated test library.
- The macOS DMG is built with ad-hoc signing only, is not Apple-notarized, and is intended for Alpha evaluation rather than a formally signed production release.
- The large Rust command, repository, and memory modules remain a follow-up refactoring item; splitting them is intentionally deferred until after this release stabilization.

---

# 回声记忆 v0.6.0

## 本版重点

- **质量门禁**：CI 现在依次执行前端类型检查与构建、成长时间线/UI 测试、Rust 格式检查、Clippy 全警告阻断、Rust 测试和密钥扫描；本地发布使用同一套确定性门禁。
- **可靠的批量操作**：批量移动在单个数据库事务中执行并统一回滚；批量删除先原子删除数据库记录，事务提交后再尽力清理文件，文件失败单独返回，不再伪装成数据库事务失败。
- **服务端隐私门禁**：外部 AI 快照生成、问答和可用性检查共用同一套同意校验；未记录隐私同意时间时，任何外发请求都不能构造。
- **跟随资料库的播放权限**：音频播放权限跟随当前资料库根目录，不再授予无关目录访问权限。
- **事件驱动刷新**：知识库索引优先通过事件刷新，只在索引或处理进行时短期兜底轮询；空闲页面不再持续产生轮询请求。
- **回归覆盖**：新增记录详情、工作区批量操作、知识问答、项目导航、助手面板、成长轨迹、首次配置和事件刷新的直接测试。
- **转写时间轴单调**：分块转写完成后统一规范化分段顺序，接续合并不再让结束时间向后回退。

## 验证结果

- 确定性基线：TypeScript 类型检查与前端构建通过；成长时间线测试 9/9；UI 测试 81/81；Rust 格式与 Clippy 检查通过；Rust 确定性测试共执行 155 个用例并全部通过。
- 发布构建基线：前端和 Release MCP 二进制构建通过；生产 npm 依赖已知漏洞为 0。
- macOS DMG 基线：`回声记忆_0.6.0_aarch64.dmg` 通过 `hdiutil verify` 并可直接启动；SHA-256：`f463c75cfaf96b5c36e047db3845d4416c38763291d623d4f435a79897f29478`。
- 真实长录音回归：`benchmarks/results/20260923T183736Z-39422170.json` 已通过，显式选择中文，共 292 段，时间轴单调，`accepted: true`，转写无错误。
- 文档状态：已知 P0 产品故障和可重复真实长录音回归均已解除，归档 JSON 作为发布证据。

## 已知边界

- 依赖真实环境的 `#[ignore]` 测试仍需要本机 Whisper 模型、真实录音、Ollama/分析模型配置、按需的 Hugging Face 凭证和隔离测试资料库。
- macOS DMG 仅采用 ad-hoc 签名，未进行 Apple 公证，定位为 Alpha 评估安装包，不是正式签名生产版本。
- `commands.rs`、`db/repository.rs` 和 `memory.rs` 仍是后续拆分对象；为避免混入本版稳定化变更，模块拆分有意延后处理。
