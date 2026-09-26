# macOS 签名与公证

## 方案决策

项目采用 App Store 之外的正式分发路径：

- 使用 `Developer ID Application` 证书签名应用和 DMG。
- 必须完成 Apple 公证并装订票据，不把 ad-hoc 签名当作正式发布。
- 不扩大为全盘文件权限，不使用 `--no-sign` 或 `--skip-stapling` 生成正式产物。
- 凭据不完整时，发布脚本在构建前明确失败，不产生“构建成功但不可分发”的假状态。
- 真实 secrets 就绪前，CI 只保留确定性质量门禁，不增加始终失败的签名任务。

Tauri 的 `bundle.macOS.hardenedRuntime` 默认开启。本项目通过环境变量选择签名身份和公证凭据，不要求把证书或私钥写入仓库。

## 当前状态

截至 2026-09-25，本机尚未具备正式签名发布条件：

```text
security find-identity -v -p codesigning
0 valid identities found

xcrun notarytool history
Error: Must provide credentials.
```

因此当前没有声称已完成 Developer ID 签名或公证，也没有向正式渠道发布签名 DMG。

## 前置条件

1. 加入付费 Apple Developer Program；免费账号不能完成公证。
2. 创建 `Developer ID Application` 证书，并将私钥导入发布机钥匙串。
3. 从钥匙串或 `security find-identity -v -p codesigning` 获取精确身份名称。
4. 配置一组公证凭据。以下两组任选其一：

```bash
# Apple ID 和 App 专用密码
export APPLE_ID="developer@example.com"
export APPLE_PASSWORD="app-specific-password"
export APPLE_TEAM_ID="TEAMID1234"
```

```bash
# App Store Connect API Key
export APPLE_API_ISSUER="issuer-id"
export APPLE_API_KEY="key-id"
export APPLE_API_KEY_PATH="/absolute/path/AuthKey_KEYID.p8"
```

5. 设置签名身份：

```bash
export APPLE_SIGNING_IDENTITY="Developer ID Application: Example (TEAMID1234)"
```

## 安全要求

- 不提交 `.p12`、`.p8`、App 专用密码、API Issuer 或 Team Secret。
- `APPLE_API_KEY_PATH` 指向的 `.p8` 权限应收紧为当前用户可读。
- `APPLE_PASSWORD` 必须是 App 专用密码，不是 Apple ID 登录密码。
- CI 接入时使用托管 secrets，并按最小权限配置；凭据就绪前不伪造成功任务。

## 执行与检核

先做预检：

```bash
cd app
npm run check:signing
```

预检会一次列出所有缺失项。只有返回 `0` 才应继续。

构建并验证正式 DMG：

```bash
cd app
npm run release:macos
```

该命令依次执行：

1. 签名与公证前置检查。
2. `tauri build --bundles dmg --ci`，正常等待公证和装订。
3. 对 DMG 执行 `hdiutil verify`、`codesign`、`spctl` 和 `stapler validate`。
4. 只读挂载 DMG，对内部 `.app` 再执行 `codesign`、`spctl` 和 `stapler validate`。
5. 输出 DMG 绝对路径与 SHA-256。

任一步骤失败都会返回非零退出码。正式验收还应在干净 macOS 机器上安装并启动一次，确认 Gatekeeper 不显示未验证阻断。

## 预期的当前结果

在当前未配置凭据的机器上运行 `npm run check:signing`，应返回 `1`，并至少明确报告：

```text
APPLE_SIGNING_IDENTITY
Notarization credentials (...)
```

这是预期结果，不应用 `--no-sign` 绕过。
