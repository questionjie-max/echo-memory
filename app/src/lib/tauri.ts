import { invoke } from "@tauri-apps/api/core";
import type {
  AgentScanResult,
  AgentSessionSelection,
  AnalysisTemplate,
  AudioPreprocessorStatus,
  BulkExportResult,
  ChatImportOutcome,
  DeleteRecordsResult,
  RecordArtifact,
  DiskSpaceStatus,
  ExternalAiSettings,
  DiagnosisReport,
  ProviderHealthSnapshot,
  ProviderCatalog,
  RemoteModelList,
  SaveAndVerifyOutcome,
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
  OllamaInstallStatus,
  ProfileSnapshot,
  Project,
  RecordBrief,
  SearchResult,
  StorageCleanupResult,
  StorageReport,
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
  MicrophonePermission,
  RecordingState,
  StoppedRecording,
  RecoverableRecording,
  RecordingInputDevice,
  SystemAudioApplication,
  SystemAudioPermission,
  MinutesView,
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

export function listArchivedRecords(projectId?: string | null, unfiledOnly = false): Promise<RecordBrief[]> {
  return invoke<RecordBrief[]>("list_archived_records", { projectId: projectId ?? null, unfiledOnly });
}

export function setRecordArchived(recordId: string, archived: boolean): Promise<RecordBrief> {
  return invoke<RecordBrief>("set_record_archived", { recordId, archived });
}

/** 块4：导入内容确认门——确认后内容才对 MCP 可见，撤回立即收回。 */
export function setRecordImportConfirmed(recordId: string, confirmed: boolean): Promise<RecordBrief> {
  return invoke<RecordBrief>("set_record_import_confirmed", { recordId, confirmed });
}

/** 块7：导出包——单条记录全量交付（逐字稿 md+srt + 纪要 + manifest）。 */
export function exportRecordPackage(recordId: string, destinationPath: string, ticket: string): Promise<string> {
  return invoke<string>("export_record_package", { recordId, destinationPath, ticket });
}

/** v1.4 包7：批量导出包——每条记录一个子文件夹；单条失败不中断，如实回报。 */
export function exportRecordPackages(
  recordIds: string[],
  destinationPath: string,
  ticket: string,
): Promise<BulkExportResult> {
  return invoke<BulkExportResult>("export_record_packages", {
    recordIds,
    destinationPath,
    ticket,
  });
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

export function getMemoryIndexStatus(): Promise<KnowledgeIndexStatus> {
  return invoke<KnowledgeIndexStatus>("get_memory_index_status");
}

export function getMemoryProfile(): Promise<ProfileSnapshot> {
  return invoke<ProfileSnapshot>("get_memory_profile");
}

export function scanLocalAgents(): Promise<AgentScanResult> {
  return invoke<AgentScanResult>("scan_local_agents");
}

export function importAgentScans(input: {
  sessions: AgentSessionSelection[];
  memoryPaths: string[];
  projectId?: string | null;
  duplicateConfirmed: boolean;
}): Promise<ChatImportOutcome> {
  return invoke<ChatImportOutcome>("import_agent_scans", {
    sessions: input.sessions,
    memoryPaths: input.memoryPaths,
    projectId: input.projectId ?? null,
    duplicateConfirmed: input.duplicateConfirmed,
  });
}

export function chatArchiveCount(): Promise<number> {
  return invoke<number>("chat_archive_count");
}

export function generateChatDigest(): Promise<IngestResult> {
  return invoke<IngestResult>("generate_chat_digest");
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

/** 批量移动：把多条记录归到某个知识库，传 null 移出到「未归档」。 */
export function moveRecords(recordIds: string[], knowledgeBaseId: string | null): Promise<number> {
  return invoke<number>("move_records", { recordIds, knowledgeBaseId });
}

/** 批量删除：数据库事务提交后尽力清理文件，并返回遗留文件。 */
export function deleteRecords(recordIds: string[]): Promise<DeleteRecordsResult> {
  return invoke<DeleteRecordsResult>("delete_records", { recordIds });
}

export function getMcpStatus(): Promise<McpStatus> {
  return invoke<McpStatus>("get_mcp_status");
}

export function getLocalAiStatus(): Promise<LocalAiStatus> {
  return invoke<LocalAiStatus>("get_local_ai_status");
}

export function getOllamaInstallStatus(): Promise<OllamaInstallStatus> {
  return invoke<OllamaInstallStatus>("get_ollama_install_status");
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

/** 取消正在进行的模型下载（Whisper 与 Ollama 通用）。已下好的部分会保留，可以接着下。 */
export function cancelModelDownload(model: string): Promise<void> {
  return invoke<void>("cancel_model_download", { model });
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

export function importChatExports(input: {
  sourcePaths: string[];
  projectId?: string | null;
  duplicateConfirmed: boolean;
}): Promise<ChatImportOutcome> {
  return invoke<ChatImportOutcome>("import_chat_exports", {
    sourcePaths: input.sourcePaths,
    projectId: input.projectId ?? null,
    duplicateConfirmed: input.duplicateConfirmed,
  });
}

export function recordAudioPath(recordId: string): Promise<string> {
  return invoke<string>("record_audio_path", { recordId });
}

export function microphonePermission(): Promise<MicrophonePermission> {
  return invoke<MicrophonePermission>("microphone_permission");
}

export function requestMicrophonePermission(): Promise<MicrophonePermission> {
  return invoke<MicrophonePermission>("request_microphone_permission");
}

export function startRecording(
  projectId?: string | null,
  source?: "microphone" | "both",
  confirmed?: boolean,
  inputDeviceId?: string | null,
  applicationBundleId?: string | null,
): Promise<RecordingState> {
  return invoke<RecordingState>("start_recording", {
    projectId: projectId ?? null,
    source: source ?? "microphone",
    confirmed: confirmed ?? null,
    inputDeviceId: inputDeviceId || null,
    applicationBundleId: applicationBundleId || null,
  });
}

export function listMicrophoneDevices(): Promise<RecordingInputDevice[]> {
  return invoke<RecordingInputDevice[]>("list_microphone_devices");
}

export function pauseRecording(): Promise<RecordingState> {
  return invoke<RecordingState>("pause_recording");
}

export function resumeRecording(): Promise<RecordingState> {
  return invoke<RecordingState>("resume_recording");
}

export function recordingLevels(): Promise<RecordingState> {
  return invoke<RecordingState>("recording_levels");
}

export function stopRecording(): Promise<StoppedRecording> {
  return invoke<StoppedRecording>("stop_recording");
}

export function cancelRecording(): Promise<void> {
  return invoke<void>("cancel_recording");
}

export function getRecordingState(): Promise<RecordingState> {
  return invoke<RecordingState>("get_recording_state");
}

/** v1.4 包5：录音中打点——记录当前录音位置（不含暂停），返回已打点清单。 */
export function markRecording(): Promise<number[]> {
  return invoke<number[]>("mark_recording");
}

/** v1.4 包5：读取某条记录的打点标记（相对录音开始的毫秒升序）。 */
export function getRecordMarks(recordId: string): Promise<number[]> {
  return invoke<number[]>("get_record_marks", { recordId });
}

/** v1.4 包8：生成口播稿改写（与既有分析同一通道选择：外部启用+同意优先，否则本机）。 */
export function rewriteRecordScript(recordId: string): Promise<RecordArtifact> {
  return invoke<RecordArtifact>("rewrite_record_script", { recordId });
}

/** v1.4 包9：生成节目简介（show notes）。 */
export function generateShowNotes(recordId: string): Promise<RecordArtifact> {
  return invoke<RecordArtifact>("generate_show_notes", { recordId });
}

/** v1.4 包9：生成社媒文案。 */
export function generateSocialPost(recordId: string): Promise<RecordArtifact> {
  return invoke<RecordArtifact>("generate_social_post", { recordId });
}

/** v1.4 包8：列出某条记录的产物（口播稿/show notes/社媒文案）。 */
export function listRecordArtifacts(recordId: string): Promise<RecordArtifact[]> {
  return invoke<RecordArtifact[]>("list_record_artifacts", { recordId });
}

export function listRecoverableRecordings(): Promise<RecoverableRecording[]> {
  return invoke<RecoverableRecording[]>("list_recoverable_recordings");
}

export function recoverRecording(sessionId: string): Promise<StoppedRecording> {
  return invoke<StoppedRecording>("recover_recording", { sessionId });
}

export function discardRecoverableRecording(sessionId: string): Promise<void> {
  return invoke<void>("discard_recoverable_recording", { sessionId });
}

/* ---------- 系统内放声采集（v1.1） ---------- */

export function systemAudioPermission(): Promise<SystemAudioPermission> {
  return invoke<SystemAudioPermission>("system_audio_permission");
}

export function requestSystemAudioPermission(): Promise<SystemAudioPermission> {
  return invoke<SystemAudioPermission>("request_system_audio_permission");
}

export function startSystemRecording(
  projectId: string | null | undefined,
  confirmed: boolean,
  applicationBundleId?: string | null,
): Promise<RecordingState> {
  return invoke<RecordingState>("start_system_recording", {
    projectId: projectId ?? null,
    confirmed,
    applicationBundleId: applicationBundleId || null,
  });
}

export function listSystemAudioApplications(): Promise<SystemAudioApplication[]> {
  return invoke<SystemAudioApplication[]>("list_system_audio_applications");
}

export function pauseSystemRecording(): Promise<RecordingState> {
  return invoke<RecordingState>("pause_system_recording");
}

export function resumeSystemRecording(): Promise<RecordingState> {
  return invoke<RecordingState>("resume_system_recording");
}

export function systemRecordingLevels(): Promise<RecordingState> {
  return invoke<RecordingState>("system_recording_levels");
}

export function stopSystemRecording(): Promise<StoppedRecording> {
  return invoke<StoppedRecording>("stop_system_recording");
}

export function cancelSystemRecording(): Promise<void> {
  return invoke<void>("cancel_system_recording");
}

export function getSystemRecordingState(): Promise<RecordingState> {
  return invoke<RecordingState>("get_system_recording_state");
}

export function isDualRecording(): Promise<boolean> {
  return invoke<boolean>("is_dual_recording");
}

/* ---------- 会议纪要（v1.1 块3/块4） ---------- */

export function getRecordMinutes(recordId: string): Promise<MinutesView | null> {
  return invoke<MinutesView | null>("get_record_minutes", { recordId });
}

export function generateMinutes(recordId: string): Promise<MinutesView> {
  return invoke<MinutesView>("generate_minutes", { recordId });
}

export function setMinutesItemStatus(
  itemId: string,
  status: "provisional" | "confirmed",
): Promise<"provisional" | "confirmed"> {
  return invoke<"provisional" | "confirmed">("set_minutes_item_status", { itemId, status });
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

export function analyzeRecord(recordId: string, templateId?: string | null, instruction?: string | null): Promise<void> {
  return invoke<void>("analyze_record", {
    recordId,
    templateId: templateId ?? null,
    instruction: instruction && instruction.trim() ? instruction : null,
  });
}

export function latestAnalysis(recordId: string): Promise<StoredAnalysis | null> {
  return invoke<StoredAnalysis | null>("latest_analysis", { recordId });
}

/** 弹出保存对话框前先领一张一次性导出票据——后端只认走过对话框的导出。 */
export function createExportTicket(): Promise<string> {
  return invoke<string>("create_export_ticket");
}

export function exportRecord(
  recordId: string,
  destinationPath: string,
  format: "md" | "txt" | "srt",
  ticket: string,
): Promise<string> {
  return invoke<string>("export_record", { recordId, destinationPath, format, ticket });
}

export function exportKnowledgeBase(projectId: string | null, unfiledOnly: boolean, destinationPath: string, format: "md" | "txt", ticket: string): Promise<string> {
  return invoke<string>("export_knowledge_base", { projectId, unfiledOnly, destinationPath, format, ticket });
}


export function getExternalAiSettings(): Promise<ExternalAiSettings> {
  return invoke<ExternalAiSettings>("get_external_ai_settings");
}

export function updateExternalAiSettings(settings: ExternalAiSettings): Promise<ExternalAiSettings> {
  return invoke<ExternalAiSettings>("update_external_ai_settings", { settings });
}

export function getProviderCatalog(): Promise<ProviderCatalog> {
  return invoke<ProviderCatalog>("get_provider_catalog");
}

export function saveExternalCredentials(
  target: "analysis" | "transcription",
  providerId: string,
  credentials: Record<string, string>,
): Promise<ExternalAiSettings> {
  return invoke<ExternalAiSettings>("save_external_credentials", {
    target,
    providerId,
    credentials,
  });
}

export function clearExternalCredentials(
  target: "analysis" | "transcription",
): Promise<ExternalAiSettings> {
  return invoke<ExternalAiSettings>("clear_external_credentials", { target });
}

export function reuseExternalCredentials(
  target: "analysis" | "transcription",
): Promise<ExternalAiSettings> {
  return invoke<ExternalAiSettings>("reuse_external_credentials", { target });
}

/** 仅在用户明确点击“显示”时取回当前槽位的凭据。 */
export function revealExternalCredentials(
  target: "analysis" | "transcription",
  providerId: string,
): Promise<Record<string, string>> {
  return invoke<Record<string, string>>("reveal_external_credentials", {
    target,
    providerId,
  });
}

export function setExternalAiApiKey(apiKey: string): Promise<ExternalAiSettings> {
  return invoke<ExternalAiSettings>("set_external_ai_api_key", { apiKey });
}

export function clearExternalAiApiKey(): Promise<ExternalAiSettings> {
  return invoke<ExternalAiSettings>("clear_external_ai_api_key");
}

export function setExternalAsrApiKey(apiKey: string): Promise<ExternalAiSettings> {
  return invoke<ExternalAiSettings>("set_external_asr_api_key", { apiKey });
}

export function clearExternalAsrApiKey(): Promise<ExternalAiSettings> {
  return invoke<ExternalAiSettings>("clear_external_asr_api_key");
}

export function testExternalAiConnection(): Promise<void> {
  return invoke<void>("test_external_ai_connection");
}

/** 发送一段约 1 秒的测试音，真实验证第三方转写的凭据、地址和模型。 */
export function testExternalAsrConnection(): Promise<void> {
  return invoke<void>("test_external_asr_connection");
}

/** 用表单中的新 Key，或当前服务商的已保存 Key，拉取分组后的模型列表。 */
export function listExternalModels(
  target: "analysis" | "transcription",
  providerId: string,
  baseUrl: string,
  apiKey: string | null,
): Promise<RemoteModelList> {
  return invoke<RemoteModelList>("list_external_models", {
    target,
    providerId,
    baseUrl,
    apiKey,
  });
}

/** 草稿诊断：不落盘的真实小请求；通过时返回一次性验证票据。 */
export function diagnoseExternalProvider(input: {
  target: "analysis" | "transcription";
  providerId: string;
  baseUrl: string;
  model: string;
  credentials: Record<string, string>;
}): Promise<DiagnosisReport> {
  return invoke<DiagnosisReport>("diagnose_external_provider", input);
}

/** 保存并验证：先核销一次性票据再提交；验证失败原配置不变。 */
export function saveAndVerifyProvider(args: {
  target: "analysis" | "transcription";
  providerId: string;
  baseUrl: string;
  model: string;
  credentials: Record<string, string>;
  privacyConsentAt: string | null;
  audioUploadConsentAt: string | null;
  enabled: boolean;
  verificationToken: string;
}): Promise<SaveAndVerifyOutcome> {
  return invoke<SaveAndVerifyOutcome>("save_and_verify_provider", { args });
}

/** 验证当前已保存配置（结果写入健康快照，含失败）。 */
export function verifySavedProvider(
  target: "analysis" | "transcription",
): Promise<DiagnosisReport> {
  return invoke<DiagnosisReport>("verify_saved_provider", { target });
}

/** 健康快照（可能为 null = 尚未验证过）与凭据修订号。 */
export function getProviderHealthStatus(
  target: "analysis" | "transcription",
): Promise<ProviderHealthSnapshot | null> {
  return invoke<ProviderHealthSnapshot | null>("get_provider_health_status", { target });
}

export function getCredentialRevision(
  target: "analysis" | "transcription",
): Promise<number> {
  return invoke<number>("get_credential_revision", { target });
}

/** 拉取服务商的 OpenAI 兼容模型列表（保存前使用；Key 只在内存中传递）。 */
export function listRemoteModels(baseUrl: string, apiKey: string): Promise<string[]> {
  return invoke<string[]>("list_remote_models", { baseUrl, apiKey });
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

export interface MemoryFeedbackEntry {
  itemId: string;
  decision: string;
  note: string;
}

export function updateMemoryFeedbackBatch(snapshotId: string, entries: MemoryFeedbackEntry[]): Promise<MemoryFeedback[]> {
  return invoke<MemoryFeedback[]>("update_memory_feedback_batch", { snapshotId, entries });
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

export function getDiskSpaceStatus(): Promise<DiskSpaceStatus> {
  return invoke<DiskSpaceStatus>("get_disk_space_status");
}

export function getStorageReport(): Promise<StorageReport> {
  return invoke<StorageReport>("get_storage_report");
}

export function cleanupStorage(): Promise<StorageCleanupResult> {
  return invoke<StorageCleanupResult>("cleanup_storage");
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
