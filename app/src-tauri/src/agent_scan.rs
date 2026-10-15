//! 扫描本机主流 Agent 的会话与记忆文件。
//!
//! 隐私边界：只读白名单位置——Claude Code 与 Codex 的会话目录、三个记忆文件
//! 路径，绝不做全盘扫描；扫描只读文件头部几 KB 提取元数据，不做整文件解析；
//! 全程本地，无网络。导入由 chat_import 的解析适配器与既有管线完成。

use crate::error::AppResult;
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

/// 单会话大小上限：扫描时预标记「过大」默认不勾，导入时同样拒收。
pub const MAX_SESSION_BYTES: u64 = 50 * 1024 * 1024;
/// 扫描时读取的文件头大小：只为提取标题/起始时间，不解析全文。
const HEAD_BYTES: u64 = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgentKind {
    ClaudeCode,
    Codex,
}

impl AgentKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude-code",
            Self::Codex => "codex",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::ClaudeCode => "Claude Code",
            Self::Codex => "Codex",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "claude-code" => Some(Self::ClaudeCode),
            "codex" => Some(Self::Codex),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredSession {
    pub agent: AgentKind,
    pub path: String,
    pub title: String,
    pub started_at: String,
    pub size_bytes: u64,
    pub oversized: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredMemoryFile {
    pub agent_label: String,
    pub path: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentScanResult {
    pub sessions: Vec<DiscoveredSession>,
    pub memory_files: Vec<DiscoveredMemoryFile>,
}

/// 各 Agent 会话目录的默认白名单位置。
pub fn default_agent_roots() -> Vec<(AgentKind, PathBuf)> {
    let home = home_dir();
    vec![
        (AgentKind::ClaudeCode, home.join(".claude").join("projects")),
        (AgentKind::Codex, home.join(".codex").join("sessions")),
    ]
}

/// 记忆/规则文件白名单：路径与归属 Agent 标签。
pub fn default_memory_files() -> Vec<(String, PathBuf)> {
    let home = home_dir();
    vec![
        (
            "Claude Code".to_owned(),
            home.join(".claude").join("CLAUDE.md"),
        ),
        ("Codex".to_owned(), home.join(".codex").join("AGENTS.md")),
        (
            "Gemini CLI".to_owned(),
            home.join(".gemini").join("GEMINI.md"),
        ),
    ]
}

fn home_dir() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".to_owned()))
}

/// 全量扫描：默认白名单位置。
pub fn scan_agent_sessions() -> AppResult<AgentScanResult> {
    let roots = default_agent_roots();
    let memory = default_memory_files();
    scan_at(&roots, &memory)
}

/// 可注入路径的扫描主体（测试与命令共用）。
pub fn scan_at(
    roots: &[(AgentKind, PathBuf)],
    memory_files: &[(String, PathBuf)],
) -> AppResult<AgentScanResult> {
    let mut sessions = Vec::new();
    for (agent, root) in roots {
        match agent {
            AgentKind::ClaudeCode => sessions.extend(scan_claude_code_root(root)?),
            AgentKind::Codex => sessions.extend(scan_codex_root(root)?),
        }
    }
    sessions.sort_by(|left, right| right.started_at.cmp(&left.started_at));
    let mut memory_results = Vec::new();
    for (agent_label, path) in memory_files {
        let Ok(metadata) = fs::metadata(path) else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }
        memory_results.push(DiscoveredMemoryFile {
            agent_label: agent_label.clone(),
            path: path.display().to_string(),
            size_bytes: metadata.len(),
        });
    }
    Ok(AgentScanResult {
        sessions,
        memory_files: memory_results,
    })
}

/// Claude Code：`~/.claude/projects/<项目 slug>/<session id>.jsonl`，一层目录。
fn scan_claude_code_root(root: &Path) -> AppResult<Vec<DiscoveredSession>> {
    let mut sessions = Vec::new();
    let Some(project_dirs) = read_dirs(root) else {
        return Ok(sessions);
    };
    for project_dir in project_dirs {
        let Ok(entries) = fs::read_dir(&project_dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("jsonl") {
                continue;
            }
            if let Some(session) = describe_session(AgentKind::ClaudeCode, &path, |head| {
                claude_session_brief(head)
            }) {
                sessions.push(session);
            }
        }
    }
    Ok(sessions)
}

/// Codex：`~/.codex/sessions/<年>/<月>/<日>/rollout-*.jsonl`，递归三层。
fn scan_codex_root(root: &Path) -> AppResult<Vec<DiscoveredSession>> {
    let mut files = Vec::new();
    collect_jsonl_files(root, 0, &mut files);
    let mut sessions = Vec::new();
    for path in files {
        if let Some(session) = describe_session(AgentKind::Codex, &path, |head| {
            codex_session_brief(head)
        }) {
            sessions.push(session);
        }
    }
    Ok(sessions)
}

fn collect_jsonl_files(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth > 3 {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_jsonl_files(&path, depth + 1, out);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("jsonl") {
            out.push(path);
        }
    }
}

/// 读文件头、取元数据并委托对应 Agent 的摘要提取；统一填充大小与超大标记。
fn describe_session(
    agent: AgentKind,
    path: &Path,
    brief: impl FnOnce(&str) -> SessionBrief,
) -> Option<DiscoveredSession> {
    let metadata = fs::metadata(path).ok()?;
    if !metadata.is_file() {
        return None;
    }
    let size_bytes = metadata.len();
    let head = read_head(path, HEAD_BYTES);
    let brief = brief(&head);
    let started_at = brief
        .started_at
        .unwrap_or_else(|| mtime_rfc3339(&metadata));
    Some(DiscoveredSession {
        agent,
        path: path.display().to_string(),
        title: brief.title,
        started_at,
        size_bytes,
        oversized: size_bytes > MAX_SESSION_BYTES,
    })
}

struct SessionBrief {
    title: String,
    started_at: Option<String>,
}

fn read_head(path: &Path, max_bytes: u64) -> String {
    use std::io::Read;
    let Ok(mut file) = fs::File::open(path) else {
        return String::new();
    };
    let mut buffer = vec![0_u8; max_bytes as usize];
    let mut total = 0usize;
    loop {
        match file.read(&mut buffer[total..]) {
            Ok(0) => break,
            Ok(count) => {
                total += count;
                if total == buffer.len() {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    String::from_utf8_lossy(&buffer[..total]).to_string()
}

/// Claude Code 头部：`{"type":"summary","summary":"…"}` 或第一条 user 消息做标题。
fn claude_session_brief(head: &str) -> SessionBrief {
    let mut title: Option<String> = None;
    let mut fallback: Option<String> = None;
    for line in head.lines() {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        match value.get("type").and_then(serde_json::Value::as_str) {
            Some("summary") => {
                if title.is_none() {
                    title = value
                        .get("summary")
                        .and_then(serde_json::Value::as_str)
                        .map(str::trim)
                        .filter(|text| !text.is_empty())
                        .map(str::to_owned);
                }
            }
            Some("user") if fallback.is_none() => {
                let text = filter_wrapped_text(&claude_message_text(value.get("message")));
                if !text.is_empty() {
                    fallback = Some(text.chars().take(60).collect::<String>());
                }
            }
            _ => {}
        }
        if title.is_some() {
            break;
        }
    }
    SessionBrief {
        title: title.or(fallback).unwrap_or_else(|| "未命名会话".to_owned()),
        started_at: None,
    }
}

/// Codex 头部：首行 session_meta 带 cwd 与时间戳，标题取「目录名 · 日期」。
fn codex_session_brief(head: &str) -> SessionBrief {
    let mut started_at = None;
    let mut title = None;
    for line in head.lines() {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if value.get("type").and_then(serde_json::Value::as_str) == Some("session_meta") {
            let payload = value.get("payload");
            // 时间戳在 payload 里，旧版本可能只在顶层——两层都探测。
            started_at = payload
                .and_then(|payload| payload.get("timestamp"))
                .or_else(|| value.get("timestamp"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned);
            title = payload
                .and_then(|payload| payload.get("cwd"))
                .and_then(serde_json::Value::as_str)
                .map(Path::new)
                .and_then(Path::file_name)
                .and_then(|name| name.to_str())
                .map(str::to_owned);
            break;
        }
    }
    let date = started_at
        .as_deref()
        .and_then(|stamp| stamp.get(..10))
        .unwrap_or("")
        .to_owned();
    let title = match (title, date.is_empty()) {
        (Some(cwd), false) => format!("{cwd} · {date}"),
        (Some(cwd), true) => cwd,
        (None, _) => "未命名会话".to_owned(),
    };
    SessionBrief { title, started_at }
}

/// 供 chat_import 解析器复用：Claude 消息体可能是字符串或分块数组。
pub(crate) fn claude_message_text(message: Option<&serde_json::Value>) -> String {
    let Some(message) = message else {
        return String::new();
    };
    match message.get("content") {
        Some(serde_json::Value::String(text)) => text.trim().to_owned(),
        Some(serde_json::Value::Array(blocks)) => blocks
            .iter()
            .filter(|block| block.get("type").and_then(serde_json::Value::as_str) == Some("text"))
            .filter_map(|block| block.get("text").and_then(serde_json::Value::as_str))
            .collect::<Vec<_>>()
            .join("\n")
            .trim()
            .to_owned(),
        _ => String::new(),
    }
}

/// Agent 会话里以 `<` 开头的行是命令包装或环境注入（如
/// `<command-name>`、`<local-command-caveat>`），不是用户对话内容。
pub(crate) fn filter_wrapped_text(text: &str) -> String {
    text.lines()
        .filter(|line| !line.trim_start().starts_with('<'))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_owned()
}

pub(crate) fn mtime_rfc3339(metadata: &fs::Metadata) -> String {
    let stamp = metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or_default();
    chrono::DateTime::from_timestamp(stamp, 0)
        .unwrap_or_default()
        .to_rfc3339()
}

fn read_dirs(root: &Path) -> Option<Vec<PathBuf>> {
    let entries = fs::read_dir(root).ok()?;
    Some(
        entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.is_dir())
            .collect(),
    )
}

/// 会话路径白名单校验：导入命令只接受位于对应 Agent 根目录内的路径。
pub fn is_session_path_allowed(agent: AgentKind, path: &str) -> bool {
    let resolved = Path::new(path);
    default_agent_roots()
        .into_iter()
        .filter(|(kind, _)| *kind == agent)
        .any(|(_, root)| resolved.starts_with(&root) && resolved.extension().and_then(|ext| ext.to_str()) == Some("jsonl"))
}

/// 记忆文件路径白名单校验。
pub fn is_memory_path_allowed(path: &str) -> bool {
    let resolved = Path::new(path);
    default_memory_files()
        .into_iter()
        .any(|(_, allowed)| resolved == allowed)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试夹具写文件：直接内联 create_dir_all + write，不抽具名函数。
    fn fixture(path: &Path, content: &str) -> PathBuf {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
        path.to_path_buf()
    }

    #[test]
    fn scan_finds_claude_and_codex_sessions_with_titles() {
        let root = std::env::temp_dir().join(format!("agent_scan_{}", uuid::Uuid::new_v4()));
        let claude_root = root.join(".claude").join("projects");
        let codex_root = root.join(".codex").join("sessions");
        fixture(
            &claude_root
                .join("-Users-demo-project")
                .join("abc.jsonl"),
            "{\"type\":\"summary\",\"summary\":\"讨论灰度发布\"}\n{\"type\":\"user\",\"message\":{\"content\":\"先看结论\"}}\n",
        );
        fixture(
            &codex_root.join("2026").join("10").join("11").join("rollout-x.jsonl"),
            "{\"timestamp\":\"2026-10-11T03:46:45.916Z\",\"type\":\"session_meta\",\"payload\":{\"cwd\":\"/Users/demo/工作目录\",\"session_id\":\"x\"}}\n",
        );
        let memory = fixture(&root.join(".claude").join("CLAUDE.md"), "# 规则");

        let result = scan_at(
            &[(AgentKind::ClaudeCode, claude_root), (AgentKind::Codex, codex_root)],
            &[("Claude Code".to_owned(), memory)],
        )
        .unwrap();
        assert_eq!(result.sessions.len(), 2);
        let claude = result
            .sessions
            .iter()
            .find(|session| session.agent == AgentKind::ClaudeCode)
            .unwrap();
        assert_eq!(claude.title, "讨论灰度发布");
        assert!(!claude.oversized);
        let codex = result
            .sessions
            .iter()
            .find(|session| session.agent == AgentKind::Codex)
            .unwrap();
        assert_eq!(codex.title, "工作目录 · 2026-10-11");
        assert_eq!(codex.started_at, "2026-10-11T03:46:45.916Z");
        assert_eq!(result.memory_files.len(), 1);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn oversized_sessions_are_flagged() {
        let root = std::env::temp_dir().join(format!("agent_scan_big_{}", uuid::Uuid::new_v4()));
        let claude_root = root.join(".claude").join("projects");
        let big = fixture(&claude_root.join("proj").join("big.jsonl"), "{}\n");
        let file = fs::OpenOptions::new().write(true).open(&big).unwrap();
        file.set_len(MAX_SESSION_BYTES + 1).unwrap();
        let result = scan_at(
            &[(AgentKind::ClaudeCode, claude_root)],
            &[],
        )
        .unwrap();
        assert_eq!(result.sessions.len(), 1);
        assert!(result.sessions[0].oversized);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn missing_roots_yield_empty_result() {
        let root = std::env::temp_dir().join(format!("agent_scan_empty_{}", uuid::Uuid::new_v4()));
        let result = scan_at(
            &[(AgentKind::Codex, root.join("nothing"))],
            &[],
        )
        .unwrap();
        assert!(result.sessions.is_empty());
        assert!(result.memory_files.is_empty());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn path_allowlist_blocks_outside_paths() {
        let claude_session = format!(
            "{}/.claude/projects/p/abc.jsonl",
            std::env::var("HOME").unwrap()
        );
        assert!(is_session_path_allowed(AgentKind::ClaudeCode, &claude_session));
        assert!(!is_session_path_allowed(AgentKind::Codex, &claude_session));
        assert!(!is_session_path_allowed(
            AgentKind::ClaudeCode,
            "/etc/passwd"
        ));
        assert!(!is_session_path_allowed(
            AgentKind::ClaudeCode,
            &format!("{}/.claude/projects/p/abc.txt", std::env::var("HOME").unwrap())
        ));
        let memory = format!("{}/.codex/AGENTS.md", std::env::var("HOME").unwrap());
        assert!(is_memory_path_allowed(&memory));
        assert!(!is_memory_path_allowed("/tmp/AGENTS.md"));
    }

    /// 真机冒烟：扫描真实主目录并打印发现摘要。
    /// 运行：ECHO_SCAN_REAL=1 cargo test scan_real_machine -- --ignored --nocapture
    #[test]
    #[ignore = "scans the real home directory; run manually"]
    fn scan_real_machine_reports_sessions() {
        if std::env::var("ECHO_SCAN_REAL").is_err() {
            return;
        }
        let started = std::time::Instant::now();
        let result = scan_agent_sessions().expect("真机扫描失败");
        let elapsed = started.elapsed();
        for agent in [AgentKind::ClaudeCode, AgentKind::Codex] {
            let group: Vec<_> = result
                .sessions
                .iter()
                .filter(|session| session.agent == agent)
                .collect();
            println!(
                "{:?}: {} 个会话，共 {}，最新标题: {}",
                agent,
                group.len(),
                format_bytes(group.iter().map(|session| session.size_bytes).sum()),
                group.first().map(|session| session.title.as_str()).unwrap_or("-"),
            );
        }
        println!("记忆文件: {}", result.memory_files.len());
        for file in &result.memory_files {
            println!("  {} ({}, {})", file.path, file.agent_label, format_bytes(file.size_bytes));
        }
        let oversized = result.sessions.iter().filter(|session| session.oversized).count();
        println!("耗时: {elapsed:?}，超大标记: {oversized}");
        assert!(!result.sessions.is_empty(), "真机上应至少发现一个会话");
    }

    fn format_bytes(bytes: u64) -> String {
        if bytes < 1024 * 1024 {
            format!("{:.0} KB", bytes as f64 / 1024.0)
        } else {
            format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
        }
    }
}
