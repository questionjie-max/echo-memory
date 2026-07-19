import { invoke } from "@tauri-apps/api/core";
import type {
  AnalysisTemplate,
  AudioPreprocessorStatus,
  AppInfo,
  IngestResult,
  KnowledgeAnswer,
  KnowledgeIndexStatus,
  KnowledgeOverview,
  KnowledgeSettings,
  LocalAiStatus,
  McpStatus,
  Project,
  RecordBrief,
  SearchResult,
  StoredAnalysis,
  TemplateSection,
  TranscriptSegment,
  TranscriptBlock,
} from "../shared/types";

export function getAppInfo(): Promise<AppInfo> {
  return invoke<AppInfo>("app_info");
}

export function generateId(): Promise<string> {
  return invoke<string>("generate_id");
}

export function greet(name: string): Promise<string> {
  return invoke<string>("greet", { name });
}

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

export function retranscribeRecord(recordId: string, language: string, modelPath: string | null, qualityPreset = "enhanced"): Promise<void> {
  return invoke<void>("retranscribe_record", { recordId, language, modelPath, qualityPreset });
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
