//! 后端领域类型，与前端 src/shared/types.ts 一一对应。
//! 经 Tauri 的 serde_json 序列化，字段名保持一致（camelCase）。

use serde::{Deserialize, Serialize};

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
    pub source_type: String,
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

/* -------------------------- external memory views -------------------------- */

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalAiSettings {
    pub enabled: bool,
    pub base_url: String,
    pub model: String,
    pub has_api_key: bool,
    pub privacy_consent_at: Option<String>,
    /// Reserved for a future cloud transcription provider. Audio upload is not implemented.
    pub transcription_provider: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MemoryViewKind {
    Timeline,
    Map,
    Evolution,
}

impl MemoryViewKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Timeline => "timeline",
            Self::Map => "map",
            Self::Evolution => "evolution",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryScope {
    pub kind: String,
    pub project_id: Option<String>,
}

impl MemoryScope {
    pub fn key(&self) -> String {
        match self.kind.as_str() {
            "project" => format!("project:{}", self.project_id.as_deref().unwrap_or_default()),
            "unfiled" => "unfiled".to_owned(),
            _ => "all".to_owned(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MemoryGenerationStatus {
    Generating,
    Completed,
    Partial,
    Failed,
    Cancelled,
}

impl MemoryGenerationStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Generating => "generating",
            Self::Completed => "completed",
            Self::Partial => "partial",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MemorySourceReference {
    pub record_id: String,
    pub segment_id: Option<String>,
    pub start_ms: Option<i64>,
    pub end_ms: Option<i64>,
    pub quote_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineItem {
    pub id: String,
    pub occurred_at: String,
    pub item_type: String,
    pub title: String,
    pub summary: String,
    pub project_id: Option<String>,
    pub project_name: Option<String>,
    pub inferred: bool,
    pub confidence: Option<f64>,
    #[serde(default)]
    pub sources: Vec<MemorySourceReference>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryNode {
    pub id: String,
    pub node_type: String,
    pub label: String,
    pub summary: String,
    pub inferred: bool,
    pub confidence: Option<f64>,
    #[serde(default)]
    pub sources: Vec<MemorySourceReference>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryEdge {
    pub id: String,
    pub source_id: String,
    pub target_id: String,
    pub relation: String,
    pub inferred: bool,
    pub confidence: Option<f64>,
    #[serde(default)]
    pub sources: Vec<MemorySourceReference>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryBranch {
    pub id: String,
    pub label: String,
    pub branch_type: String,
    pub project_id: Option<String>,
    pub parent_id: Option<String>,
    pub order: i64,
    pub inferred: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GrowthNode {
    pub id: String,
    pub node_type: String,
    pub label: String,
    pub summary: String,
    pub occurred_at: String,
    pub branch_id: String,
    pub project_id: Option<String>,
    pub project_name: Option<String>,
    pub inferred: bool,
    pub confidence: Option<f64>,
    #[serde(default)]
    pub sources: Vec<MemorySourceReference>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GrowthEdge {
    pub id: String,
    pub source_id: String,
    pub target_id: String,
    pub relation: String,
    pub inferred: bool,
    pub confidence: Option<f64>,
    #[serde(default)]
    pub sources: Vec<MemorySourceReference>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GrowthGraph {
    pub branches: Vec<MemoryBranch>,
    pub nodes: Vec<GrowthNode>,
    pub edges: Vec<GrowthEdge>,
    pub range_start: Option<String>,
    pub range_end: Option<String>,
    pub generated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvolutionItem {
    pub id: String,
    pub topic: String,
    pub change_type: String,
    pub before_text: String,
    pub after_text: String,
    pub reason: String,
    pub occurred_at: String,
    pub inferred: bool,
    pub confidence: Option<f64>,
    #[serde(default)]
    pub sources: Vec<MemorySourceReference>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MemorySnapshotResult {
    #[serde(default)]
    pub timeline_items: Vec<TimelineItem>,
    #[serde(default)]
    pub nodes: Vec<MemoryNode>,
    #[serde(default)]
    pub edges: Vec<MemoryEdge>,
    #[serde(default)]
    pub evolution_items: Vec<EvolutionItem>,
    #[serde(default)]
    pub dormant_questions: Vec<String>,
    #[serde(default)]
    pub stalled_projects: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemorySnapshot {
    pub id: String,
    pub view_kind: MemoryViewKind,
    pub scope: MemoryScope,
    pub range_start: Option<String>,
    pub range_end: Option<String>,
    pub status: MemoryGenerationStatus,
    pub provider: String,
    pub model: String,
    pub source_record_ids: Vec<String>,
    pub request_hash: String,
    pub result: MemorySnapshotResult,
    pub quality_warning: Option<String>,
    pub error_message: Option<String>,
    pub is_stale: bool,
    pub version: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryFeedback {
    pub id: String,
    pub snapshot_id: String,
    pub item_id: String,
    pub decision: String,
    pub note: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryGenerationRequest {
    pub generation_id: String,
    pub view_kind: MemoryViewKind,
    pub scope: MemoryScope,
    pub range_start: Option<String>,
    pub range_end: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryGenerationJob {
    pub generation_id: String,
    pub snapshot_id: Option<String>,
    pub status: MemoryGenerationStatus,
    pub error_message: Option<String>,
    pub started_at: String,
    pub updated_at: String,
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

/* ----------------------------- v0.3.0：收件箱 / 词汇库 / 仪表盘 ----------------------------- */

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InboxWatchFolder {
    pub id: String,
    pub path: String,
    pub label: String,
    pub enabled: bool,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InboxSeenFile {
    pub id: String,
    pub source_kind: String, // folder | volume
    pub source_path: String,
    pub file_path: String,
    pub file_name: String,
    pub file_size: i64,
    pub mtime_ms: i64,
    pub sha256: Option<String>,
    pub status: String, // pending | importing | imported | duplicate | skipped | failed
    pub record_id: Option<String>,
    pub error_message: Option<String>,
    pub seen_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InboxCounts {
    pub pending: u32,
    pub imported: u32,
    pub failed: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InboxStatus {
    pub usb_detection: bool,
    pub watch_folders: Vec<InboxWatchFolder>,
    pub counts: InboxCounts,
    pub recent_files: Vec<InboxSeenFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Hotword {
    pub id: String,
    pub term: String,
    pub note: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnboardingStatus {
    pub completed: bool,
    pub completed_at: Option<String>,
    pub whisper_ready: bool,
    pub whisper_model_path: Option<String>,
    pub ollama_running: bool,
    pub analysis_model_ready: bool,
    pub analysis_model: String,
    pub watch_folder_count: u32,
    pub usb_detection: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionDashboardItem {
    pub id: String,
    pub record_id: String,
    pub record_title: String,
    pub project_id: Option<String>,
    pub project_name: Option<String>,
    pub title: String,
    pub owner_text: String,
    pub due_text: String,
    pub status: String,
    pub source_segment_id: Option<String>,
    pub imported_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenQuestionItem {
    pub text: String,
    pub citation_segment_ids: Vec<String>,
    pub record_id: String,
    pub record_title: String,
    pub imported_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionDashboard {
    pub open_count: u32,
    pub done_count: u32,
    pub items: Vec<ActionDashboardItem>,
    pub open_questions: Vec<OpenQuestionItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelatedRecord {
    pub record_id: String,
    pub title: String,
    pub similarity: f32,
    pub project_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SuggestedWatchFolder {
    pub label: String,
    pub path: String,
}

/* ----------------------------- v0.4.0：AI 伙伴 / 模板向导 / 产出文件夹 ----------------------------- */

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DockChat {
    pub id: String,
    pub title: String,
    pub engine: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DockMessage {
    pub id: String,
    pub chat_id: String,
    pub role: String,
    pub content: String,
    pub mode: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DockReply {
    pub chat: DockChat,
    pub user_message: DockMessage,
    pub assistant_message: DockMessage,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DockStatus {
    pub chat: Option<DockChat>,
    pub messages: Vec<DockMessage>,
    pub external_available: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateDraftSection {
    pub key: String,
    pub title: String,
    pub format: String, // paragraph | list
    pub instruction: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateDraft {
    pub name: String,
    pub description: String,
    pub sections: Vec<TemplateDraftSection>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputFile {
    pub file_name: String,
    pub path: String,
    pub size: u64,
    pub modified_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputStatus {
    pub folder: Option<String>,
    pub auto_export_analysis: bool,
    pub recent_files: Vec<OutputFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub version: String,
    pub library_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeakerSummary {
    pub label: String,
    pub segment_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptionEngineStatus {
    pub engine: String, // embedded | whisperx
    pub whisperx_available: bool,
    pub whisperx_path: Option<String>,
    pub hf_token_set: bool,
}
