# Echo Memory v0.9.0

> Cumulative release: this brings together the v0.8 "make what you saved retrievable" work and the v0.9 "analysis quality + engineering guardrails" work, plus the new workspace content grouping. v0.8.0 was never published separately.

## Highlights

- **Confirmed memories are now retrievable.** Knowledge chat runs hybrid retrieval (vector + full-text, RRF-fused) over records *and* confirmed memory items. Answers label the two sources separately (资料原文 / 你的过往记忆), memory citations carry verbatim quotes and can jump back to the record or the original audio, and every quote is verified word for word against the transcript — a failing citation is dropped, never shown.
- **AI conversations are a first-class input, in their own lane.** ChatGPT/Claude exports, conversational Markdown, and a local agent-session scanner (Claude Code, Codex) import as searchable records. They stay out of the recording memory chain (evolution / growth / timeline), skip per-record analysis by design, and no longer flood the pending queue. A batch "digest report" distills an imported archive into what you were actually working on.
- **A personal profile (L3).** A cross-snapshot, first-person summary of your stances, preferences, trajectory, and what you are watching. Generated under the existing two-consent gate, versioned with the other memory views, and exposed read-only to agents through the new `get_my_context` MCP tool.
- **Per-record analysis instructions.** Fill a supplementary request in a record's analysis panel — for example "keep only action items, grouped by owner". It outranks the template (highest priority in every generation, retry, and long-transcript merge prompt), and the implied section selection is enforced deterministically in code rather than trusted to the model. Verbatim-citation and safety rules cannot be overridden by an instruction.
- **Chapters and verbatim quotes.** Long recordings get 2–6 clickable chapters that seek the player, plus 0–5 quotes that must match the transcript word for word or they are discarded. Both flow into search and Markdown export.
- **Anti-fabrication, tuned by measurement.** The analysis prompt now states an uncertainty rule: gaps are only reported when the transcript really is unclear or incomplete, and a complete transcript must not be given a fabricated "gaps" note — an earlier wording was found (via a live end-to-end run) to induce exactly that behavior, and was tightened.
- **The workspace groups content into 录音 / 文档 / AI 对话.** Each group header collapses, shows its count and the formats it accepts, and carries that category's import entry. Content and entry live in the same place; collapse state is remembered per category.
- **Storage is visible and cleanable.** A storage panel reports per-category usage (audio / intermediates / models / exports / database), clears regenerable intermediates in one click, and a banner warns when the disk runs low. Original audio is never auto-deleted: in a cloud-first product the local copy is a cache, but here the recording is the asset.
- **Bounded self-recovery and input correctness.** Transient transcription failures retry twice with backoff (60 s / 180 s) while deterministic failures fail immediately; jobs interrupted by an app quit resume at next launch; disk space is pre-checked before import, preprocessing, and model download; and `AAC` audio — previously advertised by the inbox watcher but rejected at import — now imports.
- **Release hygiene as a gate.** The release script audits the built app before notarization and CI audits the frontend bundle: no sourcemaps, no test/dev leftovers, no developer machine paths, no dev endpoints, production CSP free of dev websocket channels, version consistency across config/Cargo/package/Info.plist. Self-testing this gate against the published v0.7.1 build found 595 occurrences of the developer's home path in the main binary; both root causes are fixed (release builds remap recorded paths, and the development-only migrations fallback no longer bakes `CARGO_MANIFEST_DIR` into release binaries).

## Verification status

- Rust: 236 tests pass across all targets; `cargo clippy --all-targets --features mcp-bin -- -D warnings` and `cargo fmt --check` are clean.
- Frontend: 113 UI tests (15 files, including 6 new workspace-grouping cases), growth-timeline tests, `tsc --noEmit`, and the production build all pass.
- Analysis quality E2E (`ECHO_E2E=1`, real local Ollama with `qwen2.5:7b`): three scenarios pass — long transcript (6 chapters, monotonic timestamps, verified citations), instruction coverage (0 key points / 0 decisions / 3 action items under "keep only action items"), and sparse material (no invented decisions, todos, or questions; the summary states the material limitation).
- Release artifact audit: passes for the frontend bundle; the pre-notarization audit runs inside `npm run release:macos`.
- Known gap in this repository's history: v0.8.0 was never tagged. This notes release covers it.

## Distribution status

This source release is version `0.9.0`. The public artifact is `Echo-Memory_0.9.0_aarch64.dmg`, SHA-256 `b0b132a5ef92bf3c97315f07e8e3226caec9c7ddbe8964b284e0a4bbb3ead8b9`.

- `codesign --verify --deep --strict` passes for the application inside the DMG.
- `hdiutil verify` confirms the disk image is intact.
- `spctl --assess --type execute` reports `accepted` with `source=Notarized Developer ID` for the mounted application.
- This DMG is an **Apple-signed and notarized Developer ID release**. Notarization submission `81224d6d-94c1-4a6b-9546-f86896146f3e` completed with status `Accepted`, and the ticket is stapled to the DMG (`xcrun stapler validate` passes).
- A pre-notarization approval gate audits the built bundle (sourcemaps, test/dev leftovers, developer machine paths, dev endpoints, CSP dev websocket channels, version consistency); this release passed after the audit caught and eliminated two real developer-path leaks in the compiled binary (see the engineering notes above).

Users can download the DMG, open it, and move `回声记忆.app` into `/Applications` normally.

---

# 回声记忆 v0.9.0

> 累积发布：把 v0.8「让沉淀可被召回」与 v0.9「分析输出质量 + 工程护栏」一并送上，外加新的工作区内容分组。v0.8.0 未曾单独发布过。

## 本版重点

- **确认过的记忆真能召回了**：问知识库在资料原文与已确认记忆条目上做混合检索（向量 + 全文，RRF 融合）。回答把两类来源分开标注（资料原文 / 你的过往记忆）；记忆引用带逐字引文，可跳回原记录或回听原声；每条引用都逐字比对逐字稿，对不上就丢弃，绝不展示编造引用。
- **AI 对话成为一等输入，且走独立车道**：ChatGPT / Claude 导出、对话式 Markdown、本机 Agent 会话扫描（Claude Code、Codex）都作为可搜索记录入库；不进入录音记忆链（演化 / 成长 / 时间线）、按设计免逐条分析，也不再淹没「待处理」队列。批次「梳理报告」能把整批导入归纳成你当时真正在推进的事情。
- **个人总述（L3）**：跨快照聚合的第一人称画像（立场、偏好、怎么变过来的、正在关注什么），沿用既有的双同意门禁，与其他记忆视图同样版本化，并通过新的 `get_my_context` 只读 MCP 工具提供给授权 Agent。
- **单条记录的分析补充要求**：在记录的分析面板填「只保留待办事项，按负责人分组」这类要求即可覆盖模板（在每一次生成、纠错重试与长文合并提示里都是最高优先级），而且栏目取舍由代码确定性执行，不赌模型遵从；逐字引用与安全底线不可被指令改变。
- **章节与金句**：长录音自动切成 2–6 个可点击跳播的章节，并给出 0–5 条必须逐字命中的金句（对不上直接丢弃）。两者都进入搜索与 Markdown 导出。
- **防幻觉靠实测调校**：分析提示词新增不确定性规则——只有逐字稿确实听不清或缺失时才说明缺口，材料完整时禁止杜撰缺口说明（旧措辞经真实端到端运行证实会诱发模型凭空写「有 N 处内容不清晰」，已收紧）。
- **工作区按录音 / 文档 / AI 对话三类分组**：组头可折叠、显示该类计数与支持的格式，右侧就是这一类的导入入口——入口和内容在同一处；折叠状态按类目记忆。
- **存储可见可控**：存储面板分类显示占用（录音 / 中间文件 / 模型 / 产出 / 数据库），一键清理可再生中间文件，磁盘紧张时给出提醒。原件永不自动删除：在云端优先的产品里本地副本只是缓存，而在这里录音本身就是资产。
- **有限自愈与输入正确性**：转写瞬时失败自动重试两次（退避 60 / 180 秒），确定性失败立即交还用户；应用退出中断的任务下次启动自动重排；导入、预处理与模型下载前都做磁盘空间预检；`AAC` 音频（收件箱一直声称支持、导入却拒收）现已可正常导入。
- **发布卫生变成门禁**：发布脚本在公证前审计构建产物、CI 审计前端包——不含 sourcemap、测试/开发残留、开发机路径、开发端点；生产 CSP 不含开发 websocket 通道；配置 / Cargo / package / Info.plist 版本号一致。用这道门禁自测已发布的 v0.7.1，在主二进制里查出 595 处开发机家目录路径；两个根因均已修复（release 构建重映射记录路径，开发专用的迁移回退不再把 `CARGO_MANIFEST_DIR` 编进 release 二进制）。

## 验证状态

- Rust：全部目标 236 项测试通过；`cargo clippy --all-targets --features mcp-bin -- -D warnings` 与 `cargo fmt --check` 干净。
- 前端：113 项 UI 测试（15 个文件，含 6 项新的工作区分组用例）、成长时间线测试、`tsc --noEmit` 与生产构建全部通过。
- 分析质量端到端（`ECHO_E2E=1`，本机真实 Ollama + `qwen2.5:7b`）：三场景全过——长文（6 章节、时间戳递增、引用全部可验证）、指令覆盖（「只保留待办事项」下 0 观点 / 0 决策 / 3 待办）、材料不足（不编造决策待办与问题，摘要如实说明材料情况）。
- 发布产物审计：前端包通过；公证前审计已内置在 `npm run release:macos` 中。
- 历史账目说明：v0.8.0 从未打过 tag，本版说明覆盖它。

## 分发状态

本源码版本为 `0.9.0`，公开安装包为 `Echo-Memory_0.9.0_aarch64.dmg`，SHA-256 为 `b0b132a5ef92bf3c97315f07e8e3226caec9c7ddbe8964b284e0a4bbb3ead8b9`。

- DMG 内应用程序通过 `codesign --verify --deep --strict`。
- `hdiutil verify` 确认磁盘镜像完整。
- 挂载后的应用经 `spctl --assess --type execute` 判定为 `accepted`，来源 `Notarized Developer ID`。
- 因此它明确标注为 **Apple 已签名并完成公证的 Developer ID 发布版**。公证提交 `81224d6d-94c1-4a6b-9546-f86896146f3e` 状态为 `Accepted`，装订票已贴在 DMG 上（`xcrun stapler validate` 通过）。
- 公证前包含审批门禁审计（sourcemap、测试/开发残留、开发机路径、开发端点、CSP 开发 websocket 通道、版本号一致性）：本次发布在门禁抓出并消除编译二进制里的两处真实开发机路径泄漏后通过（见上文工程说明）。

macOS 用户下载 DMG 后可正常打开，并把 `回声记忆.app` 拖入 `/Applications`。
