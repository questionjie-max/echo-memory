/**
 * 跨边界共享类型（前端 ↔ Rust 后端）
 * -------------------------------------------------------------
 * 这些接口与 src-tauri/src/types.rs 中的结构体一一对应，
 * 经由 Tauri 的 serde_json 序列化约定保持字段名一致。
 *
 * 统一约定（与「架构与数据.md」对齐）：
 *   - 所有 ID 为 UUID 字符串；
 *   - 所有时间为 UTC ISO-8601 字符串；
 *   - 音频位置统一用毫秒整数（number）。
 *
 * 前端是这些类型的唯一消费方与契约参照；后端结构体
 * 须通过 #[serde(rename_all = "camelCase")] 保持字段名一致。
 */

/* ---------------- 领域模型（M1 起落地数据库，此处仅定义契约） ---------------- */

export type RecordStatus =
  | "queued"
  | "preparing"
  | "transcribing"
  | "analyzing"
  | "completed"
  | "failed";

export type JobType = "transcribe" | "analyze";

export interface Project {
  id: string; // UUID
  name: string;
  status: "active" | "archived";
  createdAt: string; // UTC ISO-8601
  updatedAt: string; // UTC ISO-8601
}

export interface RecordBrief {
  id: string; // UUID
  title: string;
  projectId: string | null; // UUID
  projectName: string | null;
  /** import/recording 表示音频，document 表示导入的文字文档 */
  sourceType: string;
  audioHash: string; // SHA-256
  audioDurationMs: number; // 毫秒整数
  importedAt: string; // UTC ISO-8601
  status: RecordStatus;
  hasTranscript: boolean;
  hasAnalysis: boolean;
  analysisStatus: "completed" | "incomplete" | "stale" | null;
  lastAnalysisError: string | null;
  analysisTemplateId: string | null;
  processingStage: string | null;
  progressCurrent: number;
  progressTotal: number;
}

export interface TranscriptSegment {
  id: string; // UUID
  recordId: string; // UUID
  sequence: number;
  speakerLabel: string | null;
  startMs: number; // 毫秒整数
  endMs: number; // 毫秒整数
  originalText: string;
  normalizedText: string | null;
  normalizationVersion: string | null;
  editedText: string | null;
}

export interface TranscriptBlock {
  id: string;
  segmentIds: string[];
  startMs: number;
  endMs: number;
  text: string;
  speakerLabel: string | null;
  segments: TranscriptSegment[];
}

export interface TranscriptVersion {
  id: string;
  recordId: string;
  provider: string;
  model: string;
  status: string;
  language: string;
  pipelineVersion: string;
  preprocessingJson: string;
  createdAt: string;
}

export interface AudioPreprocessorStatus {
  enhancedAvailable: boolean;
  engine: string;
  executablePath: string | null;
  version: string | null;
}

export interface SearchResult {
  recordId: string;
  projectId: string | null;
  projectName: string | null;
  sourceId: string;
  targetSegmentId: string | null;
  sourceType: "title" | "transcript" | "analysis";
  title: string;
  snippet: string;
  importedAt: string;
  speakerLabel: string | null;
  startMs: number | null;
  endMs: number | null;
}

export interface AnalysisResultItem {
  text: string;
  citation_segment_ids?: string[];
  quote_text?: string;
  start_ms?: number;
  end_ms?: number;
}

export interface AnalysisContent {
  summary: string;
  key_points?: AnalysisResultItem[];
  decisions?: AnalysisResultItem[];
  action_items?: AnalysisResultItem[];
  open_questions?: AnalysisResultItem[];
  quality_warning?: string;
  custom_sections?: AnalysisCustomSection[];
}

export interface TemplateSection {
  key: string;
  title: string;
  format: "paragraph" | "list";
  instruction: string;
}

export interface AnalysisCustomSection {
  key: string;
  title: string;
  format: "paragraph" | "list";
  text?: string;
  items?: AnalysisResultItem[];
}

export interface AnalysisTemplate {
  id: string;
  name: string;
  description: string;
  focusInstructions: string;
  customSections: TemplateSection[];
  isBuiltin: boolean;
  createdAt: string;
  updatedAt: string;
}

export interface KnowledgeSettings {
  transcriptionLanguage: string;
  whisperModelPath: string;
  analysisModel: string;
  embeddingModel: string;
}

export interface LocalModelInfo { name: string; size: number; }
export interface LocalWhisperModel { id: string; path: string; size: number; }

export interface LocalAiStatus {
  whisperAvailable: boolean;
  whisperModelPath: string | null;
  whisperModelSource: string | null;
  whisperModels: LocalWhisperModel[];
  ollamaAvailable: boolean;
  ollamaModels: LocalModelInfo[];
  settings: KnowledgeSettings;
}

export interface ModelDownloadProgress {
  model: string;
  status: string;
  completed: number | null;
  total: number | null;
  error: string | null;
}

export interface KnowledgeIndexStatus {
  scopeKey: string;
  status: "not_built" | "stale" | "indexing" | "completed" | "failed";
  totalRecords: number;
  processedRecords: number;
  chunkCount: number;
  embeddingModel: string;
  lastError: string | null;
  updatedAt: string;
}

export interface KnowledgeReference {
  recordId: string;
  recordTitle: string;
  text: string;
  quoteText: string;
  segmentId: string | null;
  startMs: number | null;
  endMs: number | null;
}

export interface KnowledgeOverview {
  recordCount: number;
  transcriptCount: number;
  analyzedCount: number;
  decisions: KnowledgeReference[];
  actionItems: KnowledgeReference[];
}

export interface KnowledgeAnswerCitation {
  chunkId: string;
  recordId: string;
  recordTitle: string;
  quoteText: string;
  segmentId: string;
  startMs: number;
  endMs: number;
}

export interface KnowledgeAnswer {
  answer: string;
  citations: KnowledgeAnswerCitation[];
  insufficientEvidence: boolean;
}

export interface McpAccessLog {
  toolName: string;
  recordId: string | null;
  projectId: string | null;
  calledAt: string;
}

export interface McpStatus {
  enabled: boolean;
  authorizedScope: string;
  executableAvailable: boolean;
  executablePath: string | null;
  recentCalls: McpAccessLog[];
}

export interface StoredAnalysis {
  id: string;
  recordId: string;
  sourceTranscriptVersionId: string;
  status: string;
  contentJson: string;
  provider: string;
  model: string;
  templateVersion: string;
  templateId: string | null;
  templateSnapshotJson: string;
  createdAt: string;
}

export interface AnalysisItem {
  id: string; // UUID
  recordId: string; // UUID
  text: string;
  citationSegmentIds: string[]; // 引用到的片段 UUID
}

export type JobStatus =
  | "queued"
  | "preparing"
  | "transcribing"
  | "analyzing"
  | "completed"
  | "failed";

/** 处理任务，与后端 ProcessingJob / processing_jobs 表一一对应 */
export interface ProcessingJob {
  id: string; // UUID
  recordId: string; // UUID
  jobType: JobType; // transcribe | analyze
  status: JobStatus;
  attemptCount: number;
  lastError: string | null;
  stage: string | null;
  progressCurrent: number;
  progressTotal: number;
  createdAt: string; // UTC ISO-8601
  updatedAt: string; // UTC ISO-8601
}

/** import_audio / import_document 命令返回 */
export interface IngestResult {
  recordId: string; // UUID
  hash: string; // SHA-256
  duplicate: boolean; // 是否命中既有 audio_hash
  durationMs: number;
  title: string;
}

/* -------------------------- external memory views -------------------------- */

export interface ExternalAiSettings {
  enabled: boolean;
  baseUrl: string;
  model: string;
  hasApiKey: boolean;
  privacyConsentAt: string | null;
  /** Reserved only; cloud audio transcription is not implemented. */
  transcriptionProvider: string;
}

export type MemoryViewKind = "timeline" | "map" | "evolution";
export type MemoryGenerationStatus = "generating" | "completed" | "partial" | "failed" | "cancelled";

export interface MemoryScope {
  kind: "all" | "project" | "unfiled";
  projectId: string | null;
}

export interface MemorySourceReference {
  recordId: string;
  segmentId: string | null;
  startMs: number | null;
  endMs: number | null;
  quoteText: string;
}

export interface TimelineItem {
  id: string;
  occurredAt: string;
  itemType: string;
  title: string;
  summary: string;
  projectId: string | null;
  projectName: string | null;
  inferred: boolean;
  confidence: number | null;
  sources: MemorySourceReference[];
}

export interface MemoryNode {
  id: string;
  nodeType: string;
  label: string;
  summary: string;
  inferred: boolean;
  confidence: number | null;
  sources: MemorySourceReference[];
}

export interface MemoryEdge {
  id: string;
  sourceId: string;
  targetId: string;
  relation: string;
  inferred: boolean;
  confidence: number | null;
  sources: MemorySourceReference[];
}

export interface MemoryBranch {
  id: string;
  label: string;
  branchType: "project" | "theme" | "question" | "workflow" | string;
  projectId: string | null;
  parentId: string | null;
  order: number;
  inferred: boolean;
}

export interface GrowthNode {
  id: string;
  nodeType: string;
  label: string;
  summary: string;
  occurredAt: string;
  branchId: string;
  projectId: string | null;
  projectName: string | null;
  inferred: boolean;
  confidence: number | null;
  sources: MemorySourceReference[];
}

export interface GrowthEdge {
  id: string;
  sourceId: string;
  targetId: string;
  relation: string;
  inferred: boolean;
  confidence: number | null;
  sources: MemorySourceReference[];
}

export interface GrowthGraph {
  branches: MemoryBranch[];
  nodes: GrowthNode[];
  edges: GrowthEdge[];
  rangeStart: string | null;
  rangeEnd: string | null;
  generatedAt: string;
}

export interface EvolutionItem {
  id: string;
  topic: string;
  changeType: "新增" | "补充" | "修正" | "推翻" | "合并" | "验证" | string;
  beforeText: string;
  afterText: string;
  reason: string;
  occurredAt: string;
  inferred: boolean;
  confidence: number | null;
  sources: MemorySourceReference[];
}

export interface MemorySnapshotResult {
  timelineItems: TimelineItem[];
  nodes: MemoryNode[];
  edges: MemoryEdge[];
  evolutionItems: EvolutionItem[];
  dormantQuestions: string[];
  stalledProjects: string[];
}

export interface MemorySnapshot {
  id: string;
  viewKind: MemoryViewKind;
  scope: MemoryScope;
  rangeStart: string | null;
  rangeEnd: string | null;
  status: MemoryGenerationStatus;
  provider: string;
  model: string;
  sourceRecordIds: string[];
  requestHash: string;
  result: MemorySnapshotResult;
  qualityWarning: string | null;
  errorMessage: string | null;
  isStale: boolean;
  version: number;
  createdAt: string;
  updatedAt: string;
}

export interface MemoryFeedback {
  id: string;
  snapshotId: string;
  itemId: string;
  decision: "confirmed" | "rejected" | string;
  note: string;
  createdAt: string;
  updatedAt: string;
}

export interface MemoryGenerationRequest {
  generationId: string;
  viewKind: MemoryViewKind;
  scope: MemoryScope;
  rangeStart: string | null;
  rangeEnd: string | null;
}

export interface MemoryGenerationJob {
  generationId: string;
  snapshotId: string | null;
  status: MemoryGenerationStatus;
  errorMessage: string | null;
  startedAt: string;
  updatedAt: string;
}

/* ----------------------------- v0.3.0：收件箱 / 词汇库 / 仪表盘 ----------------------------- */

export interface InboxWatchFolder {
  id: string;
  path: string;
  label: string;
  enabled: boolean;
  createdAt: string;
}

export type InboxFileStatus =
  | "pending"
  | "importing"
  | "imported"
  | "duplicate"
  | "skipped"
  | "failed";

export interface InboxSeenFile {
  id: string;
  sourceKind: "folder" | "volume" | string;
  sourcePath: string;
  filePath: string;
  fileName: string;
  fileSize: number;
  mtimeMs: number;
  sha256: string | null;
  status: InboxFileStatus | string;
  recordId: string | null;
  errorMessage: string | null;
  seenAt: string;
  updatedAt: string;
}

export interface InboxCounts {
  pending: number;
  imported: number;
  failed: number;
}

export interface InboxStatus {
  usbDetection: boolean;
  watchFolders: InboxWatchFolder[];
  counts: InboxCounts;
  recentFiles: InboxSeenFile[];
}

export interface SuggestedWatchFolder {
  label: string;
  path: string;
}

export interface Hotword {
  id: string;
  term: string;
  note: string;
  createdAt: string;
}

export interface OnboardingStatus {
  completed: boolean;
  completedAt: string | null;
  whisperReady: boolean;
  whisperModelPath: string | null;
  ollamaRunning: boolean;
  analysisModelReady: boolean;
  analysisModel: string;
  watchFolderCount: number;
  usbDetection: boolean;
}

export interface ActionDashboardItem {
  id: string;
  recordId: string;
  recordTitle: string;
  projectId: string | null;
  projectName: string | null;
  title: string;
  ownerText: string;
  dueText: string;
  status: "open" | "done" | string;
  sourceSegmentId: string | null;
  importedAt: string;
}

export interface OpenQuestionItem {
  text: string;
  citationSegmentIds: string[];
  recordId: string;
  recordTitle: string;
  importedAt: string;
}

export interface ActionDashboard {
  openCount: number;
  doneCount: number;
  items: ActionDashboardItem[];
  openQuestions: OpenQuestionItem[];
}

export interface RelatedRecord {
  recordId: string;
  title: string;
  similarity: number;
  projectId: string | null;
}
