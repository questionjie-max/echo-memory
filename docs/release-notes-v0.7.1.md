# Echo Memory v0.7.1

## Highlights

- **Knowledge-base chat alignment fixed**: user bubbles lock to a single grid row (a sparse auto-placement regression had pushed the text into a second row), the sender label reads 我 (me) instead of 你, and the earlier one-character-per-line vertical layout is gone.
- **Assistant dock can collapse again**: the dock no longer pins a fixed grid row template that misaligned with conditionally rendered children and pushed the collapse button out of reach after chatting. A persistent context hint (current record for 总结, "not searching the library" for 随手问) now clarifies the division of labor with the 问知识库 view.
- **Cognitive evolution is narrative**: change types are normalized to Chinese (added→新增, overturned→推翻, …) with unknown values falling back to 变化; added/supplemented changes no longer render an empty "之前" column; reasons that merely restate the viewpoint or a source quote are hidden; the confidence percentage number is gone (the 待确认推断 badge carries that signal).
- **One-glance snapshot summary and bulk confirmation**: the top line reads 基于 N 条记录 · M 个主题 · K 次变化 · 其中 P 条待确认, with a 全部确认 button that writes all pending confirmations in a single SQLite transaction — a validation failure rolls the whole batch back, never leaving half-confirmed state.
- **Explanatory reasons for new snapshots**: the generation prompt now requires reasons that explain why a view changed (Simplified Chinese, first person, no survey phrasing like 受访者); items without a genuine explanation may leave the field empty. Existing snapshots benefit immediately from the frontend redundancy filter.
- **Snapshot warnings are dismissible**: quality hints and generation errors in the memory views have a close button; a dismissed message stays hidden for that snapshot and reappears when content changes or a new snapshot arrives.

## Verification status

- Frontend type checking, 105 UI tests (including 8 new evolution-view cases), 4 change-type logic tests, and the Rust suite (38 tests plus a new atomic batch-feedback transaction test) all pass.
- `cargo clippy --all-targets -- -D warnings` reports no warnings.
- The three evolution card shapes (added without a before column, revised with before/after, redundant reason hidden) were verified against the real component in both light and dark themes.

## Distribution status

This source release is version `0.7.1`. The public artifact is `Echo-Memory_0.7.1_aarch64.dmg`, SHA-256 `d1da3ef1b06899173bd0df690cdf854fdaa6c6d9a10efdac328ada3e61116ec7`.

- `codesign --verify --deep --strict` passes for the application inside the DMG.
- `hdiutil verify` confirms the disk image is intact.
- `spctl --assess --type execute` reports `accepted` with `source=Notarized Developer ID` after notarization and stapling.
- This DMG is an **Apple-signed and notarized Developer ID release**. Notarization submission `29d62a69-b00c-4429-bee8-af977feccd14` completed with status `Accepted`.

Users can download the DMG, open it, and move `回声记忆.app` into `/Applications` normally.

---

# 回声记忆 v0.7.1

## 本版重点

- **问知识库聊天对齐修复**：用户气泡显式锁定网格行（此前稀疏自动放置把正文挤到第二行），发送人改为「我」，一字一行竖排问题一并消失。
- **随行助手可以收回了**：去掉写死的网格行模板（条件渲染子元素错位曾把收起按钮顶出屏幕）；停靠栏新增常驻上下文提示（总结模式显示当前记录、随手问显示「不检索资料库」），与「问知识库」的分工一目了然。
- **认知演化改造成叙事**：变化类型归一化为中文（added→新增、overturned→推翻……未知值兜底「变化」）；新增/补充类不再渲染空壳「之前」栏；复述观点或证据的原因自动隐藏；删除置信度百分比数字（「待确认推断」徽章承担该信号）。
- **一眼看懂的快照摘要 + 批量确认**：顶部一行「基于 N 条记录 · M 个主题 · K 次变化 · 其中 P 条待确认」，配「全部确认」按钮——单条 SQLite 事务写入，任一条目校验失败整体回滚，不留半确认状态。
- **新生成快照的原因更解释性**：生成提示词要求说明「为什么变」（简体中文、第一人称、禁止「受访者」等问卷口吻）；真正无可解释时允许留空。旧快照立即受益于前端的复述过滤。
- **快照警告可以关闭**：记忆视图的质量提示与生成异常有了关闭按钮；同一条在同快照内不再出现，内容变化或新快照时重新提示。

## 验证状态

- 前端类型检查、105 个 UI 测试（含 8 个新增认知演化用例）、4 个变化类型纯逻辑测试、Rust 全量测试（38 例 + 新增批量反馈事务测试）全部通过。
- `cargo clippy --all-targets -- -D warnings` 无警告。
- 认知演化三种卡片形态（新增无之前栏 / 修正带前后对比 / 复述原因隐藏）在明暗两主题下对照真实组件逐一核验。

## 分发状态

本源码版本为 `0.7.1`，公开安装包为 `Echo-Memory_0.7.1_aarch64.dmg`，SHA-256 为 `d1da3ef1b06899173bd0df690cdf854fdaa6c6d9a10efdac328ada3e61116ec7`。

- DMG 内应用程序通过 `codesign --verify --deep --strict`。
- `hdiutil verify` 确认磁盘镜像完整。
- 公证与装订完成后 `spctl --assess --type execute` 返回 `accepted`，来源为 `Notarized Developer ID`。
- 因此它明确标注为 **Apple 已签名并完成公证的 Developer ID 发布版**。公证提交 `29d62a69-b00c-4429-bee8-af977feccd14` 状态为 `Accepted`。

macOS 用户下载 DMG 后可正常打开，并把 `回声记忆.app` 拖入 `/Applications`。
