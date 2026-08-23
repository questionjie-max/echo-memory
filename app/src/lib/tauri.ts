import { invoke } from "@tauri-apps/api/core";
import type {
  AnalysisTemplate,
  AudioPreprocessorStatus,
  ExternalAiSettings,
  GrowthGraph,
  IngestResult,
  KnowledgeAnswer,
  KnowledgeIndexStatus,
  KnowledgeOverview,
  KnowledgeSettings,
  LocalAiStatus,
  McpStatus,
  MemoryFeedback,
  MemoryGenerationJob,
  MemoryGenerationRequest,
  MemoryScope,
  MemorySnapshot,
  MemoryViewKind,
  Project,
  RecordBrief,
  SearchResult,
  StoredAnalysis,
  TemplateSection,
  TimelineItem,
  TranscriptSegment,
  TranscriptBlock,
  InboxStatus,
  InboxWatchFolder,
  SuggestedWatchFolder,
  Hotword,
  OnboardingStatus,
  ActionDashboard,
  RelatedRecord,
  DockStatus,
  DockReply,
  DockMode,
  TemplateDraft,
  OutputStatus,
  AppInfo,
  SpeakerSummary,
  TranscriptionEngineStatus,
} from "../shared/types";

export function createProject(name: string): Promise<Project> {
  return invoke<Project>("create_project", { name });
}

export function listProjects(): Promise<Project[]> {
  return invoke<Project[]>("list_projects");
}

export function updateProject(
  id: string,
  patch: { name?: string; status?: "active" | "archived" },
): Promise<Project> {
  return invoke<Project>("update_project", {
    id,
    name: patch.name ?? null,
    status: patch.status ?? null,
  });
}

export function deleteProject(id: string): Promise<void> {
  return invoke<void>("delete_project", { id });
}

export function listAnalysisTemplates(): Promise<AnalysisTemplate[]> {
  return invoke<AnalysisTemplate[]>("list_analysis_templates");
}

export function createAnalysisTemplate(input: {
  name: string;
  description: string;
  focusInstructions: string;
  customSections: TemplateSection[];
}): Promise<AnalysisTemplate> {
  return invoke<AnalysisTemplate>("create_analysis_template", input);
}

export function updateAnalysisTemplate(id: string, input: {
  name: string;
  description: string;
  focusInstructions: string;
  customSections: TemplateSection[];
}): Promise<AnalysisTemplate> {
  return invoke<AnalysisTemplate>("update_analysis_template", { id, ...input });
}

export function deleteAnalysisTemplate(id: string): Promise<void> {
  return invoke<void>("delete_analysis_template", { id });
}

export function listRecords(projectId?: string | null, unfiledOnly = false): Promise<RecordBrief[]> {
  return invoke<RecordBrief[]>("list_records", { projectId: projectId ?? null, unfiledOnly });
}

export function getRecord(id: string): Promise<RecordBrief> {
  return invoke<RecordBrief>("get_record", { id });
}

export function searchRecords(query: string, projectId?: string | null, unfiledOnly = false): Promise<SearchResult[]> {
  return invoke<SearchResult[]>("search_records", { query, projectId: projectId ?? null, unfiledOnly });
}

export function getKnowledgeOverview(projectId?: string | null, unfiledOnly = false): Promise<KnowledgeOverview> {
  return invoke<KnowledgeOverview>("get_knowledge_overview", { projectId: projectId ?? null, unfiledOnly });
}

export function getKnowledgeIndexStatus(projectId?: string | null, unfiledOnly = false): Promise<KnowledgeIndexStatus> {
  return invoke<KnowledgeIndexStatus>("get_knowledge_index_status", { projectId: projectId ?? null, unfiledOnly });
}

export function rebuildKnowledgeIndex(projectId?: string | null, unfiledOnly = false): Promise<void> {
  return invoke<void>("rebuild_knowledge_index", { projectId: projectId ?? null, unfiledOnly });
}

export function askKnowledgeBase(question: string, projectId?: string | null, unfiledOnly = false): Promise<KnowledgeAnswer> {
  return invoke<KnowledgeAnswer>("ask_knowledge_base", { question, projectId: projectId ?? null, unfiledOnly });
}

export function updateRecordKnowledgeBase(recordId: string, knowledgeBaseId: string | null): Promise<RecordBrief> {
  return invoke<RecordBrief>("update_record_knowledge_base", { recordId, knowledgeBaseId });
}

export function updateRecordTitle(recordId: string, title: string): Promise<RecordBrief> {
  return invoke<RecordBrief>("update_record_title", { recordId, title });
}

export function getMcpStatus(): Promise<McpStatus> {
  return invoke<McpStatus>("get_mcp_status");
}

export function getLocalAiStatus(): Promise<LocalAiStatus> {
  return invoke<LocalAiStatus>("get_local_ai_status");
}

export function getAudioPreprocessorStatus(): Promise<AudioPreprocessorStatus> {
  return invoke<AudioPreprocessorStatus>("get_audio_preprocessor_status");
}

export function updateKnowledgeSettings(settings: KnowledgeSettings): Promise<KnowledgeSettings> {
  return invoke<KnowledgeSettings>("update_knowledge_settings", { settings });
}

export function pullOllamaModel(model: string): Promise<void> {
  return invoke<void>("pull_ollama_model", { model });
}

export function downloadWhisperModel(modelId: string): Promise<void> {
  return invoke<void>("download_whisper_model", { modelId });
}

export function setMcpEnabled(enabled: boolean): Promise<McpStatus> {
  return invoke<McpStatus>("set_mcp_enabled", { enabled });
}

export function importAudio(input: {
  sourcePath: string;
  projectId?: string | null;
  duplicateConfirmed: boolean;
}): Promise<IngestResult> {
  return invoke<IngestResult>("import_audio", {
    sourcePath: input.sourcePath,
    projectId: input.projectId ?? null,
    duplicateConfirmed: input.duplicateConfirmed,
  });
}

export function importDocument(input: {
  sourcePath: string;
  projectId?: string | null;
  duplicateConfirmed: boolean;
}): Promise<IngestResult> {
  return invoke<IngestResult>("import_document", {
    sourcePath: input.sourcePath,
    projectId: input.projectId ?? null,
    duplicateConfirmed: input.duplicateConfirmed,
  });
}

export function recordAudioPath(recordId: string): Promise<string> {
  return invoke<string>("record_audio_path", { recordId });
}

export function listTranscriptSegments(recordId: string): Promise<TranscriptSegment[]> {
  return invoke<TranscriptSegment[]>("list_transcript_segments", { recordId });
}

export function listTranscriptBlocks(recordId: string): Promise<TranscriptBlock[]> {
  return invoke<TranscriptBlock[]>("list_transcript_blocks", { recordId });
}

export function updateTranscriptSegment(segmentId: string, editedText: string | null): Promise<TranscriptSegment> {
  return invoke<TranscriptSegment>("update_transcript_segment", { segmentId, editedText });
}

export function transcribeRecord(recordId: string): Promise<void> {
  return invoke<void>("transcribe_record", { recordId });
}

export function retranscribeRecord(recordId: string, language: string, modelPath: string | null, qualityPreset = "enhanced", engine?: "embedded" | "whisperx" | null): Promise<void> {
  return invoke<void>("retranscribe_record", { recordId, language, modelPath, qualityPreset, engine: engine ?? null });
}

export function analyzeRecord(recordId: string, templateId?: string | null): Promise<void> {
  return invoke<void>("analyze_record", { recordId, templateId: templateId ?? null });
}

export function latestAnalysis(recordId: string): Promise<StoredAnalysis | null> {
  return invoke<StoredAnalysis | null>("latest_analysis", { recordId });
}

export function exportRecord(recordId: string, destinationPath: string, format: "md" | "txt"): Promise<string> {
  return invoke<string>("export_record", { recordId, destinationPath, format });
}

export function exportKnowledgeBase(projectId: string | null, unfiledOnly: boolean, destinationPath: string, format: "md" | "txt"): Promise<string> {
  return invoke<string>("export_knowledge_base", { projectId, unfiledOnly, destinationPath, format });
}


export function getExternalAiSettings(): Promise<ExternalAiSettings> {
  return invoke<ExternalAiSettings>("get_external_ai_settings");
}

export function updateExternalAiSettings(settings: ExternalAiSettings): Promise<ExternalAiSettings> {
  return invoke<ExternalAiSettings>("update_external_ai_settings", { settings });
}

export function setExternalAiApiKey(apiKey: string): Promise<ExternalAiSettings> {
  return invoke<ExternalAiSettings>("set_external_ai_api_key", { apiKey });
}

export function clearExternalAiApiKey(): Promise<ExternalAiSettings> {
  return invoke<ExternalAiSettings>("clear_external_ai_api_key");
}

export function testExternalAiConnection(): Promise<void> {
  return invoke<void>("test_external_ai_connection");
}

export function getLocalTimeline(scope: MemoryScope, rangeStart: string | null, rangeEnd: string | null): Promise<TimelineItem[]> {
  return invoke<TimelineItem[]>("get_local_timeline", { scope, rangeStart, rangeEnd });
}

export function getLocalGrowthGraph(scope: MemoryScope, rangeStart: string | null, rangeEnd: string | null): Promise<GrowthGraph> {
  return invoke<GrowthGraph>("get_local_growth_graph", { scope, rangeStart, rangeEnd });
}

export function generateMemorySnapshot(request: MemoryGenerationRequest): Promise<MemorySnapshot> {
  return invoke<MemorySnapshot>("generate_memory_snapshot", { request });
}

export function startMemoryGeneration(request: MemoryGenerationRequest): Promise<MemoryGenerationJob> {
  return invoke<MemoryGenerationJob>("start_memory_generation", { request });
}

export function getMemoryGenerationJob(generationId: string): Promise<MemoryGenerationJob> {
  return invoke<MemoryGenerationJob>("get_memory_generation_job", { generationId });
}

export function listMemorySnapshots(viewKind: MemoryViewKind, scope: MemoryScope, rangeStart: string | null, rangeEnd: string | null): Promise<MemorySnapshot[]> {
  return invoke<MemorySnapshot[]>("list_memory_snapshots", { viewKind, scope, rangeStart, rangeEnd });
}

export function getMemorySnapshot(snapshotId: string): Promise<MemorySnapshot> {
  return invoke<MemorySnapshot>("get_memory_snapshot", { snapshotId });
}

export function cancelMemoryGeneration(generationId: string): Promise<MemoryGenerationJob> {
  return invoke<MemoryGenerationJob>("cancel_memory_generation", { generationId });
}

export function updateMemoryFeedback(snapshotId: string, itemId: string, decision: string, note: string): Promise<MemoryFeedback> {
  return invoke<MemoryFeedback>("update_memory_feedback", { snapshotId, itemId, decision, note });
}

export function listMemoryFeedback(snapshotId: string): Promise<MemoryFeedback[]> {
  return invoke<MemoryFeedback[]>("list_memory_feedback", { snapshotId });
}

/* ------------------------------ v0.3.0：引导 / 收件箱 / 词汇库 / 仪表盘 ------------------------------ */

export function getOnboardingStatus(): Promise<OnboardingStatus> {
  return invoke<OnboardingStatus>("get_onboarding_status");
}

export function completeOnboarding(): Promise<void> {
  return invoke<void>("complete_onboarding");
}

export function resetOnboarding(): Promise<void> {
  return invoke<void>("reset_onboarding");
}

export function suggestWatchFolders(): Promise<SuggestedWatchFolder[]> {
  return invoke<SuggestedWatchFolder[]>("suggest_watch_folders");
}

export function getInboxStatus(): Promise<InboxStatus> {
  return invoke<InboxStatus>("get_inbox_status");
}

export function addInboxWatchFolder(path: string, label?: string): Promise<InboxWatchFolder> {
  return invoke<InboxWatchFolder>("add_inbox_watch_folder", { path, label: label ?? null });
}

export function removeInboxWatchFolder(id: string): Promise<void> {
  return invoke<void>("remove_inbox_watch_folder", { id });
}

export function setInboxUsbDetection(enabled: boolean): Promise<void> {
  return invoke<void>("set_inbox_usb_detection", { enabled });
}

export function rescanInbox(): Promise<void> {
  return invoke<void>("rescan_inbox");
}

export function listHotwords(): Promise<Hotword[]> {
  return invoke<Hotword[]>("list_hotwords");
}

export function addHotword(term: string, note?: string): Promise<Hotword> {
  return invoke<Hotword>("add_hotword", { term, note: note ?? null });
}

export function removeHotword(id: string): Promise<void> {
  return invoke<void>("remove_hotword", { id });
}

export function getActionDashboard(): Promise<ActionDashboard> {
  return invoke<ActionDashboard>("get_action_dashboard");
}

export function setActionItemStatus(id: string, status: "open" | "done"): Promise<void> {
  return invoke<void>("set_action_item_status", { id, status });
}

export function relatedRecords(recordId: string, limit?: number): Promise<RelatedRecord[]> {
  return invoke<RelatedRecord[]>("related_records", { recordId, limit: limit ?? null });
}

export function correctTranscript(recordId: string): Promise<void> {
  return invoke<void>("correct_transcript", { recordId });
}

export function getTranscriptCorrectionEnabled(): Promise<boolean> {
  return invoke<boolean>("get_transcript_correction_enabled");
}

export function setTranscriptCorrectionEnabled(enabled: boolean): Promise<void> {
  return invoke<void>("set_transcript_correction_enabled", { enabled });
}

/* ------------------------- v0.4.0：AI 伙伴 / 模板向导 / 产出文件夹 ------------------------- */

export function getDockStatus(): Promise<DockStatus> {
  return invoke<DockStatus>("get_dock_status");
}

export function askDock(options: {
  mode: DockMode;
  message: string;
  engine?: "local" | "external";
  recordId?: string | null;
}): Promise<DockReply> {
  return invoke<DockReply>("ask_dock", {
    mode: options.mode,
    message: options.message,
    engine: options.engine ?? null,
    recordId: options.recordId ?? null,
  });
}

export function clearDockChat(): Promise<void> {
  return invoke<void>("clear_dock_chat");
}

export function generateTemplateDraft(
  messages: Array<{ role: string; content: string }>,
): Promise<TemplateDraft> {
  return invoke<TemplateDraft>("generate_template_draft", { messages });
}

export function getOutputStatus(): Promise<OutputStatus> {
  return invoke<OutputStatus>("get_output_status");
}

export function setOutputFolder(path: string | null): Promise<OutputStatus> {
  return invoke<OutputStatus>("set_output_folder", { path });
}

export function setAutoExportAnalysis(enabled: boolean): Promise<OutputStatus> {
  return invoke<OutputStatus>("set_auto_export_analysis", { enabled });
}

export function exportRecordToOutput(recordId: string, kind: "analysis" | "transcript"): Promise<string> {
  return invoke<string>("export_record_to_output", { recordId, kind });
}

export function saveDockMessageToOutput(title: string, content: string): Promise<string> {
  return invoke<string>("save_dock_message_to_output", { title, content });
}

export function getAppInfo(): Promise<AppInfo> {
  return invoke<AppInfo>("app_info");
}

export function getTranscriptionEngineStatus(): Promise<TranscriptionEngineStatus> {
  return invoke<TranscriptionEngineStatus>("get_transcription_engine_status");
}

export function setTranscriptionEngine(engine: "embedded" | "whisperx"): Promise<void> {
  return invoke<void>("set_transcription_engine", { engine });
}

export function setHfToken(token: string): Promise<void> {
  return invoke<void>("set_hf_token", { token });
}

export function clearHfToken(): Promise<void> {
  return invoke<void>("clear_hf_token");
}

export function getRecordSpeakers(recordId: string): Promise<SpeakerSummary[]> {
  return invoke<SpeakerSummary[]>("get_record_speakers", { recordId });
}

export function renameRecordSpeaker(recordId: string, fromLabel: string, toLabel: string, addHotword: boolean): Promise<number> {
  return invoke<number>("rename_record_speaker", { recordId, fromLabel, toLabel, addHotword });
}
