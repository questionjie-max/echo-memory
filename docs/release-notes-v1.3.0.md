# Echo Memory v1.3.0

> The third-party connection release: a unified provider catalog across transcription and analysis, live model listing with capability grouping, test-before-you-save draft diagnosis with a persistent verification state machine, and honest failure classification — plus two real-device crash fixes (recording abort, silently-empty keychain saves).

## Download

- `Echo-Memory_1.3.0_aarch64.dmg`（回声记忆 1.3.0）· macOS Apple Silicon
- SHA-256: `ee74c9fe0085a31db13f46734a2973a974d882d04dc9bd0a7c42ebf7b10a5776`
- Developer ID signed, Apple-notarized, stapled, and accepted by Gatekeeper.

## Highlights

- **第三方服务商接入全面优化.** 转写与分析各自独立配置，服务商由内置目录统一管理：转写已支持**智谱 AI、阿里云百炼、OpenAI、腾讯云语音、豆包语音**五家；分析已支持**智谱 AI、阿里云百炼、OpenAI、DeepSeek、月之暗面 Kimi、MiniMax、硅基流动**七家，另提供自定义 OpenAI 兼容接口（自建服务/中转站，地址与模型 ID 自填）。内置服务商的接口地址、协议与推荐模型自动配置，不需要手填；腾讯云的多字段凭据（AppID + SecretId + SecretKey）与单 Key 服务商都支持。凭据保存在 macOS 钥匙串的分析/转写双槽位，同一服务商可跨卡复用，秘密不经过界面。
- **实时模型列表 + 能力分组.** 支持的服务商可一键拉取当前账号可用的模型列表：文本对话模型作为推荐候选，embedding / rerank / 语音转写 / 语音合成 / 视觉类模型收进「其他模型」不作为推荐；当前模型不在实时列表时明确提示，但**绝不偷偷换掉你的选择**。拉取只刷新候选，不会自动保存、不会改动隐私同意。
- **先测试，再保存.** 「保存并验证」先用你刚填写的凭据发一次真实的小请求（分析=一条固定测试消息；转写=一段约 1 秒的静音测试音），通过才落盘；失败时配置零污染，原样保留。测试用的新 Key 只走内存，不进钥匙串、不进数据库。已保存的配置也可随时「验证连通」。
- **验证状态机.** 每个槽位有明确六态：待配置 → 已配置未验证 → 正在验证 → 已验证 → 配置已变更需重新验证 → 验证失败。改服务商、改地址、改模型、改 Key 中的任何一项，旧验证立即失效；重启应用后验证结果与状态依然成立。卡片状态与表单状态同源同词，不会再出现「填齐了却显示可用」的误导。
- **失败说清楚坏在哪.** 统一错误分类：Key 无效或没权限、模型不存在或已下线、模型不支持该接口、频率限制或额度不足、网络不通、服务商返回异常、模型输出不满足结构化要求——每种都配「最可能的原因 + 建议动作」，技术详情可折叠。分不清是 Key 还是模型的问题时，同时给出「换模型」和「检查账号权限」两条路，不再把一切笼统报成「失败」。
- **两处实机崩溃修复.** ①点「开始录音」即闪退：切换麦克风设备后取错了音频格式导致原生异常，已改用输入节点的正确格式 API 并对原生调用加异常兜底，任何音频设备异常都降级为可读的错误提示而不是进程中止；②钥匙串「假保存」：修复在部分 macOS 版本上保存密钥被静默写空（界面显示成功、实际为空）的问题，改为系统安全框架原生写入并带写后读回校验，保存不成功会当场如实报错。

## 中文重点（给先看中文的你）

这版把第三方接入从头理顺了一遍：**转写支持智谱、百炼、OpenAI、腾讯云、豆包五家，分析支持智谱、百炼、OpenAI、DeepSeek、Kimi、MiniMax、硅基流动七家，外加自定义 OpenAI 兼容接口**。地址、协议、推荐模型全自动，Key 存钥匙串，两个槽位（转写/分析）互不干扰又能跨卡复用。

日常体验上三个最直接的变化：

1. **能拉模型了**：点「拉取可用模型」把服务商当前返回的模型拿回来，文本模型和其他模型分开显示，点哪个用哪个；拉取不自动保存任何东西。
2. **先测后存**：「保存并验证」先拿你填的 Key 发一次真实小请求，过了才保存；没过你的原配置一根头发都不动。已保存的配置也能随时重新验证。
3. **状态不骗人**：六种状态写在卡片和表单里，改任何配置旧验证立即失效，重启后依然作数——不会再出现「显示可用其实 Key 是错的」。

另外修了两个实机撞出来的崩溃：点录音闪退（切麦克风设备时音频格式取错）、钥匙串假保存（显示存了实际是空的）。都带回归测试。

## Known limitations

- 转写与分析都需要你自己的服务商账号与 Key；本地 Whisper 转写与本机 Ollama 分析仍然完全可用，不配第三方也能用。
- 长录音按各服务商的时长/大小限制自动切块，请求数量随录音长度增加；切块位置优先选静音处，边界文本可能重复由去重逻辑兜底。
- 自定义 OpenAI 兼容接口需要你自备服务（自建或中转站），地址只允许 HTTPS（本机地址除外）。
- 腾讯云与豆包没有统一的模型列表接口，使用内置推荐模型或手动填写模型 ID。

## Verification status

- 后端 `cargo test --features mcp-bin`：**381 passed / 21 ignored**（含 MCP 集成与真二进制黑盒探针；忽略项均需真实音频/模型/外部账号等环境）；前端 `vitest`：**165 passed / 22 个测试文件**；`tsc --noEmit`、vite build、`cargo build` 全部通过；`git diff --check` 干净。
- 正式 DMG 通过 `hdiutil verify`、`codesign --verify --deep --strict`、`spctl`（`Notarized Developer ID`）与 `xcrun stapler validate`；Apple 公证 DMG `1552ce99-fa79-4ebb-8b23-65d7169eeaf0`、app `11886640-0e11-4e28-b9c1-448c83e52ccc` 均为 `Accepted`。
- 新增覆盖：八类失败分类各自独立断言、草稿诊断零落盘（数据库快照比对）、一次性验证票据的生命周期（过期/复用/字段变化/跨 target）、健康快照与凭据修订号的指纹一致性、钥匙串「原生写入 → 命令行读回」闭环、录音引擎格式与异常兜底、前端六态推导优先级链、错误单一出口与可访问性语义。
