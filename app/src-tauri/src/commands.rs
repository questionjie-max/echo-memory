use crate::analysis::{resolve_citation_aliases, verify_citations, OllamaAdapter};
use crate::audio::{
    is_overlap_duplicate, plan_chunks, preprocess, preprocessor_status, read_normalized_wav,
    AudioPreprocessorStatus,
};
use crate::db::repository::LibraryRepository;
use crate::error::AppResult;
use crate::library::ManagedLibrary;
use crate::memory::{self, OpenAiCompatibleClient};
use crate::state::AppState;
use crate::transcript::effective_text;
use crate::types::{
    AnalysisTemplate, AppInfo, ExternalAiSettings, GrowthGraph, IngestResult, KnowledgeAnswer,
    KnowledgeIndexStatus, KnowledgeOverview, KnowledgeSettings, LocalAiStatus, LocalModelInfo,
    LocalWhisperModel, McpStatus, MemoryFeedback, MemoryGenerationJob, MemoryGenerationRequest,
    MemoryScope, MemorySnapshot, MemoryViewKind, ModelDownloadProgress, ProcessingJob, Project,
    RecordBrief, SearchResult, TemplateSection, TimelineItem, TranscriptBlock, TranscriptSegment,
    TranscriptSegmentInput,
};
use crate::whisper::WhisperAdapter;
use chrono::{Duration as ChronoDuration, Local, SecondsFormat, TimeZone, Utc};
use sha2::{Digest, Sha256};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::time::Duration;
use tauri::{AppHandle, Emitter, State};

const LARGE_V3_TURBO_Q5_SIZE: u64 = 574_041_195;
const AUTO_MEMORY_UPDATE_DELAY: Duration = Duration::from_secs(120);
const LARGE_V3_TURBO_Q5_SHA256: &str =
    "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2";

#[tauri::command]
pub fn greet(name: String) -> String {
    format!("你好，{name}！欢迎使用回声记忆（本地优先）。")
}

#[tauri::command]
pub fn app_info() -> AppInfo {
    AppInfo {
        name: "回声记忆".to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        local_only: true,
        generated_at: Utc::now().to_rfc3339(),
    }
}

#[tauri::command]
pub fn generate_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

#[tauri::command]
pub fn get_local_ai_status(state: State<AppState>) -> Result<LocalAiStatus, String> {
    let settings = state
        .library
        .repository()
        .knowledge_settings()
        .map_err(|error| error.to_frontend())?;
    let whisper = WhisperAdapter::detect_with_model_path(Some(&settings.whisper_model_path)).ok();
    let whisper_model_path = whisper
        .as_ref()
        .and_then(WhisperAdapter::model_path)
        .map(|path| path.to_string_lossy().to_string());
    let whisper_model_source = whisper.as_ref().map(|adapter| {
        adapter.model_path().map_or_else(
            || "whisper.cpp CLI".to_owned(),
            |path| {
                if path
                    .to_string_lossy()
                    .contains("Application Support/com.meetily.ai")
                {
                    "Meetily 外部模型".to_owned()
                } else {
                    "用户选择的本地模型".to_owned()
                }
            },
        )
    });
    let (ollama_available, ollama_models) = match ollama_models() {
        Ok(models) => (true, models),
        Err(_) => (false, Vec::new()),
    };
    Ok(LocalAiStatus {
        whisper_available: whisper.is_some(),
        whisper_model_path,
        whisper_model_source,
        whisper_models: discover_whisper_models(&state.library, &settings.whisper_model_path),
        ollama_available,
        ollama_models,
        settings,
    })
}

fn discover_whisper_models(library: &ManagedLibrary, configured: &str) -> Vec<LocalWhisperModel> {
    let mut paths = Vec::new();
    if !configured.trim().is_empty() {
        paths.push(PathBuf::from(configured));
    }
    if let Some(home) = std::env::var_os("HOME") {
        paths.push(
            PathBuf::from(home)
                .join("Library/Application Support/com.meetily.ai/models/ggml-small.bin"),
        );
    }
    paths.push(library.root().join("models/ggml-large-v3-turbo-q5_0.bin"));
    let mut seen = std::collections::HashSet::new();
    paths
        .into_iter()
        .filter(|path| path.is_file() && seen.insert(path.clone()))
        .filter_map(|path| {
            Some(LocalWhisperModel {
                id: path.file_stem()?.to_string_lossy().to_string(),
                size: path.metadata().ok()?.len() as i64,
                path: path.to_string_lossy().to_string(),
            })
        })
        .collect()
}

#[tauri::command]
pub fn download_whisper_model(
    app: AppHandle,
    state: State<AppState>,
    model_id: String,
) -> Result<(), String> {
    if model_id != "large-v3-turbo-q5_0" {
        return Err("不支持的 Whisper 模型".to_owned());
    }
    let models_dir = state.library.root().join("models");
    std::thread::spawn(move || {
        let temporary = models_dir.join(".ggml-large-v3-turbo-q5_0.download");
        let result = (|| -> Result<(), String> {
            std::fs::create_dir_all(&models_dir).map_err(|error| error.to_string())?;
            let destination = models_dir.join("ggml-large-v3-turbo-q5_0.bin");
            if destination.is_file() {
                return Ok(());
            }
            let response = ureq::get("https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo-q5_0.bin")
                .call()
                .map_err(|error| format!("无法下载 Whisper 模型：{error}"))?;
            let total = response
                .header("content-length")
                .and_then(|value| value.parse::<u64>().ok());
            let mut reader = response.into_reader();
            let mut file = std::fs::File::create(&temporary).map_err(|error| error.to_string())?;
            let mut buffer = [0_u8; 64 * 1024];
            let mut completed = 0_u64;
            let mut hasher = Sha256::new();
            let mut header = Vec::with_capacity(4);
            loop {
                let count = reader
                    .read(&mut buffer)
                    .map_err(|error| error.to_string())?;
                if count == 0 {
                    break;
                }
                file.write_all(&buffer[..count])
                    .map_err(|error| error.to_string())?;
                if header.len() < 4 {
                    header.extend_from_slice(&buffer[..count.min(4 - header.len())]);
                }
                hasher.update(&buffer[..count]);
                completed += count as u64;
                let _ = app.emit(
                    "whisper-model-download-progress",
                    ModelDownloadProgress {
                        model: model_id.clone(),
                        status: "downloading".to_owned(),
                        completed: Some(completed),
                        total,
                        error: None,
                    },
                );
            }
            file.sync_all().map_err(|error| error.to_string())?;
            if completed != LARGE_V3_TURBO_Q5_SIZE
                || total.is_some_and(|expected| expected != LARGE_V3_TURBO_Q5_SIZE)
            {
                return Err("下载的 Whisper 模型文件不完整".to_owned());
            }
            if header.as_slice() != b"lmgg" {
                return Err("下载内容不是有效的 GGML Whisper 模型".to_owned());
            }
            let checksum = format!("{:x}", hasher.finalize());
            if checksum != LARGE_V3_TURBO_Q5_SHA256 {
                return Err("Whisper 模型 SHA-256 校验失败，临时文件已删除".to_owned());
            }
            let _ = app.emit(
                "whisper-model-download-progress",
                ModelDownloadProgress {
                    model: model_id.clone(),
                    status: format!("校验完成 · SHA-256 {}…", &checksum[..12]),
                    completed: Some(completed),
                    total: Some(completed),
                    error: None,
                },
            );
            std::fs::rename(&temporary, &destination).map_err(|error| error.to_string())?;
            Ok(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        let progress = match result {
            Ok(()) => ModelDownloadProgress {
                model: model_id.clone(),
                status: "completed".to_owned(),
                completed: None,
                total: None,
                error: None,
            },
            Err(error) => ModelDownloadProgress {
                model: model_id.clone(),
                status: "failed".to_owned(),
                completed: None,
                total: None,
                error: Some(error),
            },
        };
        let _ = app.emit("whisper-model-download-progress", progress);
    });
    Ok(())
}

#[tauri::command]
pub fn get_audio_preprocessor_status() -> AudioPreprocessorStatus {
    preprocessor_status()
}

#[tauri::command]
pub fn update_knowledge_settings(
    state: State<AppState>,
    settings: KnowledgeSettings,
) -> Result<KnowledgeSettings, String> {
    state
        .library
        .repository()
        .update_knowledge_settings(&settings)
        .map_err(|error| error.to_frontend())
}

#[tauri::command]
pub fn pull_ollama_model(app: AppHandle, model: String) -> Result<(), String> {
    let model = model.trim().to_owned();
    if model.is_empty() || model.len() > 120 {
        return Err("模型名称无效".to_owned());
    }
    std::thread::spawn(move || {
        let result = (|| -> Result<(), String> {
            let response = ureq::post("http://127.0.0.1:11434/api/pull")
                .send_json(serde_json::json!({ "model": model, "stream": true }))
                .map_err(|error| format!("无法启动模型下载：{error}"))?;
            for line in BufReader::new(response.into_reader()).lines() {
                let line = line.map_err(|error| format!("读取下载进度失败：{error}"))?;
                let value: serde_json::Value = serde_json::from_str(&line)
                    .map_err(|error| format!("下载进度格式无效：{error}"))?;
                if let Some(error) = value.get("error").and_then(serde_json::Value::as_str) {
                    return Err(error.to_owned());
                }
                let progress = ModelDownloadProgress {
                    model: model.clone(),
                    status: value
                        .get("status")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("下载中")
                        .to_owned(),
                    completed: value.get("completed").and_then(serde_json::Value::as_u64),
                    total: value.get("total").and_then(serde_json::Value::as_u64),
                    error: None,
                };
                let _ = app.emit("model-download-progress", progress);
            }
            Ok(())
        })();
        let progress = match result {
            Ok(()) => ModelDownloadProgress {
                model: model.clone(),
                status: "completed".to_owned(),
                completed: None,
                total: None,
                error: None,
            },
            Err(error) => ModelDownloadProgress {
                model: model.clone(),
                status: "failed".to_owned(),
                completed: None,
                total: None,
                error: Some(error),
            },
        };
        let _ = app.emit("model-download-progress", progress);
    });
    Ok(())
}

fn ollama_models() -> Result<Vec<LocalModelInfo>, String> {
    let response: serde_json::Value = ureq::get("http://127.0.0.1:11434/api/tags")
        .call()
        .map_err(|error| error.to_string())?
        .into_json()
        .map_err(|error| error.to_string())?;
    Ok(response
        .get("models")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| {
            Some(LocalModelInfo {
                name: item.get("name")?.as_str()?.to_owned(),
                size: item
                    .get("size")
                    .and_then(serde_json::Value::as_i64)
                    .unwrap_or(0),
            })
        })
        .collect())
}

#[tauri::command]
pub fn create_project(state: State<AppState>, name: String) -> Result<Project, String> {
    state
        .library
        .repository()
        .create_project(&name)
        .map_err(|e| e.to_frontend())
}

#[tauri::command]
pub fn list_projects(state: State<AppState>) -> Result<Vec<Project>, String> {
    state
        .library
        .repository()
        .list_projects()
        .map_err(|e| e.to_frontend())
}

#[tauri::command]
pub fn update_project(
    state: State<AppState>,
    id: String,
    name: Option<String>,
    status: Option<String>,
) -> Result<Project, String> {
    state
        .library
        .repository()
        .update_project(&id, name.as_deref(), status.as_deref())
        .map_err(|e| e.to_frontend())
}

#[tauri::command]
pub fn delete_project(state: State<AppState>, id: String) -> Result<(), String> {
    state
        .library
        .repository()
        .delete_project(&id)
        .map_err(|e| e.to_frontend())
}

#[tauri::command]
pub fn list_analysis_templates(state: State<AppState>) -> Result<Vec<AnalysisTemplate>, String> {
    state
        .library
        .repository()
        .list_analysis_templates()
        .map_err(|error| error.to_frontend())
}

#[tauri::command]
pub fn create_analysis_template(
    state: State<AppState>,
    name: String,
    description: String,
    focus_instructions: String,
    custom_sections: Vec<TemplateSection>,
) -> Result<AnalysisTemplate, String> {
    state
        .library
        .repository()
        .create_analysis_template(&name, &description, &focus_instructions, &custom_sections)
        .map_err(|error| error.to_frontend())
}

#[tauri::command]
pub fn update_analysis_template(
    state: State<AppState>,
    id: String,
    name: String,
    description: String,
    focus_instructions: String,
    custom_sections: Vec<TemplateSection>,
) -> Result<AnalysisTemplate, String> {
    state
        .library
        .repository()
        .update_analysis_template(
            &id,
            &name,
            &description,
            &focus_instructions,
            &custom_sections,
        )
        .map_err(|error| error.to_frontend())
}

#[tauri::command]
pub fn delete_analysis_template(state: State<AppState>, id: String) -> Result<(), String> {
    state
        .library
        .repository()
        .delete_analysis_template(&id)
        .map_err(|error| error.to_frontend())
}

#[tauri::command]
pub fn list_records(
    state: State<AppState>,
    project_id: Option<String>,
    unfiled_only: bool,
) -> Result<Vec<RecordBrief>, String> {
    state
        .library
        .repository()
        .list_records(project_id.as_deref(), unfiled_only)
        .map_err(|e| e.to_frontend())
}

#[tauri::command]
pub fn search_records(
    state: State<AppState>,
    query: String,
    project_id: Option<String>,
    unfiled_only: bool,
) -> Result<Vec<SearchResult>, String> {
    state
        .library
        .repository()
        .search(&query, project_id.as_deref(), unfiled_only, 50)
        .map_err(|e| e.to_frontend())
}

#[tauri::command]
pub fn get_knowledge_overview(
    state: State<AppState>,
    project_id: Option<String>,
    unfiled_only: bool,
) -> Result<KnowledgeOverview, String> {
    crate::knowledge::validate_scope(project_id.as_deref(), unfiled_only)
        .map_err(|error| error.to_frontend())?;
    state
        .library
        .repository()
        .knowledge_overview(project_id.as_deref(), unfiled_only)
        .map_err(|error| error.to_frontend())
}

#[tauri::command]
pub fn get_knowledge_index_status(
    state: State<AppState>,
    project_id: Option<String>,
    unfiled_only: bool,
) -> Result<KnowledgeIndexStatus, String> {
    crate::knowledge::validate_scope(project_id.as_deref(), unfiled_only)
        .map_err(|error| error.to_frontend())?;
    let settings = state
        .library
        .repository()
        .knowledge_settings()
        .map_err(|error| error.to_frontend())?;
    state
        .library
        .repository()
        .get_knowledge_index_status(
            &crate::knowledge::scope_key(project_id.as_deref(), unfiled_only),
            &settings.embedding_model,
        )
        .map_err(|error| error.to_frontend())
}

#[tauri::command]
pub fn rebuild_knowledge_index(
    state: State<AppState>,
    project_id: Option<String>,
    unfiled_only: bool,
) -> Result<(), String> {
    crate::knowledge::validate_scope(project_id.as_deref(), unfiled_only)
        .map_err(|error| error.to_frontend())?;
    let settings = state
        .library
        .repository()
        .knowledge_settings()
        .map_err(|error| error.to_frontend())?;
    let scope_key = crate::knowledge::scope_key(project_id.as_deref(), unfiled_only);
    let mut status = state
        .library
        .repository()
        .get_knowledge_index_status(&scope_key, &settings.embedding_model)
        .map_err(|error| error.to_frontend())?;
    if status.status == "indexing" {
        return Err("该范围的知识索引正在建立，请稍候".to_owned());
    }
    state
        .start_knowledge_index(&scope_key)
        .map_err(|error| error.to_frontend())?;
    status.status = "indexing".to_owned();
    status.last_error = None;
    status.updated_at = Utc::now().to_rfc3339();
    if let Err(error) = state
        .library
        .repository()
        .save_knowledge_index_status(&status)
    {
        state.finish_knowledge_index(&scope_key);
        return Err(error.to_frontend());
    }

    let library_root = state.library.root().to_path_buf();
    let embedding_model = settings.embedding_model;
    let failure_scope_key = scope_key.clone();
    let app_state = state.inner().clone();
    std::thread::spawn(move || {
        match ManagedLibrary::open(library_root.clone()) {
            Ok(library) => {
                if let Err(error) =
                    crate::knowledge::rebuild_scope(&library, project_id.as_deref(), unfiled_only)
                {
                    let _ = library.repository().mark_knowledge_index_failed(
                        &failure_scope_key,
                        &embedding_model,
                        &error.to_string(),
                    );
                }
            }
            Err(error) => {
                mark_knowledge_index_failed(
                    &library_root,
                    &failure_scope_key,
                    &embedding_model,
                    &error.to_string(),
                );
            }
        }
        app_state.finish_knowledge_index(&failure_scope_key);
    });
    Ok(())
}

#[tauri::command]
pub fn ask_knowledge_base(
    state: State<AppState>,
    project_id: Option<String>,
    unfiled_only: bool,
    question: String,
) -> Result<KnowledgeAnswer, String> {
    crate::knowledge::validate_scope(project_id.as_deref(), unfiled_only)
        .map_err(|error| error.to_frontend())?;
    crate::knowledge::ask(
        &state.library,
        project_id.as_deref(),
        unfiled_only,
        &question,
    )
    .map_err(|error| error.to_frontend())
}

#[tauri::command]
pub fn export_record(
    state: State<AppState>,
    record_id: String,
    destination_path: String,
    format: String,
) -> Result<String, String> {
    crate::export::export_record(
        &state.library,
        &record_id,
        &PathBuf::from(destination_path),
        &format,
    )
    .map(|path| path.to_string_lossy().to_string())
    .map_err(|error| error.to_frontend())
}

#[tauri::command]
pub fn export_knowledge_base(
    state: State<AppState>,
    project_id: Option<String>,
    unfiled_only: bool,
    destination_path: String,
    format: String,
) -> Result<String, String> {
    crate::knowledge::validate_scope(project_id.as_deref(), unfiled_only)
        .map_err(|error| error.to_frontend())?;
    crate::export::export_knowledge_base(
        &state.library,
        project_id.as_deref(),
        unfiled_only,
        &PathBuf::from(destination_path),
        &format,
    )
    .map(|path| path.to_string_lossy().to_string())
    .map_err(|error| error.to_frontend())
}

#[tauri::command]
pub fn update_record_knowledge_base(
    state: State<AppState>,
    record_id: String,
    knowledge_base_id: Option<String>,
) -> Result<RecordBrief, String> {
    let updated = state
        .library
        .repository()
        .update_record_project(&record_id, knowledge_base_id.as_deref())
        .map_err(|e| e.to_frontend())?;
    spawn_index_metadata_refresh(state.library.root().to_path_buf());
    schedule_memory_update(state.inner().clone());
    Ok(updated)
}

#[tauri::command]
pub fn update_record_title(
    state: State<AppState>,
    record_id: String,
    title: String,
) -> Result<RecordBrief, String> {
    let updated = state
        .library
        .repository()
        .update_record_title(&record_id, &title)
        .map_err(|e| e.to_frontend())?;
    schedule_memory_update(state.inner().clone());
    Ok(updated)
}

#[tauri::command]
pub fn get_mcp_status(state: State<AppState>) -> Result<McpStatus, String> {
    state
        .library
        .repository()
        .mcp_status()
        .map(decorate_mcp_status)
        .map_err(|e| e.to_frontend())
}

#[tauri::command]
pub fn set_mcp_enabled(state: State<AppState>, enabled: bool) -> Result<McpStatus, String> {
    state
        .library
        .repository()
        .set_mcp_enabled(enabled)
        .map(decorate_mcp_status)
        .map_err(|e| e.to_frontend())
}

fn decorate_mcp_status(mut status: McpStatus) -> McpStatus {
    if let Some(path) = find_mcp_executable() {
        status.executable_available = true;
        status.executable_path = Some(path.to_string_lossy().to_string());
    }
    status
}

fn find_mcp_executable() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(path) = std::env::var("ECHO_MCP_BIN") {
        candidates.push(PathBuf::from(path));
    }
    if let Ok(current) = std::env::current_exe() {
        if let Some(macos_dir) = current.parent() {
            candidates.push(macos_dir.join("echo-memory-mcp"));
            if let Some(contents_dir) = macos_dir.parent() {
                candidates.push(contents_dir.join("Resources").join("echo-memory-mcp"));
            }
        }
    }
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    candidates.push(manifest_dir.join("target/debug/echo-memory-mcp"));
    candidates.push(manifest_dir.join("target/release/echo-memory-mcp"));
    candidates.into_iter().find(|path| path.is_file())
}

#[tauri::command]
pub fn get_record(state: State<AppState>, id: String) -> Result<RecordBrief, String> {
    state
        .library
        .repository()
        .get_record(&id)
        .map_err(|e| e.to_frontend())
}

#[tauri::command]
pub fn record_audio_path(state: State<AppState>, record_id: String) -> Result<String, String> {
    let relative = state
        .library
        .repository()
        .audio_path_for_record(&record_id)
        .map_err(|e| e.to_frontend())?;
    Ok(state
        .library
        .root()
        .join(relative)
        .to_string_lossy()
        .to_string())
}

#[tauri::command]
pub fn list_jobs(state: State<AppState>, record_id: String) -> Result<Vec<ProcessingJob>, String> {
    state
        .library
        .repository()
        .list_jobs_for_record(&record_id)
        .map_err(|e| e.to_frontend())
}

#[tauri::command]
pub fn import_audio(
    state: State<AppState>,
    source_path: String,
    project_id: Option<String>,
    duplicate_confirmed: bool,
) -> Result<IngestResult, String> {
    state
        .library
        .import_audio(
            &PathBuf::from(source_path),
            project_id.as_deref(),
            duplicate_confirmed,
        )
        .map_err(|e| e.to_frontend())
}

#[tauri::command]
pub fn import_document(
    state: State<AppState>,
    source_path: String,
    project_id: Option<String>,
    duplicate_confirmed: bool,
) -> Result<IngestResult, String> {
    let result = crate::document::import_document(
        &state.library,
        &PathBuf::from(source_path),
        project_id.as_deref(),
        duplicate_confirmed,
    )
    .map_err(|error| error.to_frontend())?;
    if !result.duplicate {
        let library_root = state.library.root().to_path_buf();
        let record_id = result.record_id.clone();
        let app_state = state.inner().clone();
        spawn_incremental_index(library_root.clone(), record_id.clone());
        std::thread::spawn(move || match ManagedLibrary::open(library_root.clone()) {
            Ok(library) => {
                if analyze_with_library(&library, &record_id).is_ok() {
                    schedule_memory_update(app_state);
                }
            }
            Err(error) => {
                mark_processing_failed(
                    &library_root,
                    &record_id,
                    None,
                    "analyze",
                    &error.to_string(),
                );
            }
        });
    }
    Ok(result)
}

#[tauri::command]
pub fn list_transcript_segments(
    state: State<AppState>,
    record_id: String,
) -> Result<Vec<TranscriptSegment>, String> {
    state
        .library
        .repository()
        .list_transcript_segments(&record_id)
        .map_err(|e| e.to_frontend())
}

#[tauri::command]
pub fn list_transcript_blocks(
    state: State<AppState>,
    record_id: String,
) -> Result<Vec<TranscriptBlock>, String> {
    state
        .library
        .repository()
        .list_transcript_blocks(&record_id)
        .map_err(|error| error.to_frontend())
}

#[tauri::command]
pub fn update_transcript_segment(
    state: State<AppState>,
    segment_id: String,
    edited_text: Option<String>,
) -> Result<TranscriptSegment, String> {
    let segment = state
        .library
        .repository()
        .update_segment_text(&segment_id, edited_text.as_deref())
        .map_err(|e| e.to_frontend())?;
    spawn_incremental_index(
        state.library.root().to_path_buf(),
        segment.record_id.clone(),
    );
    schedule_memory_update(state.inner().clone());
    Ok(segment)
}

#[tauri::command]
pub fn transcribe_record(state: State<AppState>, record_id: String) -> Result<(), String> {
    let job = reserve_transcription(&state.library, &record_id).map_err(|e| e.to_frontend())?;
    let settings = state
        .library
        .repository()
        .knowledge_settings()
        .map_err(|error| error.to_frontend())?;
    let library_root = state.library.root().to_path_buf();
    let app_state = state.inner().clone();
    std::thread::spawn(move || match ManagedLibrary::open(library_root.clone()) {
        Ok(library) => {
            if run_transcription(
                &library,
                &record_id,
                &job,
                &settings.transcription_language,
                Some(&settings.whisper_model_path),
            )
            .is_ok()
            {
                schedule_memory_update(app_state);
            }
        }
        Err(error) => {
            mark_processing_failed(
                &library_root,
                &record_id,
                Some(&job.id),
                "transcribe",
                &error.to_string(),
            );
        }
    });
    Ok(())
}

/// Runs the local ASR job and persists its normalized segments.
/// Keeping this outside the Tauri wrapper makes the full import-to-database path testable.
pub fn transcribe_with_library(library: &ManagedLibrary, record_id: &str) -> AppResult<()> {
    let job = reserve_transcription(library, record_id)?;
    let settings = library.repository().knowledge_settings()?;
    run_transcription(
        library,
        record_id,
        &job,
        &settings.transcription_language,
        Some(&settings.whisper_model_path),
    )
}

#[tauri::command]
pub fn retranscribe_record(
    state: State<AppState>,
    record_id: String,
    language: String,
    model_path: Option<String>,
    quality_preset: Option<String>,
) -> Result<(), String> {
    let language = language.trim().to_owned();
    if language.is_empty() || language.len() > 16 {
        return Err("转写语言无效".to_owned());
    }
    if let Some(path) = model_path
        .as_deref()
        .map(str::trim)
        .filter(|path| !path.is_empty())
    {
        if !PathBuf::from(path).is_file() {
            return Err("选择的 Whisper 模型文件不存在".to_owned());
        }
    }
    if quality_preset.as_deref().unwrap_or("enhanced") != "enhanced" {
        return Err("转写质量预设无效".to_owned());
    }
    let repository = state.library.repository();
    if repository
        .list_jobs_for_record(&record_id)
        .map_err(|error| error.to_frontend())?
        .iter()
        .any(|job| {
            job.job_type == "transcribe"
                && matches!(job.status.as_str(), "preparing" | "transcribing")
        })
    {
        return Err("该录音正在转写".to_owned());
    }
    let job = repository
        .create_job(&record_id, "transcribe")
        .and_then(|job| {
            repository.update_job_status(&job.id, "preparing", None)?;
            repository.update_record_status(&record_id, "preparing")?;
            repository.get_job(&job.id)
        })
        .map_err(|error| error.to_frontend())?;
    let library_root = state.library.root().to_path_buf();
    let app_state = state.inner().clone();
    std::thread::spawn(move || match ManagedLibrary::open(library_root.clone()) {
        Ok(library) => {
            if run_transcription(&library, &record_id, &job, &language, model_path.as_deref())
                .is_ok()
            {
                schedule_memory_update(app_state);
            }
        }
        Err(error) => {
            mark_processing_failed(
                &library_root,
                &record_id,
                Some(&job.id),
                "transcribe",
                &error.to_string(),
            );
        }
    });
    Ok(())
}

fn reserve_transcription(library: &ManagedLibrary, record_id: &str) -> AppResult<ProcessingJob> {
    let repository = library.repository();
    let job = repository
        .list_jobs_for_record(record_id)?
        .into_iter()
        .find(|job| {
            job.job_type == "transcribe" && matches!(job.status.as_str(), "queued" | "failed")
        })
        .ok_or_else(|| crate::error::AppError::Invalid("没有可重试的转写任务".to_owned()))?;
    repository.update_job_status(&job.id, "preparing", None)?;
    repository.update_record_status(record_id, "preparing")?;
    Ok(job)
}

fn run_transcription(
    library: &ManagedLibrary,
    record_id: &str,
    job: &ProcessingJob,
    language: &str,
    model_path: Option<&str>,
) -> AppResult<()> {
    let repository = library.repository();
    let preprocessed = library
        .root()
        .join("raw")
        .join(record_id)
        .join(format!("{}-enhanced.wav", job.id));
    let result = (|| {
        let adapter = WhisperAdapter::detect_with_model_path(model_path)?;
        let relative_audio = repository.audio_path_for_record(record_id)?;
        let audio_path = library.root().join(relative_audio);
        let output_dir = library.root().join("raw").join(record_id);
        std::fs::create_dir_all(&output_dir)?;
        repository.update_job_progress(&job.id, "preprocessing", 0, 1)?;
        let metadata = preprocess(&audio_path, &preprocessed)?;
        repository.update_job_progress(&job.id, "chunking", 1, 1)?;
        let samples = read_normalized_wav(&preprocessed)?;
        let chunks = plan_chunks(&samples);
        repository.update_job_status(&job.id, "transcribing", None)?;
        repository.update_record_status(record_id, "transcribing")?;
        let mut accepted = Vec::new();
        let mut prompt = String::new();
        for (index, chunk) in chunks.iter().enumerate() {
            repository.update_job_progress(
                &job.id,
                "transcribing",
                index as i64,
                chunks.len() as i64,
            )?;
            let offset_ms = chunk.sample_start as i64 * 1_000 / 16_000;
            let chunk_segments = adapter.transcribe_samples(
                &samples[chunk.sample_start..chunk.sample_end],
                language,
                (!prompt.is_empty()).then_some(prompt.as_str()),
            )?;
            for mut segment in chunk_segments {
                segment.start_ms += offset_ms;
                segment.end_ms += offset_ms;
                let midpoint = segment.start_ms + (segment.end_ms - segment.start_ms) / 2;
                if midpoint >= chunk.accept_start_ms && midpoint <= chunk.accept_end_ms {
                    let duplicate =
                        accepted
                            .last()
                            .is_some_and(|previous: &crate::whisper::WhisperSegment| {
                                is_overlap_duplicate(previous, &segment)
                            });
                    if !duplicate {
                        accepted.push(segment);
                    }
                }
            }
            prompt = accepted
                .iter()
                .rev()
                .flat_map(|segment| segment.text.chars().rev())
                .take(80)
                .collect::<String>()
                .chars()
                .rev()
                .collect();
        }
        repository.update_job_progress(
            &job.id,
            "merging",
            chunks.len() as i64,
            chunks.len() as i64,
        )?;
        let inputs = accepted
            .into_iter()
            .map(|segment| TranscriptSegmentInput {
                start_ms: segment.start_ms,
                end_ms: segment.end_ms,
                speaker_label: Some("未知".to_owned()),
                original_text: segment.text,
            })
            .collect::<Vec<_>>();
        if inputs.is_empty() {
            return Err(crate::error::AppError::Import(
                "Whisper 没有生成可用片段".to_owned(),
            ));
        }
        repository.update_job_progress(&job.id, "normalizing", 1, 1)?;
        let model = adapter.model_name();
        let metadata_json = serde_json::to_string(&metadata)
            .map_err(|error| crate::error::AppError::Import(format!("预处理信息无效：{error}")))?;
        repository.update_job_progress(&job.id, "saving", 1, 1)?;
        repository.save_transcript_with_metadata(
            record_id,
            "whisper.cpp",
            &model,
            language,
            "enhanced-v2",
            &metadata_json,
            &inputs,
        )?;
        repository.update_job_status(&job.id, "completed", None)?;
        repository.update_record_status(record_id, "completed")?;
        spawn_incremental_index(library.root().to_path_buf(), record_id.to_owned());
        Ok::<(), crate::error::AppError>(())
    })();
    let _ = std::fs::remove_file(&preprocessed);
    if let Err(error) = result {
        let message = error.to_string();
        let _ = repository.update_job_status(&job.id, "failed", Some(&message));
        let fallback_status = if repository.latest_transcript_version(record_id).is_ok() {
            "completed"
        } else {
            "failed"
        };
        let _ = repository.update_record_status(record_id, fallback_status);
        return Err(error);
    }
    Ok(())
}

fn mark_processing_failed(
    library_root: &PathBuf,
    record_id: &str,
    job_id: Option<&str>,
    job_type: &str,
    error: &str,
) {
    let Ok(repository) = LibraryRepository::new(library_root.join("memory.db")) else {
        return;
    };
    let job = job_id
        .and_then(|id| repository.get_job(id).ok())
        .or_else(|| {
            repository
                .list_jobs_for_record(record_id)
                .ok()?
                .into_iter()
                .rev()
                .find(|job| job.job_type == job_type && job.status != "completed")
        })
        .or_else(|| repository.create_job(record_id, job_type).ok());
    if let Some(job) = job {
        let _ = repository.update_job_status(&job.id, "failed", Some(error));
    }
    let _ = repository.update_record_status(record_id, "failed");
}

fn mark_knowledge_index_failed(
    library_root: &PathBuf,
    scope_key: &str,
    embedding_model: &str,
    error: &str,
) {
    let Ok(repository) = LibraryRepository::new(library_root.join("memory.db")) else {
        return;
    };
    let _ = repository.mark_knowledge_index_failed(scope_key, embedding_model, error);
}

fn mark_knowledge_indexes_failed(library_root: &PathBuf, error: &str) {
    let Ok(repository) = LibraryRepository::new(library_root.join("memory.db")) else {
        return;
    };
    let _ = repository.mark_knowledge_indexes_failed(error);
}

fn spawn_incremental_index(library_root: PathBuf, record_id: String) {
    std::thread::spawn(move || match ManagedLibrary::open(library_root.clone()) {
        Ok(library) => {
            if let Err(error) = crate::knowledge::incremental_rebuild_record(&library, &record_id) {
                let _ = library
                    .repository()
                    .mark_knowledge_indexes_failed(&error.to_string());
            }
        }
        Err(error) => {
            mark_knowledge_indexes_failed(&library_root, &error.to_string());
        }
    });
}

fn spawn_index_metadata_refresh(library_root: PathBuf) {
    std::thread::spawn(move || {
        if let Ok(library) = ManagedLibrary::open(library_root) {
            let repository = library.repository();
            if let Ok(settings) = repository.knowledge_settings() {
                let _ = repository.refresh_knowledge_index_counts(&settings.embedding_model);
            }
        }
    });
}

#[tauri::command]
pub fn analyze_record(
    state: State<AppState>,
    record_id: String,
    template_id: Option<String>,
) -> Result<(), String> {
    if let Some(template_id) = template_id.as_deref() {
        state
            .library
            .repository()
            .set_record_analysis_template(&record_id, template_id)
            .map_err(|error| error.to_frontend())?;
    }
    let job = reserve_analysis(&state.library, &record_id).map_err(|e| e.to_frontend())?;
    let library_root = state.library.root().to_path_buf();
    let app_state = state.inner().clone();
    std::thread::spawn(move || match ManagedLibrary::open(library_root.clone()) {
        Ok(library) => {
            if run_analysis(&library, &record_id, &job).is_ok() {
                schedule_memory_update(app_state);
            }
        }
        Err(error) => {
            mark_processing_failed(
                &library_root,
                &record_id,
                Some(&job.id),
                "analyze",
                &error.to_string(),
            );
        }
    });
    Ok(())
}

/// Runs the same local analysis path synchronously for explicit real-data regression tests.
pub fn analyze_with_library(library: &ManagedLibrary, record_id: &str) -> AppResult<()> {
    let job = reserve_analysis(library, record_id)?;
    run_analysis(library, record_id, &job)
}

pub fn revalidate_analysis_with_library(
    library: &ManagedLibrary,
    analysis_id: &str,
) -> AppResult<crate::types::StoredAnalysis> {
    let repository = library.repository();
    let stored = repository.get_analysis(analysis_id)?;
    let segments = repository.list_transcript_segments(&stored.record_id)?;
    let mut draft: crate::analysis::AnalysisDraft = serde_json::from_str(&stored.content_json)
        .map_err(|error| crate::error::AppError::Analysis(format!("分析 JSON 无效: {error}")))?;
    draft.quality_warning = None;
    resolve_citation_aliases(&mut draft, &segments);
    let verified = verify_citations(&draft, &segments);
    if verified.quality_warning.is_some() {
        return Err(crate::error::AppError::Analysis(
            "仍有分析条目无法匹配可靠出处".to_owned(),
        ));
    }
    let template = stored
        .template_id
        .as_deref()
        .and_then(|id| repository.get_analysis_template(id).ok())
        .or_else(|| serde_json::from_str(&stored.template_snapshot_json).ok())
        .unwrap_or(repository.get_analysis_template("builtin-standard")?);
    repository.save_analysis_with_template(
        &stored.record_id,
        &stored.source_transcript_version_id,
        &stored.model,
        &verified,
        &template,
    )
}

fn reserve_analysis(library: &ManagedLibrary, record_id: &str) -> AppResult<ProcessingJob> {
    let repository = library.repository();
    let jobs = repository.list_jobs_for_record(record_id)?;
    if jobs
        .iter()
        .any(|job| job.job_type == "analyze" && job.status == "analyzing")
    {
        return Err(crate::error::AppError::Invalid(
            "该录音正在进行本地分析".to_owned(),
        ));
    }
    let job = match jobs
        .into_iter()
        .rev()
        .find(|job| job.job_type == "analyze" && job.status == "failed")
    {
        Some(job) => job,
        None => repository.create_job(record_id, "analyze")?,
    };
    repository.update_job_status(&job.id, "analyzing", None)?;
    repository.update_record_status(record_id, "analyzing")?;
    Ok(job)
}

fn run_analysis(library: &ManagedLibrary, record_id: &str, job: &ProcessingJob) -> AppResult<()> {
    let repository = library.repository();
    let result = (|| {
        repository.update_job_progress(&job.id, "analyzing", 0, 1)?;
        let version = repository.latest_transcript_version(record_id)?;
        let segments = repository.list_transcript_segments(record_id)?;
        let transcript = segments
            .iter()
            .map(|segment| format!("[{}] {}", segment.sequence, effective_text(segment)))
            .collect::<Vec<_>>()
            .join("\n");
        let record = repository.get_record(record_id)?;
        let template = repository.get_analysis_template(
            record
                .analysis_template_id
                .as_deref()
                .unwrap_or("builtin-standard"),
        )?;
        let model = repository.knowledge_settings()?.analysis_model;
        let mut draft = OllamaAdapter::detect(&model)?.analyze_with_template_progress(
            &transcript,
            Some(&template),
            |current, total| {
                repository.update_job_progress(
                    &job.id,
                    "analyzing",
                    current as i64,
                    total as i64,
                )?;
                Ok(())
            },
        )?;
        repository.update_job_progress(&job.id, "validating", 1, 1)?;
        resolve_citation_aliases(&mut draft, &segments);
        let verified = verify_citations(&draft, &segments);
        repository.update_job_progress(&job.id, "saving", 1, 1)?;
        repository.save_analysis_with_template(
            record_id,
            &version.id,
            &model,
            &verified,
            &template,
        )?;
        repository.update_job_status(&job.id, "completed", None)?;
        repository.update_record_status(record_id, "completed")?;
        spawn_incremental_index(library.root().to_path_buf(), record_id.to_owned());
        Ok::<(), crate::error::AppError>(())
    })();
    if let Err(error) = result {
        let message = error.to_string();
        let _ = repository.update_job_status(&job.id, "failed", Some(&message));
        let _ = repository.update_record_status(record_id, "failed");
        return Err(error);
    }
    Ok(())
}

#[tauri::command]
pub fn latest_analysis(
    state: State<AppState>,
    record_id: String,
) -> Result<Option<crate::types::StoredAnalysis>, String> {
    state
        .library
        .repository()
        .latest_analysis(&record_id)
        .map_err(|e| e.to_frontend())
}

fn schedule_memory_update(state: AppState) {
    let revision = state.schedule_memory_update();
    std::thread::spawn(move || {
        std::thread::sleep(AUTO_MEMORY_UPDATE_DELAY);
        if !state.is_latest_memory_update(revision) {
            return;
        }
        let Ok(settings) = memory::external_settings(&state.library) else {
            return;
        };
        if !settings.enabled || !settings.has_api_key || settings.privacy_consent_at.is_none() {
            return;
        }

        let (range_start, range_end) = automatic_memory_range();
        for view_kind in [MemoryViewKind::Map, MemoryViewKind::Evolution] {
            if !state.is_latest_memory_update(revision) {
                return;
            }
            let generation_id = format!("auto-{}-{}", view_kind.as_str(), uuid::Uuid::new_v4());
            let request = MemoryGenerationRequest {
                generation_id: generation_id.clone(),
                view_kind,
                scope: MemoryScope {
                    kind: "all".to_owned(),
                    project_id: None,
                },
                range_start: Some(range_start.clone()),
                range_end: Some(range_end.clone()),
            };
            if state.start_generation_job(&generation_id).is_err() {
                continue;
            }
            match memory::generate_snapshot(&state, &request) {
                Ok(snapshot) => state.finish_generation_job(&generation_id, &snapshot),
                Err(error) => state.fail_generation_job(&generation_id, error.to_frontend()),
            }
            state.clear_generation_cancel(&generation_id);
        }
    });
}

fn automatic_memory_range() -> (String, String) {
    let today = Local::now().date_naive();
    let start_naive = (today - ChronoDuration::days(89))
        .and_hms_milli_opt(0, 0, 0, 0)
        .expect("valid local start of day");
    let next_day_naive = (today + ChronoDuration::days(1))
        .and_hms_milli_opt(0, 0, 0, 0)
        .expect("valid local start of next day");
    let start_local = Local
        .from_local_datetime(&start_naive)
        .earliest()
        .unwrap_or_else(|| Local.from_utc_datetime(&start_naive));
    let next_day_local = Local
        .from_local_datetime(&next_day_naive)
        .earliest()
        .unwrap_or_else(|| Local.from_utc_datetime(&next_day_naive));
    let start = start_local
        .with_timezone(&Utc)
        .to_rfc3339_opts(SecondsFormat::Millis, true);
    let end = (next_day_local.with_timezone(&Utc) - ChronoDuration::milliseconds(1))
        .to_rfc3339_opts(SecondsFormat::Millis, true);
    (start, end)
}

/* -------------------------- external memory views -------------------------- */

#[tauri::command]
pub fn get_external_ai_settings(state: State<AppState>) -> Result<ExternalAiSettings, String> {
    memory::external_settings(&state.library).map_err(|error| error.to_frontend())
}

#[tauri::command]
pub fn update_external_ai_settings(
    state: State<AppState>,
    settings: ExternalAiSettings,
) -> Result<ExternalAiSettings, String> {
    let has_api_key = memory::get_api_key()
        .map_err(|error| error.to_frontend())?
        .is_some();
    state
        .library
        .repository()
        .update_external_ai_settings(&settings, has_api_key)
        .map_err(|error| error.to_frontend())
}

#[tauri::command]
pub fn set_external_ai_api_key(
    state: State<AppState>,
    api_key: String,
) -> Result<ExternalAiSettings, String> {
    memory::set_api_key(&api_key).map_err(|error| error.to_frontend())?;
    memory::external_settings(&state.library).map_err(|error| error.to_frontend())
}

#[tauri::command]
pub fn clear_external_ai_api_key(state: State<AppState>) -> Result<ExternalAiSettings, String> {
    memory::clear_api_key().map_err(|error| error.to_frontend())?;
    memory::external_settings(&state.library).map_err(|error| error.to_frontend())
}

#[tauri::command]
pub async fn test_external_ai_connection(state: State<'_, AppState>) -> Result<(), String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let settings =
            memory::external_settings(&state.library).map_err(|error| error.to_frontend())?;
        let api_key = memory::get_api_key()
            .map_err(|error| error.to_frontend())?
            .ok_or_else(|| "请先配置外部 AI API Key".to_owned())?;
        OpenAiCompatibleClient::new(&settings.base_url, &settings.model, &api_key)
            .and_then(|client| client.test())
            .map_err(|error| error.to_frontend())
    })
    .await
    .map_err(|_| "外部 AI 连接测试后台任务异常，请重试".to_owned())?
}

#[tauri::command]
pub fn get_local_timeline(
    state: State<AppState>,
    scope: MemoryScope,
    range_start: Option<String>,
    range_end: Option<String>,
) -> Result<Vec<TimelineItem>, String> {
    memory::local_timeline(
        &state.library,
        &scope,
        range_start.as_deref(),
        range_end.as_deref(),
    )
    .map_err(|error| error.to_frontend())
}

#[tauri::command]
pub fn get_local_growth_graph(
    state: State<AppState>,
    scope: MemoryScope,
    range_start: Option<String>,
    range_end: Option<String>,
) -> Result<GrowthGraph, String> {
    memory::local_growth_graph(
        &state.library,
        &scope,
        range_start.as_deref(),
        range_end.as_deref(),
    )
    .map_err(|error| error.to_frontend())
}

#[tauri::command]
pub async fn generate_memory_snapshot(
    state: State<'_, AppState>,
    request: MemoryGenerationRequest,
) -> Result<MemorySnapshot, String> {
    let state = state.inner().clone();
    state.clear_generation_cancel(&request.generation_id);
    tauri::async_runtime::spawn_blocking(move || {
        memory::generate_snapshot(&state, &request).map_err(|error| error.to_frontend())
    })
    .await
    .map_err(|_| "外部 AI 生成后台任务异常，请重试".to_owned())?
}

#[tauri::command]
pub fn start_memory_generation(
    state: State<AppState>,
    request: MemoryGenerationRequest,
) -> Result<MemoryGenerationJob, String> {
    start_memory_generation_with_state(state.inner().clone(), request)
        .map_err(|error| error.to_frontend())
}

fn start_memory_generation_with_state(
    state: AppState,
    request: MemoryGenerationRequest,
) -> crate::error::AppResult<MemoryGenerationJob> {
    let job = state.start_generation_job(&request.generation_id)?;
    std::thread::spawn(move || {
        match memory::generate_snapshot(&state, &request) {
            Ok(snapshot) => state.finish_generation_job(&request.generation_id, &snapshot),
            Err(error) => state.fail_generation_job(&request.generation_id, error.to_frontend()),
        }
        state.clear_generation_cancel(&request.generation_id);
    });
    Ok(job)
}

#[tauri::command]
pub fn get_memory_generation_job(
    state: State<AppState>,
    generation_id: String,
) -> Result<MemoryGenerationJob, String> {
    state
        .generation_job(&generation_id)
        .ok_or_else(|| "未找到该生成任务".to_owned())
}

#[tauri::command]
pub fn list_memory_snapshots(
    state: State<AppState>,
    view_kind: MemoryViewKind,
    scope: MemoryScope,
    range_start: Option<String>,
    range_end: Option<String>,
) -> Result<Vec<MemorySnapshot>, String> {
    state
        .library
        .repository()
        .list_memory_snapshots(
            &view_kind,
            &scope,
            range_start.as_deref(),
            range_end.as_deref(),
        )
        .map_err(|error| error.to_frontend())
}

#[tauri::command]
pub fn get_memory_snapshot(
    state: State<AppState>,
    snapshot_id: String,
) -> Result<MemorySnapshot, String> {
    state
        .library
        .repository()
        .get_memory_snapshot(&snapshot_id)
        .map_err(|error| error.to_frontend())
}

#[tauri::command]
pub fn cancel_memory_generation(
    state: State<AppState>,
    generation_id: String,
) -> Result<MemoryGenerationJob, String> {
    state
        .cancel_generation(&generation_id)
        .ok_or_else(|| "未找到该生成任务".to_owned())
}

#[tauri::command]
pub fn update_memory_feedback(
    state: State<AppState>,
    snapshot_id: String,
    item_id: String,
    decision: String,
    note: String,
) -> Result<MemoryFeedback, String> {
    state
        .library
        .repository()
        .update_memory_feedback(&snapshot_id, &item_id, &decision, &note)
        .map_err(|error| error.to_frontend())
}

#[tauri::command]
pub fn list_memory_feedback(
    state: State<AppState>,
    snapshot_id: String,
) -> Result<Vec<MemoryFeedback>, String> {
    state
        .library
        .repository()
        .list_memory_feedback(&snapshot_id)
        .map_err(|error| error.to_frontend())
}
