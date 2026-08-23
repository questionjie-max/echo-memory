# Echo Memory v0.4.2 Hotfix

## Fix

- Settings panel data never loaded: opening Settings showed perpetual "正在读取…" placeholders for local AI, external AI, and inbox because the panel's load effect was accidentally removed in v0.3.0 (it only ran on model-download events), and tab-specific loaders never fired when switching tabs. Opening the panel and every tab switch now load their data.

## 验证

后端各项加载经真实资料库探针实测均在毫秒级；typecheck / build / 106 项 Rust 测试全绿。

---

# 回声记忆 v0.4.2 热修复

## 修复

- 设置面板数据不加载：打开设置后「本机 AI / 外部 AI / 收件箱」永远显示"正在读取…"。原因是 v0.3.0 误删了面板打开时的加载逻辑（此前仅模型下载事件会触发刷新），且切换 tab 不会触发对应数据加载。现在打开设置与每次切换 tab 都会正确加载。

## 验证

后端各项加载经真实资料库探针实测均在毫秒级；typecheck / 构建 / 106 项 Rust 测试全部通过。
