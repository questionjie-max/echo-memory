# Echo Memory v0.4.1 Hotfix

中文说明见后半部分。

## Fixes

- **UI freeze / force-quit fixed**: AI dock chats, template generation, knowledge Q&A, and AI-status checks ran network calls on the main thread, freezing the interface until models replied (or forever). All long calls now run on background threads; the UI stays responsive.
- Dock chat quality: conversations now use Ollama's role-based /api/chat with mode-tuned temperatures and sharper prompts (conclusion-first, concrete answers, actionable next step).
- The "保存为文档" button on dock replies is now hover-revealed with an explanatory tooltip, instead of appearing under every message.

## 验证

106 项 Rust 测试 + 真实环境端到端（真实语音转写→分析→校对→Dock→模板→产出）全部通过。

---

# 回声记忆 v0.4.1 热修复

## 修复

- **界面卡死/被迫强制退出已修复**：AI 伙伴对话、模板生成、知识库问答、AI 状态检查此前在主线程执行网络请求，模型生成期间整个界面冻结。所有长调用已移至后台线程，界面持续可交互。
- AI 伙伴对话质量：改用 Ollama 角色化 /api/chat 接口，按模式调温，提示词更明确（先结论、答具体、给下一步）。
- 「保存为文档」按钮改为悬停显示并附带说明，不再出现在每条回复下方造成困扰。
