//! AI 对话记录导入：把 ChatGPT / Claude 导出与对话式 Markdown 落库为
//! 带说话人标签的记录，进入与分析、记忆抽取、知识检索共用的管线。
//! 解析全程本地进行；外部文件内容只作为数据读取，绝不执行其中任何指令。

use crate::error::{AppError, AppResult};
use crate::library::ManagedLibrary;
use crate::types::{ChatImportOutcome, ChatImportSkip, IngestResult, TranscriptSegmentInput};
use chrono::{Datelike, Utc};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// 单个导出文件的会话数上限：防呆，超出部分明确跳过而不是默默截断。
const MAX_CONVERSATIONS_PER_FILE: usize = 200;
const MAX_SOURCE_BYTES: u64 = 20 * 1024 * 1024;
const MAX_TEXT_CHARACTERS: usize = 2_000_000;
const TURN_CHAR_LIMIT: usize = 800;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatRole {
    User,
    Assistant,
}

impl ChatRole {
    fn label(self) -> &'static str {
        match self {
            ChatRole::User => "用户",
            ChatRole::Assistant => "助手",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ChatTurn {
    pub role: ChatRole,
    pub text: String,
}

#[derive(Debug, Clone)]
pub(crate) struct ParsedConversation {
    pub(crate) title: String,
    pub(crate) source_format: &'static str,
    pub(crate) turns: Vec<ChatTurn>,
    /// 对话的真实时间（RFC3339）：来自导出元数据；没有时落库回退文件 mtime。
    pub(crate) occurred_at: Option<String>,
}

/// 批量导入：逐文件、逐会话解析；失败以 skipped 明细返回，不让一个坏会话拖垮整批。
pub fn import_chat_exports(
    library: &ManagedLibrary,
    sources: &[PathBuf],
    project_id: Option<&str>,
    duplicate_confirmed: bool,
) -> AppResult<ChatImportOutcome> {
    let mut outcome = ChatImportOutcome {
        imported: Vec::new(),
        skipped: Vec::new(),
    };
    for source in sources {
        import_file(library, source, project_id, duplicate_confirmed, &mut outcome);
    }
    Ok(outcome)
}

fn import_file(
    library: &ManagedLibrary,
    source: &Path,
    project_id: Option<&str>,
    duplicate_confirmed: bool,
    outcome: &mut ChatImportOutcome,
) {
    let file_label = source.display().to_string();
    if let Err(reason) =
        import_file_inner(library, source, file_label.clone(), project_id, duplicate_confirmed, outcome)
    {
        outcome.skipped.push(ChatImportSkip {
            file: file_label,
            title: None,
            reason: reason.to_string(),
        });
    }
}

fn import_file_inner(
    library: &ManagedLibrary,
    source: &Path,
    file_label: String,
    project_id: Option<&str>,
    duplicate_confirmed: bool,
    outcome: &mut ChatImportOutcome,
) -> AppResult<()> {
    let extension = source
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    match extension.as_str() {
        "json" => import_json_export(library, source, file_label, project_id, duplicate_confirmed, outcome),
        "md" | "markdown" | "txt" => {
            import_dialogue_text(library, source, file_label, project_id, duplicate_confirmed, outcome)
        }
        _ => Err(AppError::Import(
            "仅支持 .json 导出文件或对话式 .md/.markdown/.txt 文本".into(),
        )),
    }
}

fn import_json_export(
    library: &ManagedLibrary,
    source: &Path,
    file_label: String,
    project_id: Option<&str>,
    duplicate_confirmed: bool,
    outcome: &mut ChatImportOutcome,
) -> AppResult<()> {
    let bytes = crate::document::read_limited(source, MAX_SOURCE_BYTES)?;
    let root: Value = serde_json::from_slice(&bytes)
        .map_err(|error| AppError::Import(format!("JSON 解析失败：{error}")))?;
    let conversations = parse_json_export(&root)?;
    if conversations.len() > MAX_CONVERSATIONS_PER_FILE {
        outcome.skipped.push(ChatImportSkip {
            file: file_label.clone(),
            title: None,
            reason: format!(
                "会话数超过 {MAX_CONVERSATIONS_PER_FILE}，仅导入前 {MAX_CONVERSATIONS_PER_FILE} 个"
            ),
        });
    }
    for conversation in conversations.into_iter().take(MAX_CONVERSATIONS_PER_FILE) {
        match conversation {
            Ok(conversation) => import_parsed_conversation(
                library,
                &file_label,
                project_id,
                duplicate_confirmed,
                conversation,
                outcome,
            ),
            Err(reason) => outcome.skipped.push(ChatImportSkip {
                file: file_label.clone(),
                title: None,
                reason,
            }),
        }
    }
    Ok(())
}

fn import_dialogue_text(
    library: &ManagedLibrary,
    source: &Path,
    file_label: String,
    project_id: Option<&str>,
    duplicate_confirmed: bool,
    outcome: &mut ChatImportOutcome,
) -> AppResult<()> {
    let bytes = crate::document::read_limited(source, MAX_SOURCE_BYTES)?;
    let text = crate::document::decode_text(bytes)?;
    let fallback_title = source
        .file_stem()
        .and_then(|name| name.to_str())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or("未命名对话");
    let conversation = parse_markdown_dialogue(&text, fallback_title)?;
    import_parsed_conversation(
        library,
        &file_label,
        project_id,
        duplicate_confirmed,
        conversation,
        outcome,
    );
    Ok(())
}

/// 顶层是数组（官方全量导出）或单个会话对象都接受，按字段形态识别格式。
fn parse_json_export(
    root: &Value,
) -> AppResult<Vec<Result<ParsedConversation, String>>> {
    const FORMAT_HINT: &str = "不是受支持的对话导出格式（需要 ChatGPT 或 Claude 导出的 conversations.json）";
    let items: Vec<&Value> = match root {
        Value::Array(items) => items.iter().collect(),
        Value::Object(object) => {
            return parse_single_json_conversation(object).map(|parsed| vec![parsed]);
        }
        _ => return Err(AppError::Import(FORMAT_HINT.into())),
    };
    let Some(first) = items.first() else {
        return Err(AppError::Import("导出文件里没有会话".into()));
    };
    if first.get("mapping").is_some() {
        Ok(items
            .iter()
            .map(|item| {
                parse_chatgpt_conversation(item).map_err(|error| error.to_string())
            })
            .collect())
    } else if first.get("chat_messages").is_some() {
        Ok(items
            .iter()
            .map(|item| {
                parse_claude_conversation(item).map_err(|error| error.to_string())
            })
            .collect())
    } else {
        Err(AppError::Import(FORMAT_HINT.into()))
    }
}

fn parse_single_json_conversation(
    object: &serde_json::Map<String, Value>,
) -> AppResult<Result<ParsedConversation, String>> {
    let value = Value::Object(object.clone());
    if object.contains_key("mapping") {
        Ok(parse_chatgpt_conversation(&value).map_err(|error| error.to_string()))
    } else if object.contains_key("chat_messages") {
        Ok(parse_claude_conversation(&value).map_err(|error| error.to_string()))
    } else {
        Err(AppError::Import(
            "不是受支持的对话导出格式（需要 ChatGPT 或 Claude 导出）".into(),
        ))
    }
}

fn parse_chatgpt_conversation(item: &Value) -> AppResult<ParsedConversation> {
    let title = json_title(item.get("title"));
    let mapping = item
        .get("mapping")
        .and_then(Value::as_object)
        .ok_or_else(|| AppError::Import("ChatGPT 会话缺少 mapping 字段".into()))?;
    let mut turns: Vec<ChatTurn> = Vec::new();
    match item.get("current_node").and_then(Value::as_str) {
        Some(current) => {
            // current_node 是当前分支的叶子：沿 parent 链回溯再反转，即当前可见对话。
            let mut cursor = Some(current.to_owned());
            while let Some(node_id) = cursor {
                let Some(node) = mapping.get(&node_id) else {
                    break;
                };
                if let Some(turn) = chatgpt_message_turn(node.get("message")) {
                    turns.push(turn);
                }
                cursor = node
                    .get("parent")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
            }
            turns.reverse();
        }
        None => {
            let Some(root) = mapping
                .values()
                .find(|node| node.get("parent").is_none_or(Value::is_null))
            else {
                return Ok(ParsedConversation {
                    title,
                    source_format: "chat-gpt",
                    turns,
                    occurred_at: gpt_create_time(item),
                });
            };
            let mut queue: Vec<String> = root
                .get("children")
                .and_then(Value::as_array)
                .map(|children| {
                    children
                        .iter()
                        .filter_map(|child| child.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default();
            while let Some(node_id) = queue.pop() {
                let Some(node) = mapping.get(&node_id) else {
                    continue;
                };
                if let Some(turn) = chatgpt_message_turn(node.get("message")) {
                    turns.push(turn);
                }
                if let Some(children) = node.get("children").and_then(Value::as_array) {
                    for child in children.iter().filter_map(Value::as_str).rev() {
                        queue.push(child.to_owned());
                    }
                }
            }
        }
    }
    Ok(ParsedConversation {
        title,
        source_format: "chat-gpt",
        turns,
        occurred_at: gpt_create_time(item),
    })
}

fn chatgpt_message_turn(message: Option<&Value>) -> Option<ChatTurn> {
    let message = message?;
    let role = message
        .get("author")
        .and_then(|author| author.get("role"))
        .and_then(Value::as_str)?;
    let role = match role {
        "user" => ChatRole::User,
        "assistant" => ChatRole::Assistant,
        _ => return None,
    };
    let parts = message
        .get("content")
        .and_then(|content| content.get("parts"))
        .and_then(Value::as_array)?;
    let text = parts
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_owned();
    if text.is_empty() {
        None
    } else {
        Some(ChatTurn { role, text })
    }
}

fn parse_claude_conversation(item: &Value) -> AppResult<ParsedConversation> {
    let title = json_title(item.get("name"));
    let messages = item
        .get("chat_messages")
        .and_then(Value::as_array)
        .ok_or_else(|| AppError::Import("Claude 会话缺少 chat_messages 字段".into()))?;
    let mut turns = Vec::new();
    for message in messages {
        let Some(role) = message
            .get("sender")
            .and_then(Value::as_str)
            .and_then(|sender| match sender {
                "human" | "user" => Some(ChatRole::User),
                "assistant" => Some(ChatRole::Assistant),
                _ => None,
            })
        else {
            continue;
        };
        let text = claude_message_text(message);
        if !text.is_empty() {
            turns.push(ChatTurn { role, text });
        }
    }
    Ok(ParsedConversation {
        title,
        source_format: "chat-claude",
        turns,
        occurred_at: item
            .get("created_at")
            .or_else(|| item.get("updated_at"))
            .and_then(Value::as_str)
            .map(str::to_owned),
    })
}

fn claude_message_text(message: &Value) -> String {
    if let Some(text) = message.get("text").and_then(Value::as_str) {
        return text.trim().to_owned();
    }
    message
        .get("content")
        .and_then(Value::as_array)
        .map(|blocks| {
            blocks
                .iter()
                .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
                .filter_map(|block| block.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
        .trim()
        .to_owned()
}

const USER_MARKERS: &[&str] = &["用户", "我", "提问", "问", "User", "user", "USER", "Human", "human"];
const ASSISTANT_MARKERS: &[&str] = &[
    "助手", "答", "回答", "AI", "Ai", "ai", "Assistant", "assistant", "Claude", "ChatGPT", "GPT", "Bot", "bot",
];

/// 行首「说话人：」标记识别。标记后必须紧跟冒号，避免把普通句子误判成说话人。
fn detect_speaker(line: &str) -> Option<(ChatRole, &str)> {
    let trimmed = line.trim_start();
    for marker in ASSISTANT_MARKERS {
        if let Some(rest) = strip_marker(trimmed, marker) {
            return Some((ChatRole::Assistant, rest));
        }
    }
    for marker in USER_MARKERS {
        if let Some(rest) = strip_marker(trimmed, marker) {
            return Some((ChatRole::User, rest));
        }
    }
    None
}

fn strip_marker<'a>(line: &'a str, marker: &str) -> Option<&'a str> {
    let rest = line.strip_prefix(marker)?;
    let rest = rest
        .strip_prefix('：')
        .or_else(|| rest.strip_prefix(':'))?;
    Some(rest.trim())
}

/// 对话式 Markdown：标记行开新 turn，其余行归入当前 turn。
/// 文件开头允许一行标题（通常是 `# 标题`），识别不出双方对话则明确报错，
/// 不静默当作普通文档导入。
fn parse_markdown_dialogue(text: &str, fallback_title: &str) -> AppResult<ParsedConversation> {
    let mut preamble: Vec<&str> = Vec::new();
    let mut turns: Vec<ChatTurn> = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim_end();
        if trimmed.trim().is_empty() {
            continue;
        }
        match detect_speaker(trimmed) {
            Some((role, rest)) => turns.push(ChatTurn {
                role,
                text: rest.to_owned(),
            }),
            None => {
                if turns.is_empty() {
                    preamble.push(trimmed.trim());
                } else if let Some(last) = turns.last_mut() {
                    if !last.text.is_empty() {
                        last.text.push('\n');
                    }
                    last.text.push_str(trimmed.trim_start());
                }
            }
        }
    }
    let turns: Vec<ChatTurn> = turns
        .into_iter()
        .filter(|turn| !turn.text.trim().is_empty())
        .collect();
    let has_user = turns.iter().any(|turn| turn.role == ChatRole::User);
    let has_assistant = turns.iter().any(|turn| turn.role == ChatRole::Assistant);
    if turns.len() < 2 || !has_user || !has_assistant {
        return Err(AppError::Import(
            "未识别出「用户：/助手：」式的对话结构；普通文档请改用「导入文档」入口".into(),
        ));
    }
    let title = preamble
        .first()
        .map(|line| line.trim_start_matches('#').trim())
        .filter(|title| !title.is_empty())
        .unwrap_or(fallback_title);
    Ok(ParsedConversation {
        title: title.to_owned(),
        source_format: "chat-md",
        turns,
        // 对话式 Markdown 自身无时间元数据，落库时回退文件 mtime。
        occurred_at: None,
    })
}

/// 源文件的修改时间（RFC3339）：对话式 Markdown 等无时间元数据时的兜底。
fn file_mtime_rfc3339(path: &Path) -> Option<String> {
    fs::metadata(path)
        .ok()
        .map(|metadata| crate::agent_scan::mtime_rfc3339(&metadata))
}

/// 单会话落库（公开给 Agent 扫描导入复用）：校验、查重、写对话文件、建记录。
pub(crate) fn import_parsed_conversation(
    library: &ManagedLibrary,
    file_label: &str,
    project_id: Option<&str>,
    duplicate_confirmed: bool,
    conversation: ParsedConversation,
    outcome: &mut ChatImportOutcome,
) {
    let title = sanitize_title(&conversation.title);
    if conversation.turns.is_empty() {
        outcome.skipped.push(ChatImportSkip {
            file: file_label.to_owned(),
            title: Some(title),
            reason: "没有可导入的对话内容".into(),
        });
        return;
    }
    let total_chars: usize = conversation
        .turns
        .iter()
        .map(|turn| turn.text.chars().count())
        .sum();
    if total_chars > MAX_TEXT_CHARACTERS {
        outcome.skipped.push(ChatImportSkip {
            file: file_label.to_owned(),
            title: Some(title),
            reason: "会话过长（超过 200 万字符），请拆分后再导入".into(),
        });
        return;
    }
    let hash = conversation_hash(&conversation);
    match library
        .repository()
        .find_record_by_source_hash("chat", &hash, project_id)
    {
        Ok(Some(_)) if !duplicate_confirmed => {
            outcome.skipped.push(ChatImportSkip {
                file: file_label.to_owned(),
                title: Some(title),
                reason: "已导入过（重复）".into(),
            });
            return;
        }
        Ok(_) => {}
        Err(error) => {
            outcome.skipped.push(ChatImportSkip {
                file: file_label.to_owned(),
                title: Some(title),
                reason: format!("查重失败：{error}"),
            });
            return;
        }
    }

    let segments = conversation_segments(&conversation.turns);
    // 优先用导出元数据里的对话真实时间；没有就回退源文件 mtime（比导入时刻真实）。
    let occurred_at = conversation
        .occurred_at
        .clone()
        .or_else(|| file_mtime_rfc3339(Path::new(file_label)));
    let record_id = Uuid::new_v4().to_string();
    let now = Utc::now();
    let relative_path = PathBuf::from("documents")
        .join(format!("{:04}", now.year()))
        .join(format!("{:02}", now.month()))
        .join(format!("{record_id}.md"));
    let destination = library.root().join(&relative_path);
    if let Err(error) = write_markdown(&destination, &conversation_markdown(&conversation)) {
        outcome.skipped.push(ChatImportSkip {
            file: file_label.to_owned(),
            title: Some(title),
            reason: format!("写入对话文件失败：{error}"),
        });
        return;
    }
    match library.repository().create_chat_record(
        &record_id,
        &title,
        project_id,
        &relative_path,
        &hash,
        conversation.source_format,
        occurred_at.as_deref(),
        &segments,
    ) {
        Ok(record) => outcome.imported.push(IngestResult {
            record_id: record.id,
            hash,
            duplicate: false,
            duration_ms: 0,
            title: record.title,
        }),
        Err(error) => {
            let _ = fs::remove_file(&destination);
            outcome.skipped.push(ChatImportSkip {
                file: file_label.to_owned(),
                title: Some(title),
                reason: format!("写入记录失败：{error}"),
            });
        }
    }
}

fn json_title(value: Option<&Value>) -> String {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .unwrap_or("未命名对话")
        .to_owned()
}

fn sanitize_title(title: &str) -> String {
    let single_line = title.split_whitespace().collect::<Vec<_>>().join(" ");
    single_line.chars().take(120).collect()
}

fn conversation_segments(turns: &[ChatTurn]) -> Vec<TranscriptSegmentInput> {
    let mut segments = Vec::new();
    let mut sequence = 0i64;
    for turn in turns {
        for piece in chunk_text(&turn.text, TURN_CHAR_LIMIT) {
            segments.push(TranscriptSegmentInput {
                start_ms: sequence * 1000,
                end_ms: (sequence + 1) * 1000,
                speaker_label: Some(turn.role.label().to_owned()),
                original_text: piece,
            });
            sequence += 1;
        }
    }
    segments
}

fn chunk_text(text: &str, limit: usize) -> Vec<String> {
    text.chars()
        .collect::<Vec<_>>()
        .chunks(limit)
        .map(|chunk| chunk.iter().collect())
        .collect()
}

/// 会话内容指纹：同格式 + 同标题 + 同对话内容视为重复；换格式重导不受影响。
fn conversation_hash(conversation: &ParsedConversation) -> String {
    let mut hasher = Sha256::new();
    hasher.update(conversation.source_format.as_bytes());
    hasher.update([0x1f]);
    hasher.update(conversation.title.as_bytes());
    hasher.update([0x1f]);
    for turn in &conversation.turns {
        hasher.update(turn.role.label().as_bytes());
        hasher.update([b'\n']);
        hasher.update(turn.text.as_bytes());
        hasher.update([0x1e]);
    }
    format!("{:x}", hasher.finalize())
}

fn conversation_markdown(conversation: &ParsedConversation) -> String {
    let mut markdown = format!("# {}\n", sanitize_title(&conversation.title));
    for turn in &conversation.turns {
        markdown.push_str(&format!("\n{}：{}\n", turn.role.label(), turn.text));
    }
    markdown
}

fn write_markdown(destination: &Path, content: &str) -> AppResult<()> {
    let parent = destination
        .parent()
        .ok_or_else(|| AppError::Import("无法确定受管理目录".into()))?;
    fs::create_dir_all(parent)?;
    fs::write(destination, content)?;
    Ok(())
}

/* ----------------------- v0.8：本机 Agent 会话适配器 ----------------------- */

/// 导入一个选中的本机 Agent 会话（路径已由命令层做白名单校验）。
pub fn import_agent_sessions(
    library: &ManagedLibrary,
    selections: &[(crate::agent_scan::AgentKind, String)],
    project_id: Option<&str>,
    duplicate_confirmed: bool,
) -> AppResult<ChatImportOutcome> {
    let mut outcome = ChatImportOutcome {
        imported: Vec::new(),
        skipped: Vec::new(),
    };
    for (agent, path) in selections {
        let file_label = path.clone();
        if fs::metadata(path).is_ok_and(|meta| meta.len() > crate::agent_scan::MAX_SESSION_BYTES) {
            outcome.skipped.push(ChatImportSkip {
                file: file_label,
                title: None,
                reason: "会话文件过大，暂不支持导入".into(),
            });
            continue;
        }
        let parsed = match agent {
            crate::agent_scan::AgentKind::ClaudeCode => parse_claude_code_session(Path::new(path)),
            crate::agent_scan::AgentKind::Codex => parse_codex_session(Path::new(path)),
        };
        match parsed {
            Ok(conversation) => import_parsed_conversation(
                library,
                &file_label,
                project_id,
                duplicate_confirmed,
                conversation,
                &mut outcome,
            ),
            Err(error) => outcome.skipped.push(ChatImportSkip {
                file: file_label,
                title: None,
                reason: error.to_string(),
            }),
        }
    }
    Ok(outcome)
}

/// ChatGPT 导出的 create_time 是 Unix 秒（浮点），转 RFC3339。
fn gpt_create_time(item: &Value) -> Option<String> {
    let seconds = item.get("create_time").and_then(Value::as_f64)?;
    chrono::DateTime::from_timestamp(seconds as i64, 0)
        .map(|stamp| stamp.to_rfc3339())
}

fn json_timestamp(value: &Value) -> Option<String> {
    value
        .get("timestamp")
        .and_then(Value::as_str)
        .map(str::to_owned)
}

/// Claude Code 会话（~/.claude/projects/**/*.jsonl）：逐行流式解析，
/// 只取 user/assistant 的文本内容；侧链（子代理）、工具块、`<` 开头的
/// 包装文本（命令/本地输出）一律忽略。
pub(crate) fn parse_claude_code_session(path: &Path) -> AppResult<ParsedConversation> {
    let file = fs::File::open(path)?;
    let reader = BufReader::with_capacity(64 * 1024, file);
    let mut turns: Vec<ChatTurn> = Vec::new();
    let mut title: Option<String> = None;
    let mut claude_code_timestamp: Option<String> = None;
    for line in reader.lines() {
        let line = line?;
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if claude_code_timestamp.is_none() {
            claude_code_timestamp = json_timestamp(&value);
        }
        let role = match value.get("type").and_then(Value::as_str) {
            Some("user") => ChatRole::User,
            Some("assistant") => ChatRole::Assistant,
            Some("summary") => {
                if title.is_none() {
                    title = value
                        .get("summary")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|text| !text.is_empty())
                        .map(str::to_owned);
                }
                continue;
            }
            _ => continue,
        };
        if value
            .get("isSidechain")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            continue;
        }
        let text = crate::agent_scan::claude_message_text(value.get("message"));
        let text = crate::agent_scan::filter_wrapped_text(&text);
        if text.is_empty() {
            continue;
        }
        if title.is_none() && role == ChatRole::User {
            title = Some(text.chars().take(60).collect::<String>());
        }
        turns.push(ChatTurn { role, text });
    }
    let turns: Vec<ChatTurn> = turns
        .into_iter()
        .filter(|turn| !turn.text.trim().is_empty())
        .collect();
    if turns.is_empty() {
        return Err(AppError::Import("会话里没有可导入的对话内容".into()));
    }
    let fallback = path
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("未命名会话");
    Ok(ParsedConversation {
        title: title.unwrap_or_else(|| fallback.to_owned()),
        source_format: "claude-code",
        turns,
        occurred_at: claude_code_timestamp,
    })
}

/// Codex 会话（~/.codex/sessions/**/*.jsonl）：只取 response_item.message 的
/// user/assistant 文本；system/developer 角色、`<environment_context>` 等
/// 注入型内容一律忽略。标题取首条用户消息，回退 cwd 目录名。
pub(crate) fn parse_codex_session(path: &Path) -> AppResult<ParsedConversation> {
    let file = fs::File::open(path)?;
    let reader = BufReader::with_capacity(64 * 1024, file);
    let mut turns: Vec<ChatTurn> = Vec::new();
    let mut title: Option<String> = None;
    let mut cwd_title: Option<String> = None;
    let mut codex_timestamp: Option<String> = None;
    for line in reader.lines() {
        let line = line?;
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if value.get("type").and_then(Value::as_str) == Some("session_meta") {
            if cwd_title.is_none() {
                cwd_title = value
                    .pointer("/payload/cwd")
                    .and_then(Value::as_str)
                    .map(Path::new)
                    .and_then(Path::file_name)
                    .and_then(|name| name.to_str())
                    .map(str::to_owned);
            }
            if codex_timestamp.is_none() {
                codex_timestamp = value
                    .pointer("/payload/timestamp")
                    .or_else(|| value.get("timestamp"))
                    .and_then(Value::as_str)
                    .map(str::to_owned);
            }
            continue;
        }
        if value.get("type").and_then(Value::as_str) != Some("response_item") {
            continue;
        }
        let payload = value.get("payload");
        if payload
            .and_then(|payload| payload.get("type"))
            .and_then(Value::as_str)
            != Some("message")
        {
            continue;
        }
        let role = match payload
            .and_then(|payload| payload.get("role"))
            .and_then(Value::as_str)
        {
            Some("user") => ChatRole::User,
            Some("assistant") => ChatRole::Assistant,
            _ => continue,
        };
        let text = payload
            .and_then(|payload| payload.get("content"))
            .and_then(Value::as_array)
            .map(|blocks| {
                blocks
                    .iter()
                    .filter(|block| {
                        matches!(
                            block.get("type").and_then(Value::as_str),
                            Some("input_text") | Some("output_text")
                        )
                    })
                    .filter_map(|block| block.get("text").and_then(Value::as_str))
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();
        let text = crate::agent_scan::filter_wrapped_text(&text);
        if text.is_empty() {
            continue;
        }
        if title.is_none() && role == ChatRole::User {
            title = Some(text.chars().take(60).collect::<String>());
        }
        turns.push(ChatTurn { role, text });
    }
    let turns: Vec<ChatTurn> = turns
        .into_iter()
        .filter(|turn| !turn.text.trim().is_empty())
        .collect();
    if turns.is_empty() {
        return Err(AppError::Import("会话里没有可导入的对话内容".into()));
    }
    let fallback = cwd_title.unwrap_or_else(|| {
        path.file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or("未命名会话")
            .to_owned()
    });
    Ok(ParsedConversation {
        title: title.unwrap_or(fallback),
        source_format: "codex",
        turns,
        occurred_at: codex_timestamp,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn chatgpt_export() -> Value {
        json!([
            {
                "title": "关于定价的讨论",
                "current_node": "m3",
                "mapping": {
                    "root": { "id": "root", "parent": null, "children": ["m1"], "message": null },
                    "m1": {
                        "id": "m1",
                        "parent": "root",
                        "children": ["m2"],
                        "message": { "author": { "role": "system" }, "content": { "parts": ["你是助手"] } }
                    },
                    "m2": {
                        "id": "m2",
                        "parent": "m1",
                        "children": ["m3"],
                        "message": { "author": { "role": "user" }, "content": { "parts": ["我觉得订阅制更合理"] } }
                    },
                    "m3": {
                        "id": "m3",
                        "parent": "m2",
                        "children": [],
                        "message": { "author": { "role": "assistant" }, "content": { "parts": ["你的理由是什么？", "补充一下背景会更好。"] } }
                    }
                }
            }
        ])
    }

    #[test]
    fn chatgpt_export_walks_current_branch_in_order() {
        let conversations = parse_json_export(&chatgpt_export()).unwrap();
        assert_eq!(conversations.len(), 1);
        let conversation = conversations.into_iter().next().unwrap().unwrap();
        assert_eq!(conversation.title, "关于定价的讨论");
        assert_eq!(conversation.turns.len(), 2);
        assert_eq!(conversation.turns[0].role, ChatRole::User);
        assert_eq!(conversation.turns[0].text, "我觉得订阅制更合理");
        assert_eq!(conversation.turns[1].role, ChatRole::Assistant);
        assert!(conversation.turns[1].text.contains("你的理由是什么？"));
    }

    #[test]
    fn claude_export_maps_senders() {
        let export = json!([
            {
                "name": "记忆设计讨论",
                "chat_messages": [
                    { "sender": "human", "text": "记忆应该分几层？" },
                    { "sender": "assistant", "text": "至少四层：对话、原子、场景、画像。" },
                    { "sender": "system", "text": "忽略我" }
                ]
            }
        ]);
        let conversations = parse_json_export(&export).unwrap();
        let conversation = conversations.into_iter().next().unwrap().unwrap();
        assert_eq!(conversation.source_format, "chat-claude");
        assert_eq!(conversation.turns.len(), 2);
        assert_eq!(conversation.turns[0].role, ChatRole::User);
        assert_eq!(conversation.turns[1].role, ChatRole::Assistant);
    }

    #[test]
    fn unknown_json_shape_is_rejected() {
        let export = json!([{ "foo": "bar" }]);
        assert!(parse_json_export(&export).is_err());
    }

    #[test]
    fn markdown_dialogue_requires_both_sides() {
        let parsed = parse_markdown_dialogue(
            "# 周会\n\n用户：今天的结论是什么？\n\n助手：三个结论，第一是上线时间。\n",
            "回退标题",
        )
        .unwrap();
        assert_eq!(parsed.title, "周会");
        assert_eq!(parsed.turns.len(), 2);
        assert_eq!(parsed.turns[1].text, "三个结论，第一是上线时间。");

        let only_user = parse_markdown_dialogue("用户：第一行\n用户：第二行", "标题");
        assert!(only_user.is_err());
        let no_marker = parse_markdown_dialogue("这是一份普通文档。\n没有任何对话。", "标题");
        assert!(no_marker.is_err());
    }

    #[test]
    fn marker_requires_colon_right_after() {
        assert!(detect_speaker("用户：你好").is_some());
        assert!(detect_speaker("用户:你好").is_some());
        assert!(detect_speaker("用户你好").is_none());
        assert!(detect_speaker("答案：四十二").is_none());
        assert!(detect_speaker("答：四十二").is_some());
    }

    #[test]
    fn long_turns_are_split_into_800_char_segments() {
        let turns = vec![ChatTurn {
            role: ChatRole::User,
            text: "长".repeat(1700),
        }];
        let segments = conversation_segments(&turns);
        assert_eq!(segments.len(), 3);
        assert_eq!(segments[0].speaker_label.as_deref(), Some("用户"));
        assert_eq!(segments[0].start_ms, 0);
        assert_eq!(segments[2].start_ms, 2000);
    }

    #[test]
    fn conversation_hash_changes_with_content() {
        let base = ParsedConversation {
            title: "t".into(),
            source_format: "chat-md",
            turns: vec![ChatTurn { role: ChatRole::User, text: "a".into() }],
            occurred_at: None,
        };
        let same = ParsedConversation {
            title: "t".into(),
            source_format: "chat-md",
            turns: vec![ChatTurn { role: ChatRole::User, text: "a".into() }],
            occurred_at: None,
        };
        let different = ParsedConversation {
            title: "t".into(),
            source_format: "chat-md",
            turns: vec![ChatTurn { role: ChatRole::User, text: "b".into() }],
            occurred_at: None,
        };
        assert_eq!(conversation_hash(&base), conversation_hash(&same));
        assert_ne!(conversation_hash(&base), conversation_hash(&different));
    }

    fn temp_session(name: &str, content: &str) -> (std::path::PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("chat_import_agent_{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        fs::write(&path, content).unwrap();
        (dir, path)
    }

    #[test]
    fn claude_code_session_skips_sidechain_and_wrapped_text() {
        let (_dir, path) = temp_session(
            "abc.jsonl",
            concat!(
                "{\"type\":\"mode\",\"mode\":\"normal\",\"sessionId\":\"s\"}\n",
                "{\"type\":\"summary\",\"summary\":\"讨论灰度发布\"}\n",
                "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"直接问题\"}}\n",
                "{\"type\":\"assistant\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"这是回答\"},{\"type\":\"tool_use\",\"name\":\"read\"}]}}\n",
                "{\"type\":\"user\",\"isSidechain\":true,\"message\":{\"role\":\"user\",\"content\":\"子代理消息\"}}\n",
                "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"<command-name>/clear</command-name>\"}}\n",
            ),
        );
        let parsed = parse_claude_code_session(&path).unwrap();
        assert_eq!(parsed.source_format, "claude-code");
        assert_eq!(parsed.title, "讨论灰度发布");
        assert_eq!(parsed.turns.len(), 2);
        assert_eq!(parsed.turns[0].role, ChatRole::User);
        assert_eq!(parsed.turns[0].text, "直接问题");
        assert_eq!(parsed.turns[1].text, "这是回答");
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn codex_session_filters_context_and_falls_back_to_cwd_title() {
        let (_dir, path) = temp_session(
            "rollout-x.jsonl",
            concat!(
                "{\"timestamp\":\"2026-10-11T03:46:45Z\",\"type\":\"session_meta\",\"payload\":{\"cwd\":\"/Users/demo/工作目录\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"<environment_context>env</environment_context>\"}]}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"真实问题\"}]}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"回答\"}]}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"system\",\"content\":[{\"type\":\"input_text\",\"text\":\"系统\"}]}}\n",
                "{\"type\":\"event_msg\",\"payload\":{\"type\":\"agent_message\",\"text\":\"不是消息体\"}}\n",
            ),
        );
        let parsed = parse_codex_session(&path).unwrap();
        assert_eq!(parsed.source_format, "codex");
        assert_eq!(parsed.turns.len(), 2);
        assert_eq!(parsed.turns[0].text, "真实问题");
        assert_eq!(parsed.turns[1].role, ChatRole::Assistant);
        // 无 summary 时标题取首条用户消息
        assert_eq!(parsed.title, "真实问题");

        let empty = temp_session(
            "rollout-y.jsonl",
            "{\"type\":\"session_meta\",\"payload\":{\"cwd\":\"/Users/demo/另一个目录\"}}\n",
        );
        assert!(parse_codex_session(&empty.1).is_err());
        let _ = fs::remove_dir_all(path.parent().unwrap());
        let _ = fs::remove_dir_all(&empty.0);
    }

    #[test]
    fn agent_import_lands_chat_record_and_dedupes() {
        let (_dir, path) = temp_session(
            "rollout-x.jsonl",
            concat!(
                "{\"type\":\"session_meta\",\"payload\":{\"cwd\":\"/Users/demo/工作目录\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"帮我看看这个功能怎么验收\"}]}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"分三层验收。\"}]}}\n",
            ),
        );
        let root = std::env::temp_dir().join(format!("chat_import_agent_db_{}", uuid::Uuid::new_v4()));
        let library = crate::library::ManagedLibrary::open(root.clone()).unwrap();
        let selection = (crate::agent_scan::AgentKind::Codex, path.display().to_string());

        let outcome = import_agent_sessions(&library, std::slice::from_ref(&selection), None, false)
            .unwrap();
        assert_eq!(outcome.imported.len(), 1, "应导入: {:?}", outcome.skipped);
        let record = library
            .repository()
            .get_record(&outcome.imported[0].record_id)
            .unwrap();
        assert_eq!(record.source_type, "chat");
        let segments = library.repository().list_transcript_segments(&record.id).unwrap();
        assert!(segments.iter().any(|segment| segment.speaker_label.as_deref() == Some("用户")));
        assert!(segments.iter().any(|segment| segment.speaker_label.as_deref() == Some("助手")));

        let repeat = import_agent_sessions(&library, &[selection], None, false).unwrap();
        assert!(repeat.imported.is_empty(), "重复导入不应产生新记录");
        assert_eq!(repeat.skipped.len(), 1);
        assert!(repeat.skipped[0].reason.contains("已导入过"));

        let _ = fs::remove_dir_all(path.parent().unwrap());
        let _ = fs::remove_dir_all(root);
    }
}
