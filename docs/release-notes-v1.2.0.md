# Echo Memory v1.2.0

> The verification-first release: a full usability audit of v1.0+v1.1 (12 scenarios, zero P0, five P1 findings all fixed), then four bounded expansions — the import confirmation gate that extends the privacy red line to imported content, a time-range MCP tool for confirmed minutes, a repeatable minutes-quality healthcheck, and a one-click export package for a single record.

## Highlights

- **Usability audit + P1 fixes（块1-3）.** A 12-scenario walkthrough (docs/usability-audit-v1.2.md) verified the product end to end with a real-binary MCP black-box probe: **P0 = 0**（未确认纪要条目经 11 个出口 0 泄漏，确认门完好）。Five P1 findings fixed, one commit each with regression tests: MCP scope copy now states both layers honestly (record原文 is readable over MCP by design; memory entries are confirmed-only); chat-import copy no longer claims analysis that doesn't happen; agent-session scan no longer defaults to select-all; the onboarding re-run actually re-pops after closing settings; live-minutes no longer degrades silently when Ollama is missing (one-shot honest notice per session).
- **Import confirmation gate（块4，定位命门）.** Imported documents and AI-chat records now start provisional: `get_record` / `get_transcript` / `search_records` / `get_project_context` / `list_action_items` return **nothing** for them — including analysis-derived decisions and action items — until the user hits 确认入记忆 in the record detail. Withdrawing hides them again. A migration adds `records.import_confirmed_at` (column-add only; existing imports are backfilled as confirmed so existing libraries keep their MCP visibility). The UI states the gate everywhere: detail-page confirm/withdraw banner, 「待确认入记忆」list badge, and both import dialogs now say the content is invisible to your AI assistant until confirmed.
- **MCP: list_confirmed_minutes（块5）.** A new read-only tool (#8) returns the user's explicitly confirmed minutes conclusions/action items within an optional time range / knowledge base, newest record first, each with full provenance (record title, evidence window, verbatim quote, confirmed time). Provisional and archived content is never returned; date-only bounds are inclusive-of-day.
- **Minutes quality healthcheck（块6）.** `ECHO_MINUTES_HEALTHCHECK=1 cargo test --test minutes_healthcheck_test` runs the post-hoc minutes pipeline over fixed fixtures (短会 / 术语密集 / 长稿) and writes a comparable report (latest.json + latest.md + history snapshots) with per-fixture topic/conclusion/action counts, evidence hit rate, dropped count and timing — plus an automatic Δ table against the previous run. It skips by default (never breaks the green suite) and uses fixtures only, never real content. This is the pre-swap baseline for the "should we move off qwen2.5:7b" decision: today's numbers are 议题 2/3/3, 结论 0/0/2, 待办 0/1/4, dropped all 0.
- **Export package（块7）.** The record detail 「操作」 menu gains 导出包: one folder with 逐字稿.md + 逐字稿.srt + 纪要.md (rendered from the same content_json as the panel — 所见即所导) + manifest.json (record metadata incl. dual-recording alignment offset, minutes status/model, **honest confirmed/provisional distribution** — nothing silently flips to confirmed in the export). Behind the same one-time ticket gate as other exports.

## 中文重点（给先看中文的你）

这次先「验证」后「拓展」：12 个场景把 v1.0+v1.1 全能力走了一遍（报告见 docs/usability-audit-v1.2.md），真二进制黑盒探针证明隐私红线完好（未确认内容全出口 0 泄漏），5 个 P1 全部修复并带回归测试。然后四项有边界拓展：①导入内容（文档/AI 对话）也过确认门——确认前你的 AI 助手完全看不到，确认后才可见、可撤回（存量导入自动视为已确认，行为不变）；②MCP 新工具 `list_confirmed_minutes`：按时间范围取已确认纪要摘要，条条带溯源；③纪要质量体检：一条命令对固定基准连跑事后纪要并出可对比报告，给「要不要换更强模型」提供换前基线（现状：结论提取偏稀，dropped 全 0 无编造）；④单条记录导出包：逐字稿+纪要+清单一个文件夹全量交付，未确认条目在导出里不悄悄变已确认。

## Known limitations

- 终审修2 之后，未确认的 document 导入不进记忆链（分析照常跑、演化条目暂不派生），确认后才补进——这是确认门的正确语义，但意味着「导入后不确认」的文档在认知演化里暂不可见。
- 导出包对 document/chat 记录的逐字稿时间轴是导入时生成的伪时间轴（每 800 字块=1 秒），SRT 对这两类记录无字幕意义（与 v1.1 相同的既有口径，导出包未改变它）。
- 纪要质量仍取决于所选本机模型（qwen2.5:7b 结论提取偏稀，见块6 体检基线）。
- 导出包选择文件夹而非 zip：仓库的 zip 依赖只用于读 docx，没有现成 zip 导出路径，文件夹零新增依赖面且可直接打开。

## Verification status

- Backend: `cargo test`（含 mcp-bin）**315 过 / 0 挂 / 20 ignored / 19+4 测试目标**（v1.1 基线 306 → 本 Loop 净增 9 个测试 + 4 个新测试文件）；前端 `vitest` **140 过**（133 → +7）；`tsc` / vite build / `cargo build` 全过、编译警告 0。
- 黑盒探针（真二进制 + 隔离库）：导入确认门全出口 0 泄漏（含终审补测的独立 list_action_items 与派生待办）；未确认纪要条目 11 出口 0 泄漏（块1）；list_confirmed_minutes 三态数据（确认/暂定/归档）范围与库过滤逐项断言。
- 量化实测（docs/usability-audit-v1.2.md「量化实测」节）：实时文字延迟 3043ms（与 v1.1 的 3.2s 同口径持平）；事后纪要 01-short 10.4s / 02-medium 29.2s / 06-long 59.5s（qwen2.5:7b）；500 条隔离库 list 5ms / search 8ms / MCP 1ms 无悬崖；DMG 14MB 与 v1.1 同级。
- 独立终审（四视角攻击）：初审判定打回——抓出并当场修复 1 个 P0（独立工具 list_action_items 未过导入确认门，实测复现泄漏后修复+补测）、2 个 P1（记忆派生联动、until 边界）；复审全绿后放行。
- 每块构建核查 + 测试 + 对抗审核三关全过；测试总数只涨不跌（306→315，133→140）。

## Not verified (needs a human)

- **TCC 弹窗 / 真机 UI 走查**：通宵环境无法驱动真机 UI——所有 UI 层结论均为代码层+测试层验证，真机点击路径（录音按钮、确认横幅、导出包流程）需要人手验。
- **发布前产物审计**：DMG 未签名未公证（本地构建、已验挂载与内容齐全）；发布需按仓库惯例走签名公证流程（APPLE_SIGNING_IDENTITY + NOTARYTOOL_KEYCHAIN_PROFILE）。
- **签名公证、Release 转正、PR 合入**：全部留给人定——本分支叠加在未合入的 PR #1 之上，是否先合 #1 再合本 PR、draft release 何时建，由人决定。
- **真机说话人分离**（需 HF token）与 **真实会议体验**：沿 v1.1 未验证项不变。
