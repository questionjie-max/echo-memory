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
    ActionDashboard, AnalysisTemplate, DockReply, DockStatus, ExternalAiSettings, GrowthGraph,
    Hotword, InboxStatus, InboxWatchFolder, IngestResult, KnowledgeAnswer, KnowledgeIndexStatus,
    KnowledgeOverview, KnowledgeSettings, LocalAiStatus, LocalModelInfo, LocalWhisperModel,
    McpStatus, MemoryFeedback, MemoryGenerationJob, MemoryGenerationRequest, MemoryScope,
    MemorySnapshot, MemoryViewKind, ModelDownloadProgress, OnboardingStatus, OutputStatus,
    ProcessingJob, Project, RecordBrief, RelatedRecord, SearchResult, SuggestedWatchFolder,
    TemplateDraft, TemplateSection, TimelineItem, TranscriptBlock, TranscriptSegment,
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
pub async fn get_local_ai_status(state: State<'_, AppState>) -> Result<LocalAiStatus, String> {
    // 命令必须 async + spawn_blocking：Ollama 探测与模型列表是网络/子进程调用，
    // 同步命令会阻塞 Tauri 主线程导致界面冻结。
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || get_local_ai_status_blocking(&state))
        .await
        .map_err(|_| "读取本机 AI 状态的后台任务异常".to_owned())?
}

fn get_local_ai_status_blocking(state: &AppState) -> Result<LocalAiStatus, String> {
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

/// 断点续传下载：临时文件保留进度，网络失败后重试从上次位置继续；
/// 校验失败（数据损坏）才删除临时文件重新来过。
#[tauri::command]
pub fn download_whisper_model(
    app: AppHandle,
    state: State<AppState>,
    model_id: String,
) -> Result<(), String> {
    if model_id != "large-v3-turbo-q5_0" {
        return Err("不支持的 Whisper 模型".to_owned());
    }
    if !claim_whisper_download() {
        return Err("模型正在下载中".to_owned());
    }
    let models_dir = state.library.root().join("models");
    std::thread::spawn(move || {
        let temporary = models_dir.join(".ggml-large-v3-turbo-q5_0.download");
        let result = download_whisper_model_resumable(&app, &models_dir, &temporary, &model_id);
        // 网络中断保留临时文件以便续传；校验失败说明内容损坏，删除重来。
        if matches!(result, Err(WhisperDownloadError::Corrupted(_))) {
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
                error: Some(error.to_string()),
            },
        };
        let _ = app.emit("whisper-model-download-progress", progress);
        release_whisper_download();
    });
    Ok(())
}

enum WhisperDownloadError {
    Network(String),
    Corrupted(String),
}

impl std::fmt::Display for WhisperDownloadError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WhisperDownloadError::Network(message) => write!(formatter, "{message}"),
            WhisperDownloadError::Corrupted(message) => write!(formatter, "{message}"),
        }
    }
}

static WHISPER_DOWNLOAD_RUNNING: std::sync::Mutex<bool> = std::sync::Mutex::new(false);

fn claim_whisper_download() -> bool {
    match WHISPER_DOWNLOAD_RUNNING.lock() {
        Ok(mut running) => {
            if *running {
                false
            } else {
                *running = true;
                true
            }
        }
        Err(_) => false,
    }
}

fn release_whisper_download() {
    if let Ok(mut running) = WHISPER_DOWNLOAD_RUNNING.lock() {
        *running = false;
    }
}

fn download_whisper_model_resumable(
    app: &AppHandle,
    models_dir: &std::path::Path,
    temporary: &std::path::Path,
    model_id: &str,
) -> Result<(), WhisperDownloadError> {
    let network = |message: String| WhisperDownloadError::Network(message);
    let corrupted = |message: String| WhisperDownloadError::Corrupted(message);
    std::fs::create_dir_all(models_dir).map_err(|error| network(error.to_string()))?;
    let destination = models_dir.join("ggml-large-v3-turbo-q5_0.bin");
    if destination.is_file() {
        return Ok(());
    }

    // 从上次的临时文件继续：发 Range 请求；服务器不支持时回退到完整下载。
    let resume_from = std::fs::metadata(temporary)
        .map(|meta| meta.len())
        .unwrap_or(0);
    let mut request = ureq::get(
        "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo-q5_0.bin",
    );
    if resume_from > 0 && resume_from < LARGE_V3_TURBO_Q5_SIZE {
        request = request.set("Range", &format!("bytes={resume_from}-"));
    } else if resume_from >= LARGE_V3_TURBO_Q5_SIZE {
        // 临时文件异常偏大，作废重下。
        let _ = std::fs::remove_file(temporary);
    }
    let response = request
        .call()
        .map_err(|error| network(format!("无法下载 Whisper 模型：{error}")))?;
    let partial = response.status() == 206;
    let content_length = response
        .header("content-length")
        .and_then(|value| value.parse::<u64>().ok());
    let (offset, expected_total) = if partial {
        let total = resume_from + content_length.unwrap_or(0);
        (resume_from, total)
    } else {
        (0, content_length.unwrap_or(0))
    };
    let mut reader = response.into_reader();
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(temporary)
        .map_err(|error| network(error.to_string()))?;
    if !partial && resume_from > 0 {
        // 服务器忽略了 Range，从头写会追加到旧数据后面，必须先清空。
        file.set_len(0)
            .map_err(|error| network(format!("无法重置下载临时文件：{error}")))?;
    }

    let mut buffer = [0_u8; 64 * 1024];
    let mut completed = offset;
    loop {
        let count = reader
            .read(&mut buffer)
            .map_err(|error| network(format!("下载中断：{error}")))?;
        if count == 0 {
            break;
        }
        file.write_all(&buffer[..count])
            .map_err(|error| network(format!("写入临时文件失败：{error}")))?;
        completed += count as u64;
        let _ = app.emit(
            "whisper-model-download-progress",
            ModelDownloadProgress {
                model: model_id.to_owned(),
                status: "downloading".to_owned(),
                completed: Some(completed),
                total: Some(LARGE_V3_TURBO_Q5_SIZE),
                error: None,
            },
        );
    }
    file.sync_all()
        .map_err(|error| network(format!("落盘失败：{error}")))?;
    if completed != LARGE_V3_TURBO_Q5_SIZE
        || expected_total != 0 && expected_total != LARGE_V3_TURBO_Q5_SIZE
    {
        return Err(network(
            "下载的 Whisper 模型文件不完整，已保留进度供续传".to_owned(),
        ));
    }

    // 续传场景下分段哈希不可信，完成后对整个文件做最终校验。
    let mut final_file = std::fs::File::open(temporary)
        .map_err(|error| network(format!("无法读取下载文件：{error}")))?;
    let mut hasher = Sha256::new();
    let mut header = [0_u8; 4];
    let mut header_read = 0;
    loop {
        use std::io::Read;
        let count = final_file
            .read(&mut buffer)
            .map_err(|error| network(format!("校验读取失败：{error}")))?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
        if header_read < 4 {
            let take = count.min(4 - header_read);
            header[header_read..header_read + take].copy_from_slice(&buffer[..take]);
            header_read += take;
        }
    }
    if header != b"lmgg"[..] {
        return Err(corrupted(
            "下载内容不是有效的 GGML Whisper 模型，已重置".to_owned(),
        ));
    }
    let checksum = format!("{:x}", hasher.finalize());
    if checksum != LARGE_V3_TURBO_Q5_SHA256 {
        return Err(corrupted(
            "Whisper 模型 SHA-256 校验失败，临时文件已重置".to_owned(),
        ));
    }
    let _ = app.emit(
        "whisper-model-download-progress",
        ModelDownloadProgress {
            model: model_id.to_owned(),
            status: format!("校验完成 · SHA-256 {}…", &checksum[..12]),
            completed: Some(completed),
            total: Some(completed),
            error: None,
        },
    );
    std::fs::rename(temporary, &destination).map_err(|error| network(error.to_string()))?;
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
            let response = ureq::post(&format!("{}/api/pull", crate::analysis::ollama_base_url()))
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
    let response: serde_json::Value =
        ureq::get(&format!("{}/api/tags", crate::analysis::ollama_base_url()))
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
    let heavy_jobs = crate::state::heavy_job_limiter().clone();
    std::thread::spawn(move || {
        let _permit = heavy_jobs.acquire();
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
pub async fn ask_knowledge_base(
    state: State<'_, AppState>,
    project_id: Option<String>,
    unfiled_only: bool,
    question: String,
) -> Result<KnowledgeAnswer, String> {
    // 本地模型问答是长网络调用，必须在后台线程执行，避免冻结主线程。
    let library = state.inner().library.clone();
    tauri::async_runtime::spawn_blocking(move || {
        crate::knowledge::validate_scope(project_id.as_deref(), unfiled_only)
            .map_err(|error| error.to_frontend())?;
        crate::knowledge::ask(&library, project_id.as_deref(), unfiled_only, &question)
            .map_err(|error| error.to_frontend())
    })
    .await
    .map_err(|_| "知识库问答后台任务异常".to_owned())?
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
    // 编译机上的 target 目录只对开发环境有意义，release 包不应探测出开发路径。
    if cfg!(debug_assertions) {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        candidates.push(manifest_dir.join("target/debug/echo-memory-mcp"));
        candidates.push(manifest_dir.join("target/release/echo-memory-mcp"));
    }
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
        let heavy_jobs = crate::state::heavy_job_limiter().clone();
        std::thread::spawn(move || {
            let _permit = heavy_jobs.acquire();
            match ManagedLibrary::open(library_root.clone()) {
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
    let heavy_jobs = crate::state::heavy_job_limiter().clone();
    std::thread::spawn(move || {
        let _permit = heavy_jobs.acquire();
        match ManagedLibrary::open(library_root.clone()) {
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
    let heavy_jobs = crate::state::heavy_job_limiter().clone();
    std::thread::spawn(move || {
        let _permit = heavy_jobs.acquire();
        match ManagedLibrary::open(library_root.clone()) {
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
        let hotword_prefix = repository.hotwords_prompt().unwrap_or_default();
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
            let initial_prompt = if prompt.is_empty() {
                hotword_prefix.clone()
            } else if hotword_prefix.is_empty() {
                prompt.clone()
            } else {
                format!("{hotword_prefix}\n{prompt}")
            };
            let chunk_segments = adapter.transcribe_samples(
                &samples[chunk.sample_start..chunk.sample_end],
                language,
                (!initial_prompt.is_empty()).then_some(initial_prompt.as_str()),
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
    let heavy_jobs = crate::state::heavy_job_limiter().clone();
    std::thread::spawn(move || {
        let _permit = heavy_jobs.acquire();
        match ManagedLibrary::open(library_root.clone()) {
            Ok(library) => {
                if let Err(error) =
                    crate::knowledge::incremental_rebuild_record(&library, &record_id)
                {
                    let _ = library
                        .repository()
                        .mark_knowledge_indexes_failed(&error.to_string());
                }
            }
            Err(error) => {
                mark_knowledge_indexes_failed(&library_root, &error.to_string());
            }
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
    let heavy_jobs = crate::state::heavy_job_limiter().clone();
    std::thread::spawn(move || {
        let _permit = heavy_jobs.acquire();
        match ManagedLibrary::open(library_root.clone()) {
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
        if crate::dock::auto_export_enabled(&library.repository())? {
            let _ = crate::dock::export_analysis_to_output(library, record_id);
        }
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

/* ------------------------------ v0.3.0：引导 / 收件箱 / 词汇库 / 仪表盘 ------------------------------ */

#[tauri::command]
pub async fn get_onboarding_status(state: State<'_, AppState>) -> Result<OnboardingStatus, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || get_onboarding_status_blocking(&state))
        .await
        .map_err(|_| "读取引导状态的后台任务异常".to_owned())?
}

fn get_onboarding_status_blocking(state: &AppState) -> Result<OnboardingStatus, String> {
    let repository = state.library.repository();
    let settings = repository
        .knowledge_settings()
        .map_err(|error| error.to_frontend())?;
    let whisper = WhisperAdapter::detect_with_model_path(Some(&settings.whisper_model_path)).ok();
    let whisper_model_path = whisper
        .as_ref()
        .and_then(WhisperAdapter::model_path)
        .map(|path| path.to_string_lossy().to_string());
    let models = ollama_models().unwrap_or_default();
    let analysis_model_ready = models
        .iter()
        .any(|model| model.name == settings.analysis_model);
    let watch_folders = repository
        .list_watch_folders()
        .map_err(|error| error.to_frontend())?;
    Ok(OnboardingStatus {
        completed: repository
            .onboarding_completed_at()
            .map_err(|error| error.to_frontend())?
            .is_some(),
        completed_at: repository
            .onboarding_completed_at()
            .map_err(|error| error.to_frontend())?,
        whisper_ready: whisper.is_some(),
        whisper_model_path,
        ollama_running: !models.is_empty() || ollama_models().is_ok(),
        analysis_model_ready,
        analysis_model: settings.analysis_model,
        watch_folder_count: watch_folders.len() as u32,
        usb_detection: repository
            .inbox_usb_detection()
            .map_err(|error| error.to_frontend())?,
    })
}

#[tauri::command]
pub fn complete_onboarding(state: State<AppState>) -> Result<(), String> {
    state
        .library
        .repository()
        .complete_onboarding()
        .map_err(|error| error.to_frontend())
}

#[tauri::command]
pub fn reset_onboarding(state: State<AppState>) -> Result<(), String> {
    state
        .library
        .repository()
        .reset_onboarding()
        .map_err(|error| error.to_frontend())
}

#[tauri::command]
pub fn suggest_watch_folders() -> Vec<SuggestedWatchFolder> {
    crate::inbox::suggest_watch_folders()
        .into_iter()
        .map(|(label, path)| SuggestedWatchFolder { label, path })
        .collect()
}

#[tauri::command]
pub fn get_inbox_status(state: State<AppState>) -> Result<InboxStatus, String> {
    let repository = state.library.repository();
    let watch_folders = repository
        .list_watch_folders()
        .map_err(|error| error.to_frontend())?;
    let counts = repository
        .inbox_status_counts()
        .map_err(|error| error.to_frontend())?;
    let recent_files = repository
        .recent_seen_files(12)
        .map_err(|error| error.to_frontend())?;
    Ok(InboxStatus {
        usb_detection: repository
            .inbox_usb_detection()
            .map_err(|error| error.to_frontend())?,
        watch_folders,
        counts,
        recent_files,
    })
}

#[tauri::command]
pub fn add_inbox_watch_folder(
    state: State<AppState>,
    path: String,
    label: Option<String>,
) -> Result<InboxWatchFolder, String> {
    let validated = crate::inbox::validate_watch_folder(&path).map_err(|e| e.to_frontend())?;
    // 先基线后登记：登记前扫描器不认识该目录，基线完成前的竞态窗口不存在。
    crate::inbox::baseline_folder(&state.library, &validated);
    let label = label.unwrap_or_else(|| {
        validated
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| "监听文件夹".to_owned())
    });
    let folder = state
        .library
        .repository()
        .add_watch_folder(&validated.to_string_lossy(), &label)
        .map_err(|error| error.to_frontend())?;
    crate::inbox::request_rescan();
    Ok(folder)
}

#[tauri::command]
pub fn remove_inbox_watch_folder(state: State<AppState>, id: String) -> Result<(), String> {
    state
        .library
        .repository()
        .remove_watch_folder(&id)
        .map_err(|error| error.to_frontend())
}

#[tauri::command]
pub fn set_inbox_usb_detection(state: State<AppState>, enabled: bool) -> Result<(), String> {
    state
        .library
        .repository()
        .set_inbox_usb_detection(enabled)
        .map_err(|error| error.to_frontend())?;
    crate::inbox::request_rescan();
    Ok(())
}

#[tauri::command]
pub fn rescan_inbox() {
    crate::inbox::request_rescan();
}

#[tauri::command]
pub fn list_hotwords(state: State<AppState>) -> Result<Vec<Hotword>, String> {
    state
        .library
        .repository()
        .list_hotwords()
        .map_err(|error| error.to_frontend())
}

#[tauri::command]
pub fn add_hotword(
    state: State<AppState>,
    term: String,
    note: Option<String>,
) -> Result<Hotword, String> {
    state
        .library
        .repository()
        .add_hotword(&term, note.as_deref().unwrap_or(""))
        .map_err(|error| error.to_frontend())
}

#[tauri::command]
pub fn remove_hotword(state: State<AppState>, id: String) -> Result<(), String> {
    state
        .library
        .repository()
        .remove_hotword(&id)
        .map_err(|error| error.to_frontend())
}

#[tauri::command]
pub fn get_action_dashboard(state: State<AppState>) -> Result<ActionDashboard, String> {
    let repository = state.library.repository();
    let items = repository
        .list_action_items_detailed()
        .map_err(|error| error.to_frontend())?;
    let open_count = items.iter().filter(|item| item.status == "open").count() as u32;
    let done_count = (items.len() as u32).saturating_sub(open_count);
    let open_questions = repository
        .list_open_questions(30)
        .map_err(|error| error.to_frontend())?;
    Ok(ActionDashboard {
        open_count,
        done_count,
        items,
        open_questions,
    })
}

#[tauri::command]
pub fn set_action_item_status(
    state: State<AppState>,
    id: String,
    status: String,
) -> Result<(), String> {
    state
        .library
        .repository()
        .set_action_item_status(&id, &status)
        .map_err(|error| error.to_frontend())
}

#[tauri::command]
pub fn related_records(
    state: State<AppState>,
    record_id: String,
    limit: Option<u32>,
) -> Result<Vec<RelatedRecord>, String> {
    let limit = limit.unwrap_or(5).clamp(1, 10);
    let repository = state.library.repository();
    let settings = repository
        .knowledge_settings()
        .map_err(|error| error.to_frontend())?;
    let chunks = repository
        .list_knowledge_chunks(None, false, &settings.embedding_model)
        .map_err(|error| error.to_frontend())?;
    if chunks.is_empty() {
        return Ok(Vec::new());
    }

    // 记录级质心：对该记录全部向量块求平均，再算余弦相似度。
    let mut centroids: std::collections::HashMap<String, Vec<f32>> =
        std::collections::HashMap::new();
    let mut counts: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
    let mut titles: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut projects: std::collections::HashMap<String, Option<String>> =
        std::collections::HashMap::new();
    for chunk in &chunks {
        titles.insert(chunk.record_id.clone(), chunk.record_title.clone());
        projects.insert(chunk.record_id.clone(), chunk.project_id.clone());
        let dimension = chunk.embedding.len();
        let entry = centroids
            .entry(chunk.record_id.clone())
            .or_insert_with(|| vec![0.0; dimension]);
        for (index, value) in chunk.embedding.iter().enumerate() {
            if index < entry.len() {
                entry[index] += value;
            }
        }
        *counts.entry(chunk.record_id.clone()).or_insert(0) += 1;
    }
    let Some(target) = centroids.get(&record_id) else {
        return Ok(Vec::new());
    };
    let norm =
        |vector: &[f32]| -> f32 { vector.iter().map(|value| value * value).sum::<f32>().sqrt() };
    let target_norm = norm(target);
    if target_norm == 0.0 {
        return Ok(Vec::new());
    }
    let mut scored: Vec<RelatedRecord> = centroids
        .iter()
        .filter(|(id, _)| *id != &record_id)
        .filter_map(|(id, vector)| {
            let vector_norm = norm(vector);
            if vector_norm == 0.0 {
                return None;
            }
            let dot: f32 = target
                .iter()
                .zip(vector.iter())
                .map(|(left, right)| left * right)
                .sum();
            Some(RelatedRecord {
                record_id: id.clone(),
                title: titles.get(id).cloned().unwrap_or_default(),
                similarity: dot / (target_norm * vector_norm),
                project_id: projects.get(id).cloned().flatten(),
            })
        })
        .collect();
    scored.sort_by(|left, right| {
        right
            .similarity
            .partial_cmp(&left.similarity)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    scored.truncate(limit as usize);
    Ok(scored)
}

#[tauri::command]
pub fn correct_transcript(
    app: AppHandle,
    state: State<AppState>,
    record_id: String,
) -> Result<(), String> {
    let library_root = state.library.root().to_path_buf();
    std::thread::spawn(move || {
        let result = (|| -> Result<u32, String> {
            let library = ManagedLibrary::open(library_root).map_err(|error| error.to_string())?;
            let _permit = crate::state::heavy_job_limiter().acquire();
            crate::analysis::correct_transcript_with_library(&library, &record_id)
                .map_err(|error| error.to_string())
        })();
        let _ = app.emit(
            "transcript-corrected",
            serde_json::json!({
                "recordId": record_id,
                "ok": result.is_ok(),
                "message": match &result {
                    Ok(count) => format!("已校对 {count} 个片段"),
                    Err(error) => error.clone(),
                },
            }),
        );
    });
    Ok(())
}

#[tauri::command]
pub fn get_transcript_correction_enabled(state: State<AppState>) -> Result<bool, String> {
    Ok(state
        .library
        .repository()
        .setting_value("transcript_correction_enabled")
        .map_err(|error| error.to_frontend())?
        .as_deref()
        .is_some_and(|value| value == "true"))
}

#[tauri::command]
pub fn set_transcript_correction_enabled(
    state: State<AppState>,
    enabled: bool,
) -> Result<(), String> {
    state
        .library
        .repository()
        .set_setting_value(
            "transcript_correction_enabled",
            if enabled { "true" } else { "false" },
        )
        .map_err(|error| error.to_frontend())
}

/* ------------------------- v0.4.0：AI 伙伴 / 模板向导 / 产出文件夹 ------------------------- */

#[tauri::command]
pub async fn get_dock_status(state: State<'_, AppState>) -> Result<DockStatus, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || get_dock_status_blocking(&state))
        .await
        .map_err(|_| "读取对话状态的后台任务异常".to_owned())?
}

fn get_dock_status_blocking(state: &AppState) -> Result<DockStatus, String> {
    let repository = state.library.repository();
    let chat = repository
        .latest_dock_chat()
        .map_err(|error| error.to_frontend())?;
    let messages = match &chat {
        Some(chat) => repository
            .list_dock_messages(&chat.id, 60)
            .map_err(|error| error.to_frontend())?,
        None => Vec::new(),
    };
    Ok(DockStatus {
        chat,
        messages,
        external_available: crate::dock::external_available(&state.library),
    })
}

#[tauri::command]
pub async fn ask_dock(
    state: State<'_, AppState>,
    mode: String,
    message: String,
    engine: Option<String>,
    record_id: Option<String>,
) -> Result<DockReply, String> {
    // 模型生成可能长达几十秒：必须放后台线程，主线程只等结果（UI 保持可交互）。
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        ask_dock_blocking(&state, &mode, message, engine, record_id)
    })
    .await
    .map_err(|_| "AI 伙伴后台任务异常".to_owned())?
}

fn ask_dock_blocking(
    state: &AppState,
    mode: &str,
    message: String,
    engine: Option<String>,
    record_id: Option<String>,
) -> Result<DockReply, String> {
    let message = message.trim().to_owned();
    if message.is_empty() || message.chars().count() > 8_000 {
        return Err("消息内容无效".to_owned());
    }
    let library = &state.library;
    let repository = library.repository();
    let mut chat = match repository
        .latest_dock_chat()
        .map_err(|error| error.to_frontend())?
    {
        Some(chat) => chat,
        None => repository
            .create_dock_chat("local", "新对话")
            .map_err(|error| error.to_frontend())?,
    };
    let is_new_chat = repository
        .list_dock_messages(&chat.id, 1)
        .map_err(|error| error.to_frontend())?
        .is_empty();
    let title = if is_new_chat {
        message.chars().take(24).collect::<String>()
    } else {
        chat.title.clone()
    };
    let user_message = repository
        .append_dock_message(&chat.id, "user", &message, mode)
        .map_err(|error| error.to_frontend())?;
    let history = repository
        .list_dock_messages(&chat.id, crate::dock::history_turns() * 2)
        .map_err(|error| error.to_frontend())?
        .into_iter()
        .filter(|existing| existing.id != user_message.id)
        .collect::<Vec<_>>();

    let engine = engine.unwrap_or_else(|| "local".to_owned());
    let reply = match engine.as_str() {
        "external" => {
            crate::dock::ask_external(library, &mode, &history, &message, record_id.as_deref())
                .map_err(|error| error.to_frontend())?
        }
        _ => crate::dock::ask_local(library, &mode, &history, &message, record_id.as_deref())
            .map_err(|error| error.to_frontend())?,
    };
    let assistant_message = repository
        .append_dock_message(&chat.id, "assistant", &reply, mode)
        .map_err(|error| error.to_frontend())?;
    repository
        .touch_dock_chat(&chat.id, Some(&title))
        .map_err(|error| error.to_frontend())?;
    chat.title = title;
    chat.engine = engine;
    Ok(DockReply {
        chat,
        user_message,
        assistant_message,
    })
}

#[tauri::command]
pub fn clear_dock_chat(state: State<AppState>) -> Result<(), String> {
    state
        .library
        .repository()
        .clear_dock_chats()
        .map_err(|error| error.to_frontend())
}

#[tauri::command]
pub async fn generate_template_draft(
    state: State<'_, AppState>,
    messages: Vec<crate::dock::WizardMessage>,
) -> Result<TemplateDraft, String> {
    let library = state.inner().library.clone();
    tauri::async_runtime::spawn_blocking(move || {
        crate::dock::generate_template_draft(&library, &messages)
            .map_err(|error| error.to_frontend())
    })
    .await
    .map_err(|_| "模板草稿后台任务异常".to_owned())?
}

#[tauri::command]
pub fn get_output_status(state: State<AppState>) -> Result<OutputStatus, String> {
    let repository = state.library.repository();
    let folder = repository
        .setting_value("output_folder")
        .map_err(|error| error.to_frontend())?
        .filter(|value| !value.trim().is_empty());
    let auto_export_analysis = repository
        .setting_value("auto_export_analysis")
        .map_err(|error| error.to_frontend())?
        .as_deref()
        .is_some_and(|value| value == "true");
    let recent_files = match &folder {
        Some(folder) => crate::dock::list_recent_outputs(std::path::Path::new(folder), 10),
        None => Vec::new(),
    };
    Ok(OutputStatus {
        folder,
        auto_export_analysis,
        recent_files,
    })
}

#[tauri::command]
pub fn set_output_folder(
    state: State<AppState>,
    path: Option<String>,
) -> Result<OutputStatus, String> {
    let repository = state.library.repository();
    match path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(path) => {
            let candidate = std::path::PathBuf::from(path);
            if !candidate.is_dir() {
                return Err("目录不存在，请检查路径".to_owned());
            }
            repository
                .set_setting_value("output_folder", &candidate.to_string_lossy())
                .map_err(|error| error.to_frontend())?;
        }
        None => {
            repository
                .set_setting_value("output_folder", "")
                .map_err(|error| error.to_frontend())?;
        }
    }
    get_output_status(state)
}

#[tauri::command]
pub fn set_auto_export_analysis(
    state: State<AppState>,
    enabled: bool,
) -> Result<OutputStatus, String> {
    state
        .library
        .repository()
        .set_setting_value(
            "auto_export_analysis",
            if enabled { "true" } else { "false" },
        )
        .map_err(|error| error.to_frontend())?;
    get_output_status(state)
}

#[tauri::command]
pub fn export_record_to_output(
    state: State<AppState>,
    record_id: String,
    kind: String,
) -> Result<String, String> {
    crate::dock::export_record_kind(&state.library, &record_id, &kind)
        .map(|path| path.to_string_lossy().to_string())
        .map_err(|error| error.to_frontend())
}

#[tauri::command]
pub fn save_dock_message_to_output(
    state: State<AppState>,
    title: String,
    content: String,
) -> Result<String, String> {
    crate::dock::save_markdown_to_output(&state.library, &title, &content, "对话")
        .map(|path| path.to_string_lossy().to_string())
        .map_err(|error| error.to_frontend())
}