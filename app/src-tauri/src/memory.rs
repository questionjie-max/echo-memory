//! Versioned external-AI memory views. Source records remain authoritative and immutable.

use crate::error::{AppError, AppResult};
use crate::library::ManagedLibrary;
use crate::state::AppState;
use crate::transcript::effective_text;
use crate::types::{
    ExternalAiSettings, GrowthEdge, GrowthGraph, GrowthNode, MemoryBranch, MemoryGenerationRequest,
    MemoryGenerationStatus, MemoryScope, MemorySnapshot, MemorySnapshotResult,
    MemorySourceReference, MemoryViewKind, TimelineItem,
};
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::process::Command;
use std::time::Duration;

const KEYCHAIN_SERVICE: &str = "com.soloplay.echo-memory.external-ai";
const KEYCHAIN_ACCOUNT: &str = "default";
const CHUNK_TEXT_BUDGET: usize = 28_000;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct MemorySourceRecord {
    id: String,
    title: String,
    project_id: Option<String>,
    project_name: Option<String>,
    imported_at: String,
    analysis: Option<Value>,
    transcript: Vec<MemorySourceSegment>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct MemorySourceSegment {
    id: String,
    start_ms: i64,
    end_ms: i64,
    speaker_label: Option<String>,
    text: String,
}

pub struct OpenAiCompatibleClient {
    endpoint: String,
    model: String,
    api_key: String,
    agent: ureq::Agent,
}

impl OpenAiCompatibleClient {
    pub fn new(base_url: &str, model: &str, api_key: &str) -> AppResult<Self> {
        let base = base_url.trim().trim_end_matches('/');
        if base.is_empty() || model.trim().is_empty() || api_key.trim().is_empty() {
            return Err(AppError::Invalid("外部 AI 配置不完整".to_owned()));
        }
        let endpoint = if base.ends_with("/chat/completions") {
            base.to_owned()
        } else {
            format!("{base}/chat/completions")
        };
        Ok(Self {
            endpoint,
            model: model.trim().to_owned(),
            api_key: api_key.to_owned(),
            agent: ureq::AgentBuilder::new()
                .timeout_connect(Duration::from_secs(10))
                .timeout(Duration::from_secs(120))
                .build(),
        })
    }

    pub fn complete_json(&self, prompt: &str) -> AppResult<String> {
        let body = json!({
            "model": self.model,
            "messages": [
                {"role": "system", "content": "You extract traceable memory structures. Return one valid JSON object only. Never invent source IDs."},
                {"role": "user", "content": prompt}
            ]
        });
        let mut last_transport_error = None;
        for attempt in 0..3 {
            let response = self
                .agent
                .post(&self.endpoint)
                .set("Authorization", &format!("Bearer {}", self.api_key))
                .set("Content-Type", "application/json")
                .send_json(body.clone());
            match response {
                Ok(response) => {
                    let response_body = response
                        .into_string()
                        .map_err(|_| AppError::ExternalAi("外部 AI 返回了无效响应".to_owned()))?;
                    return extract_completion_text(&response_body);
                }
                Err(ureq::Error::Status(code, _response)) if code >= 500 && attempt < 2 => {
                    retry_delay(attempt);
                }
                Err(ureq::Error::Status(code, response)) => {
                    return Err(AppError::ExternalAi(format_external_ai_http_error(
                        code,
                        response,
                        &self.api_key,
                    )));
                }
                Err(ureq::Error::Transport(_)) if attempt < 2 => {
                    last_transport_error = Some(AppError::ExternalAi(
                        "无法连接外部 AI 服务，请检查地址与网络".to_owned(),
                    ));
                    retry_delay(attempt);
                }
                Err(ureq::Error::Transport(_)) => {
                    return Err(last_transport_error.unwrap_or_else(|| {
                        AppError::ExternalAi("无法连接外部 AI 服务，请检查地址与网络".to_owned())
                    }));
                }
            }
        }
        Err(last_transport_error
            .unwrap_or_else(|| AppError::ExternalAi("外部 AI 请求失败，请稍后重试".to_owned())))
    }

    pub fn test(&self) -> AppResult<()> {
        let content = self.complete_json("Return exactly this JSON object: {\"ok\":true}")?;
        let value = parse_json_object(&content)?;
        if value.get("ok").and_then(Value::as_bool) != Some(true) {
            return Err(AppError::ExternalAi(
                "外部 AI 连接成功，但模型未返回预期 JSON".to_owned(),
            ));
        }
        Ok(())
    }
}

fn extract_completion_text(body: &str) -> AppResult<String> {
    if let Ok(value) = serde_json::from_str::<Value>(body) {
        return extract_json_completion_text(&value)
            .filter(|content| !content.trim().is_empty())
            .ok_or_else(|| AppError::ExternalAi("外部 AI 响应缺少内容".to_owned()));
    }

    let mut chunks = Vec::new();
    let mut saw_event = false;
    for line in body.lines() {
        let Some(data) = line.trim().strip_prefix("data:") else {
            continue;
        };
        let data = data.trim();
        if data.is_empty() || data == "[DONE]" {
            continue;
        }
        saw_event = true;
        let Ok(value) = serde_json::from_str::<Value>(data) else {
            continue;
        };
        if let Some(content) = extract_sse_completion_text(&value) {
            chunks.push(content);
        }
    }
    let content = chunks.concat();
    if !content.trim().is_empty() {
        Ok(content)
    } else if saw_event {
        Err(AppError::ExternalAi("外部 AI 响应缺少内容".to_owned()))
    } else {
        Err(AppError::ExternalAi("外部 AI 返回了无效响应".to_owned()))
    }
}

fn extract_json_completion_text(value: &Value) -> Option<String> {
    for root in [Some(value), value.get("data")].into_iter().flatten() {
        if let Some(choice) = root
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|choices| choices.first())
        {
            if let Some(message) = choice.get("message") {
                if let Some(content) = message.get("content").and_then(non_empty_content_text) {
                    return Some(content);
                }
                if let Some(content) = message
                    .get("reasoning_content")
                    .and_then(non_empty_content_text)
                {
                    return Some(content);
                }
            }
            if let Some(content) = choice.get("text").and_then(non_empty_content_text) {
                return Some(content);
            }
        }
        if let Some(content) = root.get("output_text").and_then(non_empty_content_text) {
            return Some(content);
        }
    }
    None
}

fn extract_sse_completion_text(value: &Value) -> Option<String> {
    for root in [Some(value), value.get("data")].into_iter().flatten() {
        if let Some(choice) = root
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|choices| choices.first())
        {
            if let Some(delta) = choice.get("delta") {
                if let Some(content) = delta.get("content").and_then(non_empty_content_text) {
                    return Some(content);
                }
                if let Some(content) = delta
                    .get("reasoning_content")
                    .and_then(non_empty_content_text)
                {
                    return Some(content);
                }
            }
        }
    }
    extract_json_completion_text(value)
}

fn non_empty_content_text(value: &Value) -> Option<String> {
    content_text(value).filter(|content| !content.trim().is_empty())
}

fn content_text(value: &Value) -> Option<String> {
    match value {
        Value::String(content) => Some(content.clone()),
        Value::Array(parts) => {
            let combined = parts.iter().filter_map(content_text).collect::<String>();
            (!combined.is_empty()).then_some(combined)
        }
        Value::Object(map) => {
            if let Some(content) = map.get("text").and_then(content_text) {
                return Some(content);
            }
            if let Some(content) = map.get("content").and_then(content_text) {
                return Some(content);
            }
            if let Some(content) = map.get("value").and_then(content_text) {
                return Some(content);
            }
            serde_json::to_string(value).ok()
        }
        _ => None,
    }
}

fn retry_delay(_attempt: usize) {
    #[cfg(test)]
    let delay = Duration::from_millis(1);
    #[cfg(not(test))]
    let delay = if _attempt == 0 {
        Duration::from_millis(100)
    } else {
        Duration::from_millis(250)
    };
    std::thread::sleep(delay);
}

fn format_external_ai_http_error(code: u16, response: ureq::Response, api_key: &str) -> String {
    let prefix = format!("外部 AI 请求失败（HTTP {code}）");
    let Ok(body) = response.into_string() else {
        return prefix;
    };
    let Ok(value) = serde_json::from_str::<Value>(&body) else {
        return prefix;
    };
    let detail = value
        .pointer("/error/message")
        .and_then(Value::as_str)
        .or_else(|| value.get("error").and_then(Value::as_str))
        .or_else(|| value.get("message").and_then(Value::as_str))
        .or_else(|| value.get("detail").and_then(Value::as_str));
    let Some(detail) = detail else {
        return prefix;
    };

    let redacted = detail.replace(api_key, "[已隐藏]");
    let normalized = redacted.split_whitespace().collect::<Vec<_>>().join(" ");
    let limited: String = normalized.chars().take(240).collect();
    if limited.is_empty() {
        prefix
    } else {
        format!("{prefix}：{limited}")
    }
}

pub fn get_api_key() -> AppResult<Option<String>> {
    if let Ok(value) = std::env::var("ECHO_EXTERNAL_AI_API_KEY") {
        if !value.trim().is_empty() {
            return Ok(Some(value));
        }
    }
    #[cfg(target_os = "macos")]
    {
        let output = Command::new("security")
            .args([
                "find-generic-password",
                "-s",
                KEYCHAIN_SERVICE,
                "-a",
                KEYCHAIN_ACCOUNT,
                "-w",
            ])
            .output()
            .map_err(|_| AppError::Io(std::io::Error::other("无法访问 macOS 钥匙串")))?;
        if !output.status.success() {
            return Ok(None);
        }
        let value = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        return Ok((!value.is_empty()).then_some(value));
    }
    #[cfg(not(target_os = "macos"))]
    Ok(None)
}

pub fn set_api_key(api_key: &str) -> AppResult<()> {
    if api_key.trim().is_empty() {
        return Err(AppError::Invalid("API Key 不能为空".to_owned()));
    }
    #[cfg(target_os = "macos")]
    {
        let status = Command::new("security")
            .args([
                "add-generic-password",
                "-U",
                "-s",
                KEYCHAIN_SERVICE,
                "-a",
                KEYCHAIN_ACCOUNT,
                "-w",
                api_key.trim(),
            ])
            .status()
            .map_err(|_| AppError::Io(std::io::Error::other("无法访问 macOS 钥匙串")))?;
        if !status.success() {
            return Err(AppError::Invalid(
                "API Key 保存到 macOS 钥匙串失败".to_owned(),
            ));
        }
        return Ok(());
    }
    #[cfg(not(target_os = "macos"))]
    Err(AppError::Invalid("当前平台不支持 macOS 钥匙串".to_owned()))
}

pub fn clear_api_key() -> AppResult<()> {
    #[cfg(target_os = "macos")]
    {
        let _ = Command::new("security")
            .args([
                "delete-generic-password",
                "-s",
                KEYCHAIN_SERVICE,
                "-a",
                KEYCHAIN_ACCOUNT,
            ])
            .status();
    }
    Ok(())
}

pub fn external_settings(library: &ManagedLibrary) -> AppResult<ExternalAiSettings> {
    library
        .repository()
        .external_ai_settings(get_api_key()?.is_some())
}

pub fn local_timeline(
    library: &ManagedLibrary,
    scope: &MemoryScope,
    range_start: Option<&str>,
    range_end: Option<&str>,
) -> AppResult<Vec<TimelineItem>> {
    let records = source_records(library, scope, range_start, range_end)?;
    let mut items = Vec::new();
    for record in records {
        let first = record.transcript.first();
        items.push(TimelineItem {
            id: stable_id("local-record", &[&record.id]),
            occurred_at: record.imported_at.clone(),
            item_type: "record".to_owned(),
            title: record.title.clone(),
            summary: record
                .analysis
                .as_ref()
                .and_then(|value| value.get("summary"))
                .and_then(Value::as_str)
                .unwrap_or("已导入记录")
                .to_owned(),
            project_id: record.project_id.clone(),
            project_name: record.project_name.clone(),
            inferred: false,
            confidence: None,
            sources: vec![MemorySourceReference {
                record_id: record.id.clone(),
                segment_id: first.map(|segment| segment.id.clone()),
                start_ms: first.map(|segment| segment.start_ms),
                end_ms: first.map(|segment| segment.end_ms),
                quote_text: first
                    .map(|segment| segment.text.clone())
                    .unwrap_or_default(),
            }],
        });
        if let Some(analysis) = record.analysis.as_ref() {
            for (kind, key) in [
                ("decision", "decisions"),
                ("task", "action_items"),
                ("viewpoint", "key_points"),
            ] {
                for entry in analysis
                    .get(key)
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    let text = entry
                        .get("text")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .trim();
                    if text.is_empty() {
                        continue;
                    }
                    let segment_id = entry
                        .get("citation_segment_ids")
                        .and_then(Value::as_array)
                        .and_then(|ids| ids.first())
                        .and_then(Value::as_str);
                    let source_segment = segment_id.and_then(|id| {
                        record
                            .transcript
                            .iter()
                            .find(|segment| segment.id == id || id == segment.id)
                    });
                    items.push(TimelineItem {
                        id: stable_id("local-analysis", &[&record.id, kind, text]),
                        occurred_at: record.imported_at.clone(),
                        item_type: kind.to_owned(),
                        title: text.to_owned(),
                        summary: text.to_owned(),
                        project_id: record.project_id.clone(),
                        project_name: record.project_name.clone(),
                        inferred: false,
                        confidence: None,
                        sources: vec![MemorySourceReference {
                            record_id: record.id.clone(),
                            segment_id: source_segment.map(|segment| segment.id.clone()),
                            start_ms: source_segment.map(|segment| segment.start_ms),
                            end_ms: source_segment.map(|segment| segment.end_ms),
                            quote_text: entry
                                .get("quote_text")
                                .and_then(Value::as_str)
                                .unwrap_or("")
                                .to_owned(),
                        }],
                    });
                }
            }
        }
    }
    items.sort_by(|a, b| b.occurred_at.cmp(&a.occurred_at).then(a.id.cmp(&b.id)));
    Ok(items)
}

pub fn local_growth_graph(
    library: &ManagedLibrary,
    scope: &MemoryScope,
    range_start: Option<&str>,
    range_end: Option<&str>,
) -> AppResult<GrowthGraph> {
    let timeline = local_timeline(library, scope, range_start, range_end)?;
    Ok(growth_graph_from_timeline(timeline, range_start, range_end))
}

fn growth_graph_from_timeline(
    mut timeline: Vec<TimelineItem>,
    range_start: Option<&str>,
    range_end: Option<&str>,
) -> GrowthGraph {
    timeline.sort_by(|a, b| a.occurred_at.cmp(&b.occurred_at).then(a.id.cmp(&b.id)));

    let mut branch_meta: HashMap<String, (String, Option<String>, String)> = HashMap::new();
    let mut nodes = Vec::with_capacity(timeline.len());
    let mut record_nodes = HashMap::new();
    for item in timeline {
        let project_key = item.project_id.as_deref().unwrap_or("unfiled");
        let branch_id = stable_id("growth-branch", &[project_key]);
        branch_meta.entry(branch_id.clone()).or_insert_with(|| {
            (
                item.project_name
                    .clone()
                    .unwrap_or_else(|| "未归档".to_owned()),
                item.project_id.clone(),
                item.occurred_at.clone(),
            )
        });
        if item.item_type == "record" {
            if let Some(source) = item.sources.first() {
                record_nodes.insert(source.record_id.clone(), item.id.clone());
            }
        }
        nodes.push(GrowthNode {
            id: item.id,
            node_type: item.item_type,
            label: item.title,
            summary: item.summary,
            occurred_at: item.occurred_at,
            branch_id,
            project_id: item.project_id,
            project_name: item.project_name,
            inferred: item.inferred,
            confidence: item.confidence,
            sources: item.sources,
        });
    }

    let mut branch_rows = branch_meta.into_iter().collect::<Vec<_>>();
    branch_rows.sort_by(|(_, (label_a, _, first_a)), (_, (label_b, _, first_b))| {
        first_a.cmp(first_b).then(label_a.cmp(label_b))
    });
    let branches = branch_rows
        .into_iter()
        .enumerate()
        .map(|(order, (id, (label, project_id, _)))| MemoryBranch {
            id,
            label,
            branch_type: "project".to_owned(),
            project_id,
            parent_id: None,
            order: order as i64,
            inferred: false,
        })
        .collect::<Vec<_>>();

    let mut edges = Vec::new();
    let mut previous_by_branch: HashMap<String, String> = HashMap::new();
    for node in nodes.iter().filter(|node| node.node_type == "record") {
        if let Some(previous_id) =
            previous_by_branch.insert(node.branch_id.clone(), node.id.clone())
        {
            edges.push(GrowthEdge {
                id: stable_id("growth-edge", &[&previous_id, &node.id, "continuation"]),
                source_id: previous_id,
                target_id: node.id.clone(),
                relation: "continuation".to_owned(),
                inferred: false,
                confidence: None,
                sources: node.sources.clone(),
            });
        }
    }
    for node in nodes.iter().filter(|node| node.node_type != "record") {
        let Some(record_id) = node.sources.first().map(|source| source.record_id.as_str()) else {
            continue;
        };
        let Some(record_node_id) = record_nodes.get(record_id) else {
            continue;
        };
        edges.push(GrowthEdge {
            id: stable_id("growth-edge", &[record_node_id, &node.id, "produces"]),
            source_id: record_node_id.clone(),
            target_id: node.id.clone(),
            relation: "produces".to_owned(),
            inferred: false,
            confidence: None,
            sources: node.sources.clone(),
        });
    }

    GrowthGraph {
        branches,
        nodes,
        edges,
        range_start: range_start.map(str::to_owned),
        range_end: range_end.map(str::to_owned),
        generated_at: Utc::now().to_rfc3339(),
    }
}

pub fn generate_snapshot(
    state: &AppState,
    request: &MemoryGenerationRequest,
) -> AppResult<MemorySnapshot> {
    let settings = external_settings(&state.library)?;
    if !settings.enabled {
        return Err(AppError::Invalid("请先在设置中启用外部 AI".to_owned()));
    }
    let api_key =
        get_api_key()?.ok_or_else(|| AppError::Invalid("请先配置外部 AI API Key".to_owned()))?;
    let records = source_records(
        &state.library,
        &request.scope,
        request.range_start.as_deref(),
        request.range_end.as_deref(),
    )?;
    if records.is_empty() {
        return Err(AppError::Invalid("当前范围没有可用于生成的记录".to_owned()));
    }
    let feedback = state
        .library
        .repository()
        .feedback_context(&request.view_kind, &request.scope)?;
    let source_record_ids = records
        .iter()
        .map(|record| record.id.clone())
        .collect::<Vec<_>>();
    let request_material = serde_json::to_string(&(request, &records, &feedback))
        .map_err(|error| AppError::Invalid(format!("无法准备外部 AI 数据: {error}")))?;
    let request_hash = hex_hash(&request_material);
    let snapshot = state.library.repository().create_memory_snapshot(
        &request.view_kind,
        &request.scope,
        request.range_start.as_deref(),
        request.range_end.as_deref(),
        &settings.model,
        &source_record_ids,
        &request_hash,
    )?;
    let client = OpenAiCompatibleClient::new(&settings.base_url, &settings.model, &api_key)?;
    let chunks = chunk_records(&records, CHUNK_TEXT_BUDGET)?;
    let mut partials = Vec::new();
    let mut failure: Option<String> = None;
    for (index, chunk) in chunks.iter().enumerate() {
        if state.is_generation_cancelled(&request.generation_id) {
            return state.library.repository().finish_memory_snapshot(
                &snapshot.id,
                MemoryGenerationStatus::Cancelled,
                &MemorySnapshotResult::default(),
                None,
                None,
            );
        }
        let prompt = extraction_prompt(
            &request.view_kind,
            chunk,
            &feedback,
            index + 1,
            chunks.len(),
        )?;
        match complete_snapshot_result(&client, &prompt) {
            Ok(result) => partials.push(result),
            Err(error) => {
                failure = Some(error.to_frontend());
                break;
            }
        }
    }
    if partials.is_empty() {
        return state.library.repository().finish_memory_snapshot(
            &snapshot.id,
            MemoryGenerationStatus::Failed,
            &MemorySnapshotResult::default(),
            None,
            failure.as_deref(),
        );
    }
    if state.is_generation_cancelled(&request.generation_id) {
        return state.library.repository().finish_memory_snapshot(
            &snapshot.id,
            MemoryGenerationStatus::Cancelled,
            &MemorySnapshotResult::default(),
            None,
            None,
        );
    }
    let mut result = if partials.len() == 1 {
        partials.remove(0)
    } else {
        let prompt = merge_prompt(&request.view_kind, &partials, &feedback)?;
        complete_snapshot_result(&client, &prompt).unwrap_or_else(|_| merge_locally(partials))
    };
    let warning = validate_and_normalize(&mut result, &records);
    let status = if failure.is_some() {
        MemoryGenerationStatus::Partial
    } else {
        MemoryGenerationStatus::Completed
    };
    state.library.repository().finish_memory_snapshot(
        &snapshot.id,
        status,
        &result,
        warning.as_deref(),
        failure.as_deref(),
    )
}

fn source_records(
    library: &ManagedLibrary,
    scope: &MemoryScope,
    range_start: Option<&str>,
    range_end: Option<&str>,
) -> AppResult<Vec<MemorySourceRecord>> {
    let (project_id, unfiled) = match scope.kind.as_str() {
        "all" => (None, false),
        "unfiled" => (None, true),
        "project" => (
            Some(
                scope
                    .project_id
                    .as_deref()
                    .ok_or_else(|| AppError::Invalid("项目作用域缺少项目 ID".to_owned()))?,
            ),
            false,
        ),
        _ => return Err(AppError::Invalid("记忆视图作用域无效".to_owned())),
    };
    let repository = library.repository();
    let records = repository.list_records(project_id, unfiled)?;
    let mut result = Vec::new();
    for record in records {
        if !within_range(&record.imported_at, range_start, range_end) {
            continue;
        }
        let transcript = repository
            .list_transcript_segments(&record.id)?
            .into_iter()
            .map(|segment| {
                let text = effective_text(&segment).to_owned();
                MemorySourceSegment {
                    id: segment.id,
                    start_ms: segment.start_ms,
                    end_ms: segment.end_ms,
                    speaker_label: segment.speaker_label,
                    text,
                }
            })
            .collect::<Vec<_>>();
        let analysis = repository
            .latest_analysis(&record.id)?
            .and_then(|stored| serde_json::from_str(&stored.content_json).ok());
        result.push(MemorySourceRecord {
            id: record.id,
            title: record.title,
            project_id: record.project_id,
            project_name: record.project_name,
            imported_at: record.imported_at,
            analysis,
            transcript,
        });
    }
    result.sort_by(|a, b| a.imported_at.cmp(&b.imported_at));
    Ok(result)
}

fn within_range(value: &str, start: Option<&str>, end: Option<&str>) -> bool {
    let parsed = DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|time| time.with_timezone(&Utc));
    let start = start
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|time| time.with_timezone(&Utc));
    let end = end
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|time| time.with_timezone(&Utc));
    parsed.is_none_or(|time| {
        start.is_none_or(|start| time >= start) && end.is_none_or(|end| time <= end)
    })
}

fn chunk_records(records: &[MemorySourceRecord], budget: usize) -> AppResult<Vec<String>> {
    let mut chunks = Vec::new();
    let mut current = String::new();
    for record in records {
        let encoded = serde_json::to_string(record)
            .map_err(|error| AppError::Invalid(format!("无法序列化记录: {error}")))?;
        if encoded.chars().count() > budget {
            if !current.is_empty() {
                chunks.push(std::mem::take(&mut current));
            }
            let mut start = 0;
            let chars = encoded.chars().collect::<Vec<_>>();
            while start < chars.len() {
                let end = (start + budget).min(chars.len());
                chunks.push(chars[start..end].iter().collect());
                start = end;
            }
        } else if current.chars().count() + encoded.chars().count() + 1 > budget {
            chunks.push(std::mem::take(&mut current));
            current = encoded;
        } else {
            if !current.is_empty() {
                current.push('\n');
            }
            current.push_str(&encoded);
        }
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    Ok(chunks)
}

fn extraction_prompt(
    view: &MemoryViewKind,
    chunk: &str,
    feedback: &[crate::types::MemoryFeedback],
    index: usize,
    total: usize,
) -> AppResult<String> {
    Ok(format!(
        "Build the {} memory view from source chunk {index}/{total}. Output strict JSON matching this schema: {}. Every generated object must set inferred=true and include sources with real recordId, optional segmentId, startMs, endMs, quoteText. Allowed relations: belongs_to,involves,supports,opposes,triggers,supplements,revises,validates. Allowed evolution changeType: added,supplemented,revised,overturned,merged,validated. Omit claims without evidence. User feedback from older snapshots must be respected: {}. Source data:\n{}",
        view.as_str(), schema_contract(), serde_json::to_string(feedback).unwrap_or_else(|_| "[]".to_owned()), chunk
    ))
}

fn merge_prompt(
    view: &MemoryViewKind,
    partials: &[MemorySnapshotResult],
    feedback: &[crate::types::MemoryFeedback],
) -> AppResult<String> {
    Ok(format!(
        "Merge and deduplicate these partial {} results. Preserve all valid source references, resolve duplicates, and return strict JSON matching this schema: {}. Respect feedback: {}. Partials: {}",
        view.as_str(), schema_contract(), serde_json::to_string(feedback).unwrap_or_else(|_| "[]".to_owned()), serde_json::to_string(partials).map_err(|error| AppError::Invalid(error.to_string()))?
    ))
}

fn schema_contract() -> &'static str {
    r#"{"timelineItems":[{"id":"","occurredAt":"RFC3339","itemType":"record|event|decision|task|viewpoint","title":"","summary":"","projectId":null,"projectName":null,"inferred":true,"confidence":0.0,"sources":[]}],"nodes":[{"id":"","nodeType":"project|event|theme|viewpoint|decision|task|person","label":"","summary":"","inferred":true,"confidence":0.0,"sources":[]}],"edges":[{"id":"","sourceId":"","targetId":"","relation":"belongs_to|involves|supports|opposes|triggers|supplements|revises|validates","inferred":true,"confidence":0.0,"sources":[]}],"evolutionItems":[{"id":"","topic":"","changeType":"added|supplemented|revised|overturned|merged|validated","beforeText":"","afterText":"","reason":"","occurredAt":"RFC3339","inferred":true,"confidence":0.0,"sources":[]}],"dormantQuestions":[],"stalledProjects":[]}"#
}

fn complete_snapshot_result(
    client: &OpenAiCompatibleClient,
    prompt: &str,
) -> AppResult<MemorySnapshotResult> {
    let first = client.complete_json(prompt)?;
    if let Ok(value) = parse_snapshot_result(&first) {
        return Ok(value);
    }
    let retry = client.complete_json(&format!("Your prior output was invalid. Return only a valid JSON object matching {}. Task: {prompt}", schema_contract()))?;
    parse_snapshot_result(&retry)
}

fn parse_snapshot_result(content: &str) -> AppResult<MemorySnapshotResult> {
    serde_json::from_value(parse_json_object(content)?)
        .map_err(|_| AppError::ExternalAi("外部 AI 返回的记忆结构无效".to_owned()))
}

fn parse_json_object(content: &str) -> AppResult<Value> {
    let trimmed = content.trim();
    if let Ok(value @ Value::Object(_)) = serde_json::from_str::<Value>(trimmed) {
        return Ok(value);
    }

    let mut last_valid = None;
    let mut start = None;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (index, character) in content.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                in_string = false;
            }
            continue;
        }
        match character {
            '"' if depth > 0 => in_string = true,
            '{' => {
                if depth == 0 {
                    start = Some(index);
                }
                depth += 1;
            }
            '}' if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    if let Some(start) = start.take() {
                        let end = index + character.len_utf8();
                        if let Ok(value @ Value::Object(_)) =
                            serde_json::from_str::<Value>(&content[start..end])
                        {
                            last_valid = Some(value);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    last_valid.ok_or_else(|| AppError::ExternalAi("外部 AI 未返回合法 JSON".to_owned()))
}

fn merge_locally(partials: Vec<MemorySnapshotResult>) -> MemorySnapshotResult {
    let mut merged = MemorySnapshotResult::default();
    for mut part in partials {
        merged.timeline_items.append(&mut part.timeline_items);
        merged.nodes.append(&mut part.nodes);
        merged.edges.append(&mut part.edges);
        merged.evolution_items.append(&mut part.evolution_items);
        merged.dormant_questions.append(&mut part.dormant_questions);
        merged.stalled_projects.append(&mut part.stalled_projects);
    }
    merged
}

fn validate_and_normalize(
    result: &mut MemorySnapshotResult,
    records: &[MemorySourceRecord],
) -> Option<String> {
    let valid = records
        .iter()
        .map(|record| {
            (
                record.id.clone(),
                record
                    .transcript
                    .iter()
                    .map(|segment| segment.id.clone())
                    .collect::<HashSet<_>>(),
            )
        })
        .collect::<HashMap<_, _>>();
    let mut removed = 0usize;
    for item in &mut result.timeline_items {
        removed += retain_valid_sources(&mut item.sources, &valid);
        item.inferred = true;
        item.id = stable_id(
            "timeline",
            &[&item.title, &item.occurred_at, &source_key(&item.sources)],
        );
    }
    let mut node_id_map = HashMap::new();
    for node in &mut result.nodes {
        let original_id = node.id.clone();
        removed += retain_valid_sources(&mut node.sources, &valid);
        node.inferred = true;
        node.id = stable_id(
            "node",
            &[&node.node_type, &node.label, &source_key(&node.sources)],
        );
        node_id_map.insert(original_id, node.id.clone());
    }
    let node_ids = result
        .nodes
        .iter()
        .map(|node| node.id.clone())
        .collect::<HashSet<_>>();
    result.edges.retain_mut(|edge| {
        removed += retain_valid_sources(&mut edge.sources, &valid);
        edge.inferred = true;
        if let Some(mapped) = node_id_map.get(&edge.source_id) {
            edge.source_id = mapped.clone();
        }
        if let Some(mapped) = node_id_map.get(&edge.target_id) {
            edge.target_id = mapped.clone();
        }
        edge.id = stable_id(
            "edge",
            &[
                &edge.source_id,
                &edge.target_id,
                &edge.relation,
                &source_key(&edge.sources),
            ],
        );
        let keep = node_ids.contains(&edge.source_id) && node_ids.contains(&edge.target_id);
        if !keep {
            removed += 1;
        }
        keep
    });
    for item in &mut result.evolution_items {
        removed += retain_valid_sources(&mut item.sources, &valid);
        item.inferred = true;
        item.id = stable_id(
            "evolution",
            &[
                &item.topic,
                &item.change_type,
                &item.after_text,
                &source_key(&item.sources),
            ],
        );
    }
    dedup_by_id(&mut result.timeline_items, |item| item.id.clone());
    dedup_by_id(&mut result.nodes, |item| item.id.clone());
    dedup_by_id(&mut result.edges, |item| item.id.clone());
    dedup_by_id(&mut result.evolution_items, |item| item.id.clone());
    if removed > 0 {
        Some(format!(
            "已移除 {removed} 个无法匹配本地记录或片段的来源/关系"
        ))
    } else {
        None
    }
}

fn retain_valid_sources(
    sources: &mut Vec<MemorySourceReference>,
    valid: &HashMap<String, HashSet<String>>,
) -> usize {
    let before = sources.len();
    sources.retain(|source| {
        valid.get(&source.record_id).is_some_and(|segments| {
            source
                .segment_id
                .as_ref()
                .is_none_or(|id| segments.contains(id))
        })
    });
    before - sources.len()
}

fn dedup_by_id<T, F>(items: &mut Vec<T>, key: F)
where
    F: Fn(&T) -> String,
{
    let mut seen = HashSet::new();
    items.retain(|item| seen.insert(key(item)));
}

fn source_key(sources: &[MemorySourceReference]) -> String {
    sources
        .iter()
        .map(|source| {
            format!(
                "{}:{}",
                source.record_id,
                source.segment_id.as_deref().unwrap_or("")
            )
        })
        .collect::<Vec<_>>()
        .join("|")
}

fn stable_id(prefix: &str, fields: &[&str]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(prefix.as_bytes());
    for field in fields {
        hasher.update([0]);
        hasher.update(field.as_bytes());
    }
    format!("{prefix}-{}", &format!("{:x}", hasher.finalize())[..16])
}

fn hex_hash(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{MemoryEdge, MemoryNode};
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    #[test]
    fn chunks_long_sources_with_a_fixed_budget() {
        let records = vec![MemorySourceRecord {
            id: "r1".to_owned(),
            title: "t".to_owned(),
            project_id: None,
            project_name: None,
            imported_at: Utc::now().to_rfc3339(),
            analysis: None,
            transcript: vec![MemorySourceSegment {
                id: "s1".to_owned(),
                start_ms: 0,
                end_ms: 1,
                speaker_label: None,
                text: "a".repeat(400),
            }],
        }];
        let chunks = chunk_records(&records, 100).unwrap();
        assert!(chunks.len() > 1);
        assert!(chunks.iter().all(|chunk| chunk.chars().count() <= 100));
    }

    #[test]
    fn edges_keep_working_after_node_ids_are_normalized() {
        let records = vec![MemorySourceRecord {
            id: "r1".to_owned(),
            title: "t".to_owned(),
            project_id: None,
            project_name: None,
            imported_at: Utc::now().to_rfc3339(),
            analysis: None,
            transcript: vec![MemorySourceSegment {
                id: "s1".to_owned(),
                start_ms: 0,
                end_ms: 1,
                speaker_label: None,
                text: "quote".to_owned(),
            }],
        }];
        let source = MemorySourceReference {
            record_id: "r1".to_owned(),
            segment_id: Some("s1".to_owned()),
            start_ms: Some(0),
            end_ms: Some(1),
            quote_text: "quote".to_owned(),
        };
        let mut result = MemorySnapshotResult {
            nodes: vec![
                MemoryNode {
                    id: "model-a".to_owned(),
                    node_type: "theme".to_owned(),
                    label: "A".to_owned(),
                    summary: String::new(),
                    inferred: false,
                    confidence: None,
                    sources: vec![source.clone()],
                },
                MemoryNode {
                    id: "model-b".to_owned(),
                    node_type: "decision".to_owned(),
                    label: "B".to_owned(),
                    summary: String::new(),
                    inferred: false,
                    confidence: None,
                    sources: vec![source.clone()],
                },
            ],
            edges: vec![MemoryEdge {
                id: "model-edge".to_owned(),
                source_id: "model-a".to_owned(),
                target_id: "model-b".to_owned(),
                relation: "supports".to_owned(),
                inferred: false,
                confidence: None,
                sources: vec![source],
            }],
            ..Default::default()
        };

        assert!(validate_and_normalize(&mut result, &records).is_none());
        assert_eq!(result.edges.len(), 1);
        assert_eq!(result.edges[0].source_id, result.nodes[0].id);
        assert_eq!(result.edges[0].target_id, result.nodes[1].id);
    }

    fn timeline_item(
        id: &str,
        record_id: &str,
        occurred_at: &str,
        item_type: &str,
        project_id: Option<&str>,
        project_name: Option<&str>,
    ) -> TimelineItem {
        TimelineItem {
            id: id.to_owned(),
            occurred_at: occurred_at.to_owned(),
            item_type: item_type.to_owned(),
            title: id.to_owned(),
            summary: id.to_owned(),
            project_id: project_id.map(str::to_owned),
            project_name: project_name.map(str::to_owned),
            inferred: false,
            confidence: None,
            sources: vec![MemorySourceReference {
                record_id: record_id.to_owned(),
                segment_id: Some(format!("segment-{record_id}")),
                start_ms: Some(0),
                end_ms: Some(1),
                quote_text: id.to_owned(),
            }],
        }
    }

    #[test]
    fn growth_graph_groups_projects_and_builds_stable_edges() {
        let timeline = vec![
            timeline_item(
                "record-2",
                "r2",
                "2026-07-02T08:00:00Z",
                "record",
                Some("p1"),
                Some("产品"),
            ),
            timeline_item(
                "decision-1",
                "r1",
                "2026-07-01T08:00:00Z",
                "decision",
                Some("p1"),
                Some("产品"),
            ),
            timeline_item(
                "record-unfiled",
                "r3",
                "2026-07-03T08:00:00Z",
                "record",
                None,
                None,
            ),
            timeline_item(
                "record-1",
                "r1",
                "2026-07-01T08:00:00Z",
                "record",
                Some("p1"),
                Some("产品"),
            ),
        ];

        let first = growth_graph_from_timeline(
            timeline.clone(),
            Some("2026-07-01T00:00:00Z"),
            Some("2026-07-31T23:59:59Z"),
        );
        let second = growth_graph_from_timeline(
            timeline,
            Some("2026-07-01T00:00:00Z"),
            Some("2026-07-31T23:59:59Z"),
        );

        assert_eq!(first.branches.len(), 2);
        let project_branch = first
            .branches
            .iter()
            .find(|branch| branch.project_id.as_deref() == Some("p1"))
            .unwrap();
        let unfiled_branch = first
            .branches
            .iter()
            .find(|branch| branch.project_id.is_none())
            .unwrap();
        assert_eq!(unfiled_branch.label, "未归档");
        assert_eq!(unfiled_branch.id, stable_id("growth-branch", &["unfiled"]));
        assert!(
            first
                .nodes
                .iter()
                .filter(|node| {
                    node.project_id.as_deref() == Some("p1") && node.branch_id == project_branch.id
                })
                .count()
                == 3
        );
        assert!(first.edges.iter().any(|edge| {
            edge.source_id == "record-1"
                && edge.target_id == "record-2"
                && edge.relation == "continuation"
        }));
        assert!(first.edges.iter().any(|edge| {
            edge.source_id == "record-1"
                && edge.target_id == "decision-1"
                && edge.relation == "produces"
        }));

        assert_eq!(
            first
                .branches
                .iter()
                .map(|item| &item.id)
                .collect::<Vec<_>>(),
            second
                .branches
                .iter()
                .map(|item| &item.id)
                .collect::<Vec<_>>()
        );
        assert_eq!(
            first.nodes.iter().map(|item| &item.id).collect::<Vec<_>>(),
            second.nodes.iter().map(|item| &item.id).collect::<Vec<_>>()
        );
        assert_eq!(
            first.edges.iter().map(|item| &item.id).collect::<Vec<_>>(),
            second.edges.iter().map(|item| &item.id).collect::<Vec<_>>()
        );
    }

    fn serve_responses(responses: Vec<(&str, &str)>) -> (String, thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let responses = responses
            .into_iter()
            .map(|(status, body)| (status.to_owned(), body.to_owned()))
            .collect::<Vec<_>>();
        let handle = thread::spawn(move || {
            let mut requests = Vec::with_capacity(responses.len());
            for (status, body) in responses {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = Vec::new();
                let mut buffer = [0u8; 4096];
                loop {
                    let read = stream.read(&mut buffer).unwrap();
                    if read == 0 {
                        break;
                    }
                    request.extend_from_slice(&buffer[..read]);
                    if let Some(header_end) =
                        request.windows(4).position(|part| part == b"\r\n\r\n")
                    {
                        let headers = String::from_utf8_lossy(&request[..header_end + 4]);
                        let content_length = headers
                            .lines()
                            .find_map(|line| {
                                let (name, value) = line.split_once(':')?;
                                name.eq_ignore_ascii_case("content-length")
                                    .then(|| value.trim().parse::<usize>().ok())
                                    .flatten()
                            })
                            .unwrap_or(0);
                        if request.len() >= header_end + 4 + content_length {
                            break;
                        }
                    }
                }
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(response.as_bytes()).unwrap();
                requests.push(String::from_utf8(request).unwrap());
            }
            requests
        });
        (format!("http://{address}"), handle)
    }

    fn serve_one_response(status: &str, body: &str) -> (String, thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let status = status.to_owned();
        let body = body.to_owned();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut buffer = [0u8; 4096];
            loop {
                let read = stream.read(&mut buffer).unwrap();
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..read]);
                if let Some(header_end) = request.windows(4).position(|part| part == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&request[..header_end + 4]);
                    let content_length = headers
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().ok())
                                .flatten()
                        })
                        .unwrap_or(0);
                    if request.len() >= header_end + 4 + content_length {
                        break;
                    }
                }
            }
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
            String::from_utf8(request).unwrap()
        });
        (format!("http://{address}"), handle)
    }

    #[test]
    fn openai_compatible_client_sends_expected_request() {
        let response = r#"{"choices":[{"message":{"content":"{\"ok\":true}"}}]}"#;
        let (base_url, server) = serve_one_response("200 OK", response);
        let client = OpenAiCompatibleClient::new(&base_url, "test-model", "top-secret").unwrap();

        client.test().unwrap();
        let request = server.join().unwrap();
        let request_lower = request.to_ascii_lowercase();
        assert!(request.starts_with("POST /chat/completions HTTP/1.1"));
        assert!(request_lower.contains("authorization: bearer top-secret"));
        assert!(request.contains(r#""model":"test-model""#));
        assert!(!request.contains(r#""temperature""#));
    }

    #[test]
    fn external_ai_retries_server_errors_twice_then_succeeds() {
        let success = r#"{"choices":[{"message":{"content":"{\"ok\":true}"}}]}"#;
        let (base_url, server) = serve_responses(vec![
            (
                "500 Internal Server Error",
                r#"{"error":{"message":"temporary"}}"#,
            ),
            ("503 Service Unavailable", r#"{"error":{"message":"busy"}}"#),
            ("200 OK", success),
        ]);
        let client = OpenAiCompatibleClient::new(&base_url, "test-model", "top-secret").unwrap();

        assert_eq!(client.complete_json("test").unwrap(), r#"{"ok":true}"#);
        assert_eq!(server.join().unwrap().len(), 3);
    }

    #[test]
    fn external_ai_accepts_reasoning_content_when_content_is_null() {
        let response =
            r#"{"choices":[{"message":{"content":null,"reasoning_content":"{\"ok\":true}"}}]}"#;
        let (base_url, server) = serve_one_response("200 OK", response);
        let client = OpenAiCompatibleClient::new(&base_url, "test-model", "top-secret").unwrap();

        assert_eq!(client.complete_json("test").unwrap(), r#"{"ok":true}"#);
        server.join().unwrap();
    }

    #[test]
    fn external_ai_falls_back_to_reasoning_content_when_content_is_empty() {
        let response =
            r#"{"choices":[{"message":{"content":"","reasoning_content":"{\"ok\":true}"}}]}"#;
        let (base_url, server) = serve_one_response("200 OK", response);
        let client = OpenAiCompatibleClient::new(&base_url, "test-model", "top-secret").unwrap();

        assert_eq!(client.complete_json("test").unwrap(), r#"{"ok":true}"#);
        server.join().unwrap();
    }

    #[test]
    fn external_ai_accepts_content_parts_and_data_wrappers() {
        let response = r#"{"data":{"choices":[{"message":{"content":[{"type":"text","text":"{\"ok\":"},{"type":"text","text":"true}"}]}}]}}"#;
        let (base_url, server) = serve_one_response("200 OK", response);
        let client = OpenAiCompatibleClient::new(&base_url, "test-model", "top-secret").unwrap();

        assert_eq!(client.complete_json("test").unwrap(), r#"{"ok":true}"#);
        server.join().unwrap();
    }

    #[test]
    fn external_ai_accepts_sse_delta_responses() {
        let response = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"{\\\"ok\\\":\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"true}\"}}]}\n\n",
            "data: [DONE]\n\n"
        );
        let (base_url, server) = serve_one_response("200 OK", response);
        let client = OpenAiCompatibleClient::new(&base_url, "test-model", "top-secret").unwrap();

        assert_eq!(client.complete_json("test").unwrap(), r#"{"ok":true}"#);
        server.join().unwrap();
    }

    #[test]
    fn external_ai_reports_missing_content_clearly() {
        let response = r#"{"choices":[{"message":{"content":null}}]}"#;
        let (base_url, server) = serve_one_response("200 OK", response);
        let client = OpenAiCompatibleClient::new(&base_url, "test-model", "top-secret").unwrap();

        let error = client.complete_json("test").unwrap_err().to_string();
        server.join().unwrap();
        assert!(error.contains("响应缺少内容"));
    }

    #[test]
    fn json_parser_accepts_explanatory_text_and_uses_final_object() {
        let value = parse_json_object(
            "先说明结构 {\"draft\":true}。最终答案：```json\n{\"ok\":true,\"text\":\"包含 } 和 \\\"引号\\\"\"}\n```",
        )
        .unwrap();

        assert_eq!(value.get("ok").and_then(Value::as_bool), Some(true));
        assert_eq!(
            value.get("text").and_then(Value::as_str),
            Some("包含 } 和 \"引号\"")
        );
    }

    #[test]
    fn external_ai_http_errors_include_safe_provider_message() {
        let body = r#"{"error":{"message":"Model Not Exist"}}"#;
        let (base_url, server) = serve_responses(vec![("400 Bad Request", body)]);
        let client = OpenAiCompatibleClient::new(&base_url, "test-model", "top-secret").unwrap();

        let error = client.complete_json("test").unwrap_err().to_string();
        assert_eq!(server.join().unwrap().len(), 1);
        assert!(error.contains("HTTP 400"));
        assert!(error.contains("Model Not Exist"));
    }

    #[test]
    fn external_ai_http_errors_do_not_expose_api_keys() {
        let body = r#"{"error":{"message":"invalid key: top-secret"}}"#;
        let (base_url, server) = serve_one_response("401 Unauthorized", body);
        let client = OpenAiCompatibleClient::new(&base_url, "test-model", "top-secret").unwrap();

        let error = client.complete_json("test").unwrap_err().to_string();
        server.join().unwrap();
        assert!(error.contains("HTTP 401"));
        assert!(error.contains("[已隐藏]"));
        assert!(!error.contains("top-secret"));
    }

    #[test]
    fn invalid_references_and_duplicate_nodes_are_removed() {
        let records = vec![MemorySourceRecord {
            id: "r1".to_owned(),
            title: "t".to_owned(),
            project_id: None,
            project_name: None,
            imported_at: Utc::now().to_rfc3339(),
            analysis: None,
            transcript: vec![MemorySourceSegment {
                id: "s1".to_owned(),
                start_ms: 0,
                end_ms: 1,
                speaker_label: None,
                text: "quote".to_owned(),
            }],
        }];
        let source = MemorySourceReference {
            record_id: "missing".to_owned(),
            segment_id: Some("bad".to_owned()),
            start_ms: None,
            end_ms: None,
            quote_text: String::new(),
        };
        let node = MemoryNode {
            id: String::new(),
            node_type: "theme".to_owned(),
            label: "A".to_owned(),
            summary: String::new(),
            inferred: true,
            confidence: None,
            sources: vec![source],
        };
        let mut result = MemorySnapshotResult {
            nodes: vec![node.clone(), node],
            ..Default::default()
        };
        let warning = validate_and_normalize(&mut result, &records);
        assert!(warning.is_some());
        assert_eq!(result.nodes.len(), 1);
        assert!(result.nodes[0].sources.is_empty());
    }
}
