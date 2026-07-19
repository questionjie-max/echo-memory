//! 后端领域类型，与前端 src/shared/types.ts 一一对应。
//! 经 Tauri 的 serde_json 序列化，字段名保持一致（camelCase）。

use serde::{Deserialize, Serialize};

/// 应用元信息，由 `app_info` 命令返回。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub name: String,
    pub version: String,
    pub local_only: bool,
    pub generated_at: String, // UTC ISO-8601
}

/* ---------------- 领域模型（与「架构与数据.md」对齐，M1 起落地数据库） ---------------- */

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: String,
    pub name: String,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordBrief {
    pub id: String,
    pub title: String,
    pub project_id: Option<String>,
    pub project_name: Option<String>,
    pub audio_hash: String,
    pub audio_duration_ms: i64,
    pub imported_at: String,
    pub status: String,
    pub has_transcript: bool,
    pub has_analysis: bool,
    pub analysis_status: Option<String>,
    pub last_analysis_error: Option<String>,
    pub analysis_template_id: Option<String>,
    pub processing_stage: Option<String>,
    pub progress_current: i64,
    pub progress_total: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptSegment {
    pub id: String,
    pub record_id: String,
    pub sequence: i64,
    pub speaker_label: Option<String>,
    pub start_ms: i64,
    pub end_ms: i64,
    pub original_text: String,
    pub normalized_text: Option<String>,
    pub normalization_version: Option<String>,
    pub edited_text: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptBlock {
    pub id: String,
    pub segment_ids: Vec<String>,
    pub start_ms: i64,
    pub end_ms: i64,
    pub text: String,
    pub speaker_label: Option<String>,
    pub segments: Vec<TranscriptSegment>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptVersion {
    pub id: String,
    pub record_id: String,
    pub provider: String,
    pub model: String,
    pub status: String,
    pub language: String,
    pub pipeline_version: String,
    pub preprocessing_json: String,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct TranscriptSegmentInput {
    pub start_ms: i64,
    pub end_ms: i64,
    pub speaker_label: Option<String>,
    pub original_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredAnalysis {
    pub id: String,
    pub record_id: String,
    pub source_transcript_version_id: String,
    pub status: String,
    pub content_json: String,
    pub provider: String,
    pub model: String,
    pub template_version: String,
    pub template_id: Option<String>,
    pub template_snapshot_json: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateSection {
    pub key: String,
    pub title: String,
    pub format: String,
    pub instruction: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisTemplate {
    pub id: String,
    pub name: String,
    pub description: String,
    pub focus_instructions: String,
    pub custom_sections: Vec<TemplateSection>,
    pub is_builtin: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeSettings {
    pub transcription_language: String,
    pub whisper_model_path: String,
    pub analysis_model: String,
    pub embedding_model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalModelInfo {
    pub name: String,
    pub size: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalWhisperModel {
    pub id: String,
    pub path: String,
    pub size: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalAiStatus {
    pub whisper_available: bool,
    pub whisper_model_path: Option<String>,
    pub whisper_model_source: Option<String>,
    pub whisper_models: Vec<LocalWhisperModel>,
    pub ollama_available: bool,
    pub ollama_models: Vec<LocalModelInfo>,
    pub settings: KnowledgeSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelDownloadProgress {
    pub model: String,
    pub status: String,
    pub completed: Option<u64>,
    pub total: Option<u64>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeIndexStatus {
    pub scope_key: String,
    pub status: String,
    pub total_records: i64,
    pub processed_records: i64,
    pub chunk_count: i64,
    pub embedding_model: String,
    pub last_error: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeReference {
    pub record_id: String,
    pub record_title: String,
    pub text: String,
    pub quote_text: String,
    pub segment_id: Option<String>,
    pub start_ms: Option<i64>,
    pub end_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeOverview {
    pub record_count: i64,
    pub transcript_count: i64,
    pub analyzed_count: i64,
    pub decisions: Vec<KnowledgeReference>,
    pub action_items: Vec<KnowledgeReference>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeAnswerCitation {
    pub chunk_id: String,
    pub record_id: String,
    pub record_title: String,
    pub quote_text: String,
    pub segment_id: String,
    pub start_ms: i64,
    pub end_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeAnswer {
    pub answer: String,
    pub citations: Vec<KnowledgeAnswerCitation>,
    pub insufficient_evidence: bool,
}

#[derive(Debug, Clone)]
pub struct KnowledgeChunkInput {
    pub id: String,
    pub record_id: String,
    pub project_id: Option<String>,
    pub transcript_version_id: String,
    pub segment_ids: Vec<String>,
    pub body: String,
    pub start_ms: i64,
    pub end_ms: i64,
    pub speaker_label: Option<String>,
    pub content_hash: String,
    pub embedding_model: String,
    pub embedding: Vec<f32>,
}

#[derive(Debug, Clone)]
pub struct KnowledgeChunkRecord {
    pub id: String,
    pub record_id: String,
    pub record_title: String,
    pub project_id: Option<String>,
    pub segment_ids: Vec<String>,
    pub body: String,
    pub start_ms: i64,
    pub end_ms: i64,
    pub embedding_model: String,
    pub embedding: Vec<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    pub record_id: String,
    pub project_id: Option<String>,
    pub project_name: Option<String>,
    pub source_id: String,
    pub target_segment_id: Option<String>,
    pub source_type: String,
    pub title: String,
    pub snippet: String,
    pub imported_at: String,
    pub speaker_label: Option<String>,
    pub start_ms: Option<i64>,
    pub end_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpAccessLog {
    pub tool_name: String,
    pub record_id: Option<String>,
    pub project_id: Option<String>,
    pub called_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpStatus {
    pub enabled: bool,
    pub authorized_scope: String,
    pub executable_available: bool,
    pub executable_path: Option<String>,
    pub recent_calls: Vec<McpAccessLog>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionItem {
    pub id: String,
    pub record_id: String,
    pub project_id: String,
    pub title: String,
    pub status: String,
    pub source_segment_id: Option<String>,
    pub analysis_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisItem {
    pub id: String,
    pub record_id: String,
    pub text: String,
    pub citation_segment_ids: Vec<String>,
}

/// 处理任务（转写/分析），与 processing_jobs 表对应。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessingJob {
    pub id: String,
    pub record_id: String,
    pub job_type: String, // transcribe | analyze
    pub status: String,   // queued|preparing|transcribing|analyzing|completed|failed
    pub attempt_count: i64,
    pub last_error: Option<String>,
    pub stage: Option<String>,
    pub progress_current: i64,
    pub progress_total: i64,
    pub created_at: String,
    pub updated_at: String,
}

/// `import_audio` 命令的返回。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IngestResult {
    pub record_id: String,
    pub hash: String,
    pub duplicate: bool,
    pub duration_ms: i64,
    pub title: String,
}
