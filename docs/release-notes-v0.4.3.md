# Echo Memory v0.4.3 Hotfix

## Fixes

- AI template wizard dialog was transparent: the panel relied on an undefined `.material` class in normal display mode and had no background of its own. It now has an explicit frosted background, border, and shadow.
- Template wizard redesigned as a true conversation: describe your scenario → the model proposes a template → reply "满意" to add it to the library, or keep describing adjustments and the model regenerates the full template from the conversation (name remains editable before saving).

## 验证

typecheck / build / 106 项 Rust 测试 / 真实 Ollama 模板生成测试全部通过。

---

# 回声记忆 v0.4.3 热修复

## 修复

- AI 模板向导弹窗透明：面板此前依赖正常显示模式下未定义的 `.material` 类且自身没有背景色，现已显式设置毛玻璃背景、边框与阴影。
- 模板向导重构为真正的对话流：描述使用场景 → 模型给出模板 → 点「满意，添加到模板库」入库；或不满意继续用自然语言说怎么改（增删栏目、改侧重），模型基于完整对话重新生成整版模板。入库前可修改模板名。
