# Echo Memory v0.5.1 Hotfix

## Fix

- **Long recordings always failed analysis**: a real 9-minute meeting (172 segments) failed 100% of the time, not intermittently. `num_predict` was capped at 2048, which cannot fit the analysis JSON once every list entry carries a verbatim `quote_text` — the output was cut mid-string at ~4,000 characters and became invalid JSON. The correction retry then fed that truncated text back to the model, which induced it to produce an equally over-long result, so both attempts failed the same way.
  - `num_predict` raised from 2048 to 6144, enough for a real recording's citation volume.
  - When the first output cannot be parsed at all, the retry no longer echoes it back. It asks for a deliberately compact result instead (fewer entries, one short quote each), which is the only instruction that can break the truncation loop.

## Test coverage added

- Two unit tests pin the retry prompt in both directions: unparsable output is withheld and replaced with the compactness instruction, while parsable-but-low-quality output is still echoed back as correction evidence. Both were confirmed to fail when the fix is disabled, so they guard the behavior rather than passing vacuously.
- The chunked long-transcript path (`analyze_long`) now has end-to-end coverage for the first time. It previously had none, and it is the path most exposed to the token limit because the merge step's input grows with the chunk count.

## 验证

- 109 Rust tests (was 107), 9 frontend tests, 2 UI smoke tests, typecheck / build / `cargo fmt` all green.
- Real-model run on a 15,670-character transcript (4 chunks + merge) against local `qwen2.5:7b` on an M1 Pro: completed with no quality warning — 405-character summary, 5 key points, 2 decisions, 6 action items. Six model calls against five progress steps confirms the correction-retry branch executes in a real run, not only in unit tests.

## 已知边界

- The merge step concatenates every chunk's analysis into one prompt while `num_ctx` stays fixed at 16384, and Ollama truncates oversized input silently. This was verified working at 4 chunks; a recording long enough to produce many more may lose early content during the merge without any error. Deferred to the next version.
- This fix was written on 2026-08-01 and had been sitting unmerged on `fix/analysis-truncation`; it was not part of the v0.5.0 DMG.

---

# 回声记忆 v0.5.1 热修复

## 修复

- **长录音分析必然失败**：真实 9 分钟会议录音（172 个片段）分析 100% 失败，并非偶发。`num_predict` 上限为 2048，装不下带逐字引文的分析 JSON——每个列表条目都带 `quote_text`，输出在第 4000 字左右被截断在字符串中间，成为非法 JSON。随后纠错重试又把这段截断内容原样回灌给模型，诱导它再次产出同样超长的结果，两次尝试以相同方式失败。
  - `num_predict` 从 2048 提升到 6144，足以容纳真实录音的引用体量。
  - 首次输出完全无法解析时，重试不再回灌该内容，改为明确要求更紧凑的结果（条目更少、每条只留一句短引文）。这是唯一能打破截断循环的指令。

## 新增测试覆盖

- 两个单元测试锁定重试提示的两个方向：无法解析的输出被扣下并替换为紧凑输出要求；能解析但质量不达标的输出仍然回灌作为纠正依据。两者都验证过在关闭修复时会失败，因此它们拦的是真实行为，而不是无论如何都通过。
- 分块长逐字稿路径（`analyze_long`）首次有了端到端覆盖。此前它没有任何测试，而它恰恰最容易撞上 token 上限——合并步骤的输入会随分块数量增长。

## 验证

- Rust 测试 109 项（原 107）、前端 9 项、UI 冒烟 2 项，typecheck / 构建 / `cargo fmt` 全绿。
- 真实模型实测：15,670 字逐字稿（4 个分块 + 1 次合并），本机 `qwen2.5:7b`、M1 Pro，完成且无质量警告——摘要 405 字、关键观点 5 条、决策 2 条、待办 6 条。进度 5 步而模型调用 6 次，说明纠错重试分支在真实运行中确实被执行，而非只在单元测试里走过。

## 已知边界

- 合并步骤会把每个分块的分析结果拼进同一个提示，而 `num_ctx` 固定为 16384，Ollama 对超长输入是静默截断的。本次在 4 个分块下验证通过；录音长到产生更多分块时，合并阶段可能悄悄丢失开头的内容且不报错。此项推迟到下一版本处理。
- 该修复写于 2026-08-01，一直未合并地留在 `fix/analysis-truncation` 分支上，因此 v0.5.0 的 DMG 不包含它。
