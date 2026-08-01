# 三人并行开发约定

黑客松期间三条分支并行。**开工前先读完这份,能省掉最后一小时的合并地狱。**

## 分支与分工

| 分支 | 负责人 | 做什么 | 对应会议纪要 |
|---|---|---|---|
| `feat/text-note` | A | 软件内文字记录界面（可新建文本，与语音并列） | 第 1 件 |
| `feat/templates` | B | 模板系统（减肥 / 记账 / 周报） | 说话人 2 提议 |
| `feat/packaging` | C | 打包分发 + 首次启动引导 | 第 6 件 |

三条分支都从同一个 `main` 起点切，互不嵌套。

```bash
git clone https://github.com/questionjie-max/echo-memory.git
cd echo-memory
git switch feat/xxx      # 换成自己的分支
```

## 三条硬纪律

### 1. migration 编号已分死，不许抢

现有 migration 到 `0011_`。往后：

| 人 | 可用编号 |
|---|---|
| A | `0012_` |
| B | `0013_` |
| C | `0014_` |

**用不到也别让给别人。** 两人同时建 `0012_` 是本项目最危险的冲突——Git 会认为是两个不同文件、不报冲突、直接都合进来，然后数据库迁移顺序就乱了，而且测试不一定报错，可能到演示时才发现。

### 2. 样式各写各的文件，不要都改 styles.css

`src/styles.css` 是 508 行单文件，三人同时往里塞样式必炸。

各自新建：

```
src/styles/text-note.css      ← A
src/styles/templates.css      ← B
src/styles/onboarding.css     ← C
```

然后在 `src/styles.css` **最末尾**各加一行：

```css
@import "./styles/text-note.css";
```

三人各加一行、都在文件尾部，Git 基本能自动合。

### 3. 每 2 小时往下同步一次 main

**冲突的痛苦程度跟憋多久成正比。** 每 2 小时合一次，每次解 1-2 处小冲突；憋 6 小时最后一起合，可能要解 20 处，还都是自己已经忘了细节的代码。

```bash
git switch main && git pull
git switch feat/自己的分支
git merge main
```

用 `merge` 不用 `rebase`——已推送的分支 rebase 会逼队友强推。

## 会冲突的地方（心里有数就不慌）

这四个文件三人都要改，是"汇流点"：

| 文件 | 冲突类型 | 怎么解 |
|---|---|---|
| `src-tauri/src/lib.rs` | 都在末尾加命令注册 | **良性**，两边都保留 |
| `src/lib/tauri.ts` | 都在末尾加调用封装 | **良性**，两边都保留 |
| `src/shared/types.ts` | 都在末尾加类型 | **良性**，两边都保留 |
| `src/styles.css` | 见纪律 2，已规避 | 只有 @import 那行 |

**往末尾加，不要插中间。** `types.ts` 里各人用注释分区，视觉隔离：

```ts
// === A: 文字记录 ===
// === B: 模板 ===
```

## 冲突了怎么办

不用慌。**冲突不会弄坏任何东西，也不会丢代码。**

```bash
git merge main
# Git 说: CONFLICT in src/xxx
git status              # 看哪些文件冲突
```

VS Code 打开冲突文件，高亮处上方有按钮：「采用当前更改」「采用传入的更改」「保留双方」。
**大多数情况点「保留双方」就对了**（通常是两人各加了新东西）。

改完：

```bash
git add 那个文件
git commit
```

**想反悔，任何时候都能退回去：**

```bash
git merge --abort       # 完全回到合并前，啥也没变
```

记住这条，就不用怕试错。

## 合并顺序：C → B → A

1. **C**（打包）几乎不碰业务代码，先合掉
2. **B**（模板）冲突面小
3. **A**（文字记录）最重，最后合

**为什么 A 最后**：让改动最大的人去解冲突，因为他最清楚自己改了什么。反过来 C 最后合，C 的人要去解一堆自己看不懂的业务冲突。

## 每次合并前必须过验证

```bash
cd app
export PATH="$HOME/.cargo/bin:$PATH"
npm run typecheck
cd src-tauri
cargo fmt --check
cargo test --features mcp-bin
```

**基线是 58 passed / 0 failed。退化了不许合。**

CI（`.github/workflows/ci.yml`）会自动跑这三条，PR 里能看到结果。

## 本地环境

已在开发机装好：Rust stable、CMake、Ollama + `qwen2.5:7b` + `qwen3-embedding:0.6b`、Whisper `large-v3-turbo-q5_0`。

启动：

```bash
cd app
export PATH="$HOME/.cargo/bin:$PATH"
export WHISPER_MODEL_PATH="$HOME/Library/Application Support/回声记忆/models/ggml-large-v3-turbo-q5_0.bin"
npm run tauri dev
```

分析功能需要 `ollama serve` 在后台运行。

## 已知问题（与三条分支无关，但影响演示）

`src/analysis.rs` 的 `num_predict = 2048` 装不下带逐字引文的 JSON。**实测 9 分钟真实录音分析 100% 失败**：输出在第 4042 字被截断成非法 JSON，且重试会把截断内容回灌给模型，导致两次尝试同样失败。

这个 bug 会让演示走不到"分析"那一步。建议单独开 `fix/analysis-truncation` 优先合掉，改动约 10 行。
