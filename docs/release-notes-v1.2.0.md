# Echo Memory v1.2.0

> The verification-first release: a full usability audit of v1.0+v1.1 (12 scenarios, zero P0, five P1 findings all fixed), four bounded expansions — the import confirmation gate, a time-range MCP tool for confirmed minutes, a repeatable minutes-quality healthcheck, and a one-click export package — plus a real-device recording and release-hardening pass.

## Download

- `Echo-Memory_1.2.0_aarch64.dmg`（回声记忆 1.2.0）· macOS Apple Silicon
- SHA-256: `ccd8d17e1607a10bae13cb74903921f02442ef4dd5115d7dd938a19fdc891632`
- Developer ID signed, Apple-notarized, stapled, and accepted by Gatekeeper.

## Highlights

- **Usability audit + P1 fixes（块1-3）.** A 12-scenario walkthrough (docs/usability-audit-v1.2.md) verified the product end to end with a real-binary MCP black-box probe: **P0 = 0**（未确认纪要条目经 11 个出口 0 泄漏，确认门完好）。Five P1 findings fixed, one commit each with regression tests: MCP scope copy now states both layers honestly (record原文 is readable over MCP by design; memory entries are confirmed-only); chat-import copy no longer claims analysis that doesn't happen; agent-session scan no longer defaults to select-all; the onboarding re-run actually re-pops after closing settings; live-minutes no longer degrades silently when Ollama is missing (one-shot honest notice per session).
- **Import confirmation gate（块4，定位命门）.** Imported documents and AI-chat records now start provisional: `get_record` / `get_transcript` / `search_records` / `get_project_context` / `list_action_items` return **nothing** for them — including analysis-derived decisions and action items — until the user hits 确认入记忆 in the record detail. Withdrawing hides them again. A migration adds `records.import_confirmed_at` (column-add only; existing imports are backfilled as confirmed so existing libraries keep their MCP visibility). The UI states the gate everywhere: detail-page confirm/withdraw banner, 「待确认入记忆」list badge, and both import dialogs now say the content is invisible to your AI assistant until confirmed.
- **MCP: list_confirmed_minutes（块5）.** A new read-only tool (#8) returns the user's explicitly confirmed minutes conclusions/action items within an optional time range / knowledge base, newest record first, each with full provenance (record title, evidence window, verbatim quote, confirmed time). Provisional and archived content is never returned; date-only bounds are inclusive-of-day.
- **Minutes quality healthcheck（块6）.** `ECHO_MINUTES_HEALTHCHECK=1 cargo test --test minutes_healthcheck_test` runs the post-hoc minutes pipeline over fixed fixtures (短会 / 术语密集 / 长稿) and writes a comparable report (latest.json + latest.md + history snapshots) with per-fixture topic/conclusion/action counts, evidence hit rate, dropped count and timing — plus an automatic Δ table against the previous run. It skips by default (never breaks the green suite) and uses fixtures only, never real content. This is the pre-swap baseline for the "should we move off qwen2.5:7b" decision: today's numbers are 议题 2/3/3, 结论 0/0/2, 待办 0/1/4, dropped all 0.
- **Export package（块7）.** The record detail 「操作」 menu gains 导出包: one folder with 逐字稿.md + 逐字稿.srt + 纪要.md (rendered from the same content_json as the panel — 所见即所导) + manifest.json (record metadata incl. dual-recording alignment offset, minutes status/model, **honest confirmed/provisional distribution** — nothing silently flips to confirmed in the export). Behind the same one-time ticket gate as other exports.
- **Recording source pickers（实机反馈修复）.** The local recording card now enumerates available microphones and running apps: record the microphone, system audio from one selected app or all apps, or combine the microphone and selected app into one dual-track recording. The privacy confirmation names the selected app, and the microphone bridge now uses `AVAudioEngine` + Core Audio device selection rather than a fixed default input.
- **Daily-use cleanup（实机反馈修复）.** Speaker and related-record sections on the record detail are collapsed by default and expand on demand, so they no longer occupy the bottom of the page. Clicking 首页 now always returns to the knowledge home (including the local recording card) instead of leaving the previous record detail open.
- **Release hardening.** The signed app now carries the Hardened Runtime entitlement `com.apple.security.device.audio-input`; without it macOS rejects microphone access before TCC can grant it. The bundle audit fails any release missing this entitlement or `NSMicrophoneUsageDescription`. The final DMG was signed, notarized, stapled, and accepted by Gatekeeper.

## 中文重点（给先看中文的你）

这次先「验证」后「拓展」：12 个场景把 v1.0+v1.1 全能力走了一遍（报告见 docs/usability-audit-v1.2.md），真二进制黑盒探针证明隐私红线完好（未确认内容全出口 0 泄漏），5 个 P1 全部修复并带回归测试。然后四项有边界拓展：①导入内容（文档/AI 对话）也过确认门——确认前你的 AI 助手完全看不到，确认后才可见、可撤回（存量导入自动视为已确认，行为不变）；②MCP 新工具 `list_confirmed_minutes`：按时间范围取已确认纪要摘要，条条带溯源；③纪要质量体检：一条命令对固定基准连跑事后纪要并出可对比报告，给「要不要换更强模型」提供换前基线（现状：结论提取偏稀，dropped 全 0 无编造）；④单条记录导出包：逐字稿+纪要+清单一个文件夹全量交付，未确认条目在导出里不悄悄变已确认。

实机试用后补了一轮收尾：录音前可以选择麦克风和系统声音来源（全部系统声音或某个正在运行的应用），双路录音会在确认框里明确写出所选应用；记录详情的「说话人 / 相关记录」改为默认收起，点首页会回到知识首页；正式包补上了 Hardened Runtime 的麦克风 entitlement，并把这项加入发布审计，避免出现系统设置已放行、应用却拿不到麦克风的情况。最终 DMG 已完成签名、公证、装订并通过 Gatekeeper 检查。

## Known limitations

- 终审修2 之后，未确认的 document 导入不进记忆链（分析照常跑、演化条目暂不派生），确认后才补进——这是确认门的正确语义，但意味着「导入后不确认」的文档在认知演化里暂不可见。
- 导出包对 document/chat 记录的逐字稿时间轴是导入时生成的伪时间轴（每 800 字块=1 秒），SRT 对这两类记录无字幕意义（与 v1.1 相同的既有口径，导出包未改变它）。
- 纪要质量仍取决于所选本机模型（qwen2.5:7b 结论提取偏稀，见块6 体检基线）。
- 导出包选择文件夹而非 zip：仓库的 zip 依赖只用于读 docx，没有现成 zip 导出路径，文件夹零新增依赖面且可直接打开。

## Verification status

- Backend: `cargo test`（含 mcp-bin）**315 过 / 0 挂 / 20 ignored**（v1.1 基线 306 → 本轮净增 9 个测试）；前端 `vitest` **144 过 / 21 个测试文件**（133 → +11）；`tsc` / vite build / `cargo build` 全过。
- 黑盒探针（真二进制 + 隔离库）：导入确认门全出口 0 泄漏（含终审补测的独立 list_action_items 与派生待办）；未确认纪要条目 11 出口 0 泄漏（块1）；list_confirmed_minutes 三态数据（确认/暂定/归档）范围与库过滤逐项断言。
- 量化实测（docs/usability-audit-v1.2.md「量化实测」节）：实时文字延迟 3043ms（与 v1.1 的 3.2s 同口径持平）；事后纪要 01-short 10.4s / 02-medium 29.2s / 06-long 59.5s（qwen2.5:7b）；500 条隔离库 list 5ms / search 8ms / MCP 1ms 无悬崖；DMG 14MB 与 v1.1 同级。
- 独立终审（四视角攻击）：初审判定打回——抓出并当场修复 1 个 P0（独立工具 list_action_items 未过导入确认门，实测复现泄漏后修复+补测）、2 个 P1（记忆派生联动、until 边界）；复审全绿后放行。
- 每块构建核查 + 测试 + 对抗审核三关全过；测试总数只涨不跌（306→315，133→144）。
- 正式产物：`codesign --verify --deep --strict` 通过；Apple 公证 `Accepted`；DMG 已装订并通过 `spctl`（`Notarized Developer ID`）；安装后的 app 含 `com.apple.security.device.audio-input`。

## Not verified (needs a human)

- **TCC 弹窗 / 真机 UI 走查**：正式包已具备正确的麦克风 entitlement，`tccutil reset` 后系统授权弹窗可正常出现；macOS 会保护授权弹窗，自动化不能替用户点击，因此「允许后实际录到人声、电平变化、停止入库」仍需要人手完成最后一跳。
- **干净机器安装体验**：本机已完成签名、公证与 Gatekeeper 验证；干净 macOS 首次安装和跨设备权限体验仍建议在另一台机器复核。
- **真机说话人分离**（需 HF token）与 **真实会议体验**：沿 v1.1 未验证项不变。
