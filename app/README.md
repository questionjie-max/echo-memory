# 回声记忆

macOS 本地优先的谈话记忆库。音频、逐字稿、SQLite 数据库和 MCP 均在本机运行；应用不会将内容发送到云端，也不会静默降级到云端模型。

## 当前能力

- 导入 MP3、M4A、WAV 到 `~/Library/Application Support/回声记忆/library/audio/`，以 SHA-256 检测重复文件。
- 三栏工作流：知识库、待处理/最近工作区、逐字稿/文稿分析详情。
- 知识库、记录、转写任务和可重复 SQLite migration；录音归档会事务性同步搜索和待办归属。
- 内嵌 Whisper（macOS Metal）或 `whisper.cpp` 本机命令适配、片段存储、音频播放、片段跳转和不覆盖原文的逐字稿编辑。
- Ollama 本机适配、16K 长文上下文、结构化 JSON 质量重试、重复观点过滤和连续/多处引用校验。
- SQLite FTS5 搜索标题、最新逐字稿与分析；结果包含知识库、日期、说话人和原文定位。
- 默认关闭的只读 stdio MCP；可在应用左侧启停、查看授权范围和最近调用，`get_project_context` 返回真实决策与引用。

## 依赖

- Node.js 18+
- Rust stable、Cargo 和 Xcode Command Line Tools
- CMake（首次从源码构建内嵌 Whisper 依赖时需要；仅构建期，不是应用运行时依赖）
- macOS WebView（系统自带）
- macOS 离线转写：设置 `WHISPER_MODEL_PATH` 指向本机 `ggml-*.bin` 模型；若已安装 Meetily，默认复用 `~/Library/Application Support/com.meetily.ai/models/ggml-small.bin`。
- 可选：`whisper-cli`（或设置 `WHISPER_CPP_BIN`）优先于内嵌模型，用于指定自己的 `whisper.cpp` 安装。
- 可选：Ollama 本机服务和已安装模型，用于本地分析

## 运行与验证

```bash
cd app
npm install
bash scripts/check-env.sh
npm run tauri dev

npm run typecheck
npm run build
cd src-tauri && cargo fmt --check && cargo test --features mcp-bin
```

资料库根目录默认是 `~/Library/Application Support/回声记忆`；开发和测试可用 `ECHO_LIBRARY_ROOT` 覆盖。

## MCP

MCP 服务器是一个独立、只读的 stdio 程序，只有用户在 MCP 客户端配置它时才会启动：

```json
{
  "mcpServers": {
    "echo-memory": {
      "command": "/absolute/path/to/app/src-tauri/target/debug/echo-memory-mcp"
    }
  }
}
```

开发期先运行 `cd app/src-tauri && cargo build --bin echo-memory-mcp --features mcp-bin`。该服务器不监听端口；每次工具调用仅记录工具名、记录 ID、项目 ID 和时间，不记录音频、逐字稿或密钥。

真实长录音分析回归需要本机 Ollama，并由环境变量显式指定现有资料库与记录：

```bash
ECHO_LIBRARY_ROOT="$HOME/Library/Application Support/回声记忆" \
ECHO_TEST_RECORD_ID="记录 UUID" \
cargo test --test analysis_e2e_test analyzes_long_record_with_key_points_and_verified_citations -- --ignored
```

## 已知限制

- 真实离线转写需要本机可读的 Whisper `ggml-*.bin` 模型；缺失时会显示错误并保留可重试任务。应用不会下载模型或上传音频。
- Ollama 分析需要本机已启动服务和本地模型；失败不会影响逐字稿。
- Alpha 尚未完成 10 段真实录音和断网抓包回归；当前 DMG 也未做 Apple Developer ID 签名与公证。
