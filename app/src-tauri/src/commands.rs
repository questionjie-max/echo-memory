use crate::analysis::{
    analyze_with_generator_instruction_progress, AnalysisTextGenerator, OllamaAdapter,
};
use crate::audio::{
    is_overlap_duplicate, merge_overlap_continuation, normalize_segment_timestamps, plan_chunks,
    preprocess, preprocessor_status, read_normalized_wav, AudioPreprocessorStatus,
};
use crate::db::repository::LibraryRepository;
use crate::error::AppError;
use crate::error::AppResult;
use crate::external_ai_gate::{
    clear_analysis_credentials, clear_transcription_credentials, credentials_are_complete,
    external_settings, get_analysis_credentials, get_asr_credentials, get_api_key,
    require_external_ai_consent, set_analysis_credentials, set_transcription_credentials,
};
use crate::library::ManagedLibrary;
use crate::memory::{self, OpenAiCompatibleClient};
use crate::provider_catalog::{self, ProviderCatalog};
use crate::state::AppState;
use crate::transcript::effective_text;
use crate::types::{
    ActionDashboard, AgentSessionSelection, AnalysisTemplate, AppInfo, ChatImportOutcome,
    DockReply, DockStatus, ExternalAiSettings, GrowthGraph, Hotword, InboxStatus, InboxWatchFolder,
    IngestResult, KnowledgeAnswer, KnowledgeIndexStatus, KnowledgeOverview, KnowledgeSettings,
    LocalAiStatus, LocalModelInfo, LocalWhisperModel, McpStatus, MemoryFeedback,
    MemoryGenerationJob, MemoryGenerationRequest, MemoryScope, MemorySnapshot, MemoryViewKind,
    ModelDownloadProgress, OnboardingStatus, OllamaInstallStatus, OutputStatus,
    PendingModelDownload, ProcessingJob,
    ProfileSnapshot, Project, RecommendedModel, RecordBrief, RelatedRecord, SearchResult,
    SpeakerSummary, SuggestedWatchFolder, TemplateDraft, TemplateSection, TimelineItem,
    TranscriptBlock, TranscriptSegment, TranscriptSegmentInput, TranscriptionEngineStatus,
};
use crate::whisper::{
    library_model_files, localize_speaker_label, WhisperAdapter, WhisperSession,
    RECOMMENDED_MODEL_BYTES, RECOMMENDED_MODEL_ID, RECOMMENDED_MODEL_SHA256, RECOMMENDED_MODEL_URL,
};
use chrono::{Duration as ChronoDuration, Local, SecondsFormat, TimeZone, Utc};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use tauri::{AppHandle, Emitter, State};

const AUTO_MEMORY_UPDATE_DELAY: Duration = Duration::from_secs(120);

/// 跨块接续合并时向前回溯的已接受片段数。重叠区里的重复几乎总落在最近一两条，
/// 不需要扫全表。
const OVERLAP_MERGE_SCAN: usize = 3;

fn emit_processing_progress(app: &AppHandle, record_id: &str) {
    let _ = app.emit(
        "processing-progress",
        serde_json::json!({ "recordId": record_id }),
    );
}

fn emit_knowledge_index_update(app: &AppHandle) {
    let _ = app.emit("knowledge-index-update", ());
}

/// 记忆块索引重建：确认反馈或快照生成完成后触发；不阻塞调用方，
/// 无 AppHandle 的内部路径不发光发事件，界面状态靠下一次事件刷新。
pub(crate) fn spawn_memory_index_rebuild(app: Option<AppHandle>, library_root: PathBuf) {
    let heavy_jobs = crate::state::heavy_job_limiter().clone();
    std::thread::spawn(move || {
        let _permit = heavy_jobs.acquire();
        match ManagedLibrary::open(library_root.clone()) {
            Ok(library) => {
                if let Err(error) = crate::memory_index::rebuild_memory_index(&library) {
                    let _ = library
                        .repository()
                        .mark_knowledge_indexes_failed(&error.to_string());
                }
            }
            Err(error) => {
                mark_knowledge_indexes_failed(&library_root, &error.to_string());
            }
        }
        if let Some(app) = app {
            emit_knowledge_index_update(&app);
        }
    });
}

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
    let models_dir = state.library.root().join("models");
    let whisper = WhisperAdapter::detect_with_model_path(Some(&settings.whisper_model_path)).ok();
    let whisper_model_path = whisper
        .as_ref()
        .and_then(WhisperAdapter::model_path)
        .map(|path| path.to_string_lossy().to_string());
    let whisper_model_source = whisper.as_ref().and_then(|adapter| {
        adapter
            .model_path()
            .map(|path| whisper_model_source_label(path, &settings.whisper_model_path, &models_dir))
    });
    let (ollama_available, ollama_models) = match ollama_models() {
        Ok(models) => (true, models),
        Err(_) => (false, Vec::new()),
    };
    Ok(LocalAiStatus {
        whisper_available: whisper.is_some(),
        whisper_model_path,
        whisper_model_source,
        whisper_models: discover_whisper_models(&models_dir, &settings.whisper_model_path),
        recommended_whisper_model: RecommendedModel {
            id: RECOMMENDED_MODEL_ID.to_owned(),
            label: "large-v3-turbo（q5_0）".to_owned(),
            file_name: crate::whisper::recommended_model_file(),
            bytes: RECOMMENDED_MODEL_BYTES,
        },
        pending_whisper_download: pending_whisper_download(&models_dir),
        ollama_available,
        ollama_models,
        settings,
    })
}

/// 当前这个模型是哪来的，用大白话说清楚，用户才知道能不能换掉它。
fn whisper_model_source_label(
    path: &std::path::Path,
    configured: &str,
    models_dir: &std::path::Path,
) -> String {
    if !configured.trim().is_empty() && std::path::Path::new(configured.trim()) == path {
        return "手动选择".to_owned();
    }
    if path.starts_with(models_dir) {
        return "应用内下载".to_owned();
    }
    "环境变量指定".to_owned()
}

/// 中断留下的半成品，用来显示「继续下载」而不是从零开始。
fn pending_whisper_download(models_dir: &std::path::Path) -> Option<PendingModelDownload> {
    let bytes = std::fs::metadata(models_dir.join(crate::whisper::recommended_model_partial()))
        .ok()?
        .len();
    (bytes > 0).then(|| PendingModelDownload {
        model: RECOMMENDED_MODEL_ID.to_owned(),
        bytes,
    })
}

/// 可选模型 = 显式配置的那个（如果有）+ 应用模型目录里的全部模型文件。
fn discover_whisper_models(
    models_dir: &std::path::Path,
    configured: &str,
) -> Vec<LocalWhisperModel> {
    let mut paths = Vec::new();
    if !configured.trim().is_empty() {
        paths.push(PathBuf::from(configured.trim()));
    }
    paths.extend(library_model_files(models_dir));
    let mut seen = HashSet::new();
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

/// 已请求取消的模型下载。Whisper 与 Ollama 共用一套：下载循环按模型名查这张表。
fn download_cancellations() -> &'static Mutex<HashSet<String>> {
    static CANCELLATIONS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    CANCELLATIONS.get_or_init(|| Mutex::new(HashSet::new()))
}

fn request_cancellation(model: &str) {
    if let Ok(mut cancelled) = download_cancellations().lock() {
        cancelled.insert(model.to_owned());
    }
}

fn clear_cancellation(model: &str) {
    if let Ok(mut cancelled) = download_cancellations().lock() {
        cancelled.remove(model);
    }
}

fn is_cancelled(model: &str) -> bool {
    download_cancellations()
        .lock()
        .map(|cancelled| cancelled.contains(model))
        .unwrap_or(false)
}

/// 取消正在进行的模型下载。已经下好的部分会保留，下次可以接着下。
#[tauri::command]
pub fn cancel_model_download(model: String) -> Result<(), String> {
    let model = model.trim();
    if model.is_empty() {
        return Err("模型名称无效".to_owned());
    }
    request_cancellation(model);
    Ok(())
}

/// 断点续传下载：临时文件保留进度，网络中断或用户取消后重试都从上次位置继续；
/// 校验失败（数据损坏）才删除临时文件重新来过。
#[tauri::command]
pub fn download_whisper_model(
    app: AppHandle,
    state: State<AppState>,
    model_id: String,
) -> Result<(), String> {
    if model_id != RECOMMENDED_MODEL_ID {
        return Err("不支持的 Whisper 模型".to_owned());
    }
    if !claim_whisper_download() {
        return Err("模型正在下载中".to_owned());
    }
    clear_cancellation(&model_id);
    let models_dir = state.library.root().join("models");
    // 574MB 模型 + 临时文件：空间不足立即拒绝，不启动下载（续传也一样先查）。
    crate::disk_space::ensure_free_space(
        &models_dir,
        crate::disk_space::required_model_download_bytes(crate::whisper::RECOMMENDED_MODEL_BYTES),
    )
    .map_err(|error| error.to_frontend())?;
    let library = state.library.clone();
    std::thread::spawn(move || {
        let temporary = models_dir.join(crate::whisper::recommended_model_partial());
        let result = download_whisper_model_resumable(&app, &models_dir, &temporary, &model_id);
        // 网络中断与用户取消都保留临时文件以便续传；校验失败说明内容损坏，删除重来。
        if matches!(result, Err(WhisperDownloadError::Corrupted(_))) {
            let _ = std::fs::remove_file(&temporary);
        }
        // 下载完立刻登记为当前模型。少了这一步，用户在引导里下完 574MB 之后
        // 第一次转写仍然会报「尚未安装转写模型」。
        let registration = result
            .is_ok()
            .then(|| register_downloaded_whisper_model(&library, &models_dir));
        let progress = match (result, registration) {
            (Ok(()), Some(Err(error))) => ModelDownloadProgress {
                model: model_id.clone(),
                status: "failed".to_owned(),
                completed: None,
                total: None,
                error: Some(error),
            },
            (Ok(()), _) => ModelDownloadProgress {
                model: model_id.clone(),
                status: "completed".to_owned(),
                completed: None,
                total: None,
                error: None,
            },
            (Err(WhisperDownloadError::Cancelled), _) => ModelDownloadProgress {
                model: model_id.clone(),
                status: "cancelled".to_owned(),
                completed: None,
                total: None,
                error: None,
            },
            (Err(error), _) => ModelDownloadProgress {
                model: model_id.clone(),
                status: "failed".to_owned(),
                completed: None,
                total: None,
                error: Some(error.to_string()),
            },
        };
        let _ = app.emit("whisper-model-download-progress", progress);
        clear_cancellation(&model_id);
        release_whisper_download();
    });
    Ok(())
}

/// 把刚下载好的模型写成当前模型，让「下载完就能用」成立。
fn register_downloaded_whisper_model(
    library: &ManagedLibrary,
    models_dir: &std::path::Path,
) -> Result<(), String> {
    let model_path = models_dir.join(crate::whisper::recommended_model_file());
    if !model_path.is_file() {
        return Err("模型下载完成但文件不存在".to_owned());
    }
    let repository = library.repository();
    let mut settings = repository
        .knowledge_settings()
        .map_err(|error| error.to_frontend())?;
    settings.whisper_model_path = model_path.to_string_lossy().to_string();
    repository
        .update_knowledge_settings(&settings)
        .map_err(|error| error.to_frontend())?;
    Ok(())
}

enum WhisperDownloadError {
    Network(String),
    Corrupted(String),
    Cancelled,
}

impl std::fmt::Display for WhisperDownloadError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WhisperDownloadError::Network(message) => write!(formatter, "{message}"),
            WhisperDownloadError::Corrupted(message) => write!(formatter, "{message}"),
            WhisperDownloadError::Cancelled => write!(formatter, "已取消下载"),
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
    let destination = models_dir.join(crate::whisper::recommended_model_file());
    if destination.is_file() {
        return Ok(());
    }
    if is_cancelled(model_id) {
        return Err(WhisperDownloadError::Cancelled);
    }

    // 从上次的临时文件继续：发 Range 请求；服务器不支持时回退到完整下载。
    let resume_from = std::fs::metadata(temporary)
        .map(|meta| meta.len())
        .unwrap_or(0);
    let mut request = ureq::get(RECOMMENDED_MODEL_URL);
    if resume_from > 0 && resume_from < RECOMMENDED_MODEL_BYTES {
        request = request.set("Range", &format!("bytes={resume_from}-"));
    } else if resume_from >= RECOMMENDED_MODEL_BYTES {
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
        // 取消只在下完当前这一块之后生效；已经落到磁盘的部分会保留，下次接着下。
        if is_cancelled(model_id) {
            let _ = file.sync_all();
            return Err(WhisperDownloadError::Cancelled);
        }
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
                total: Some(RECOMMENDED_MODEL_BYTES),
                error: None,
            },
        );
    }
    file.sync_all()
        .map_err(|error| network(format!("落盘失败：{error}")))?;
    if completed != RECOMMENDED_MODEL_BYTES
        || expected_total != 0 && expected_total != RECOMMENDED_MODEL_BYTES
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
    if checksum != RECOMMENDED_MODEL_SHA256 {
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

enum PullError {
    Failed(String),
    Cancelled,
}

#[tauri::command]
pub fn pull_ollama_model(app: AppHandle, model: String) -> Result<(), String> {
    let model = model.trim().to_owned();
    if model.is_empty() || model.len() > 120 {
        return Err("模型名称无效".to_owned());
    }
    clear_cancellation(&model);
    std::thread::spawn(move || {
        let result = (|| -> Result<(), PullError> {
            let failed = |message: String| PullError::Failed(message);
            // 流式下载不能设整体超时（大模型要下几十分钟），只对「连接」和
            // 「单次读」限时：只要进度行还在持续到达就不会触发。
            let pull_agent = ureq::AgentBuilder::new()
                .timeout_connect(std::time::Duration::from_secs(10))
                .timeout_read(std::time::Duration::from_secs(120))
                .build();
            let response = pull_agent
                .post(&format!("{}/api/pull", crate::analysis::ollama_base_url()))
                .send_json(serde_json::json!({ "model": model, "stream": true }))
                .map_err(|error| failed(format!("无法启动模型下载：{error}")))?;
            for line in BufReader::new(response.into_reader()).lines() {
                // 取消就断开连接：Ollama 自己会保留已下好的分块，下次 pull 能接着下。
                if is_cancelled(&model) {
                    return Err(PullError::Cancelled);
                }
                let line = line.map_err(|error| failed(format!("读取下载进度失败：{error}")))?;
                let value: serde_json::Value = serde_json::from_str(&line)
                    .map_err(|error| failed(format!("下载进度格式无效：{error}")))?;
                if let Some(error) = value.get("error").and_then(serde_json::Value::as_str) {
                    return Err(failed(error.to_owned()));
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
            Err(PullError::Cancelled) => ModelDownloadProgress {
                model: model.clone(),
                status: "cancelled".to_owned(),
                completed: None,
                total: None,
                error: None,
            },
            Err(PullError::Failed(error)) => ModelDownloadProgress {
                model: model.clone(),
                status: "failed".to_owned(),
                completed: None,
                total: None,
                error: Some(error),
            },
        };
        let _ = app.emit("model-download-progress", progress);
        clear_cancellation(&model);
    });
    Ok(())
}

fn ollama_models() -> Result<Vec<LocalModelInfo>, String> {
    let response: serde_json::Value =
        ureq::get(&format!("{}/api/tags", crate::analysis::ollama_base_url()))
            .timeout(crate::analysis::OLLAMA_TAGS_TIMEOUT)
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
pub fn list_archived_records(
    state: State<AppState>,
    project_id: Option<String>,
    unfiled_only: bool,
) -> Result<Vec<RecordBrief>, String> {
    state
        .library
        .repository()
        .list_archived_records(project_id.as_deref(), unfiled_only)
        .map_err(|e| e.to_frontend())
}

#[tauri::command]
pub fn set_record_archived(
    app: AppHandle,
    state: State<AppState>,
    record_id: String,
    archived: bool,
) -> Result<RecordBrief, String> {
    let updated = state
        .library
        .repository()
        .set_record_archived(&record_id, archived)
        .map_err(|e| e.to_frontend())?;
    spawn_index_metadata_refresh(app, state.library.root().to_path_buf());
    schedule_memory_update(state.inner().clone());
    Ok(updated)
}

/// 块4：导入内容确认门——document/chat 记录「确认入记忆 / 撤回」。
/// 确认后内容才对 MCP 可见；撤回立即收回（与纪要条目同语义，双向可逆）。
/// 确认（而非撤回）时触发记忆链调度：分析结论此刻才允许进演化/记忆（终审修2）。
#[tauri::command]
pub fn set_record_import_confirmed(
    app: AppHandle,
    state: State<AppState>,
    record_id: String,
    confirmed: bool,
) -> Result<RecordBrief, String> {
    let updated = state
        .library
        .repository()
        .set_import_confirmed(&record_id, confirmed)
        .map_err(|e| e.to_frontend())?;
    if confirmed {
        spawn_index_metadata_refresh(app, state.library.root().to_path_buf());
        schedule_memory_update(state.inner().clone());
    }
    Ok(updated)
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
    app: AppHandle,
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
    emit_knowledge_index_update(&app);

    let library_root = state.library.root().to_path_buf();
    let embedding_model = settings.embedding_model;
    let failure_scope_key = scope_key.clone();
    let app_state = state.inner().clone();
    let progress_app = app.clone();
    let heavy_jobs = crate::state::heavy_job_limiter().clone();
    std::thread::spawn(move || {
        let _permit = heavy_jobs.acquire();
        match ManagedLibrary::open(library_root.clone()) {
            Ok(library) => {
                let result = crate::knowledge::rebuild_scope(
                    &library,
                    project_id.as_deref(),
                    unfiled_only,
                    &mut || emit_knowledge_index_update(&progress_app),
                );
                if let Err(error) = result {
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
        emit_knowledge_index_update(&progress_app);
    });
    Ok(())
}

/// 记忆块索引状态（问知识库状态条与引用卡引导用）。
#[tauri::command]
pub fn get_memory_index_status(state: State<AppState>) -> Result<KnowledgeIndexStatus, String> {
    let settings = state
        .library
        .repository()
        .knowledge_settings()
        .map_err(|error| error.to_frontend())?;
    state
        .library
        .repository()
        .get_knowledge_index_status(
            crate::knowledge::MEMORY_SCOPE_KEY,
            &settings.embedding_model,
        )
        .map_err(|error| error.to_frontend())
}

/// 手动重建记忆块索引（确认反馈/快照生成会自动触发，这里是排障入口）。
#[tauri::command]
pub fn rebuild_memory_index(app: AppHandle, state: State<AppState>) -> Result<(), String> {
    let library_root = state.library.root().to_path_buf();
    spawn_memory_index_rebuild(Some(app), library_root);
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
    ticket: String,
) -> Result<String, String> {
    // 一次性票据：只有走过保存对话框的导出才签发了票。
    if !crate::state::consume_export_ticket(&ticket) {
        return Err("导出未经验证，请重新通过「另存为」对话框导出".to_owned());
    }
    crate::export::export_record(
        &state.library,
        &record_id,
        &PathBuf::from(destination_path),
        &format,
    )
    .map(|path| path.to_string_lossy().to_string())
    .map_err(|error| error.to_frontend())
}

/// 块7：导出包——单条记录全量交付（逐字稿 md+srt + 纪要 + manifest），目的地为用户
/// 通过文件夹选择器选定的目录。与既有目录导出同一道一次性票据门。
#[tauri::command]
pub fn export_record_package(
    state: State<AppState>,
    record_id: String,
    destination_path: String,
    ticket: String,
) -> Result<String, String> {
    if !crate::state::consume_export_ticket(&ticket) {
        return Err("导出未经验证，请重新通过导出对话框操作".to_owned());
    }
    crate::export::export_record_package(
        &state.library,
        &record_id,
        &PathBuf::from(destination_path),
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
    ticket: String,
) -> Result<String, String> {
    crate::knowledge::validate_scope(project_id.as_deref(), unfiled_only)
        .map_err(|error| error.to_frontend())?;
    if !crate::state::consume_export_ticket(&ticket) {
        return Err("导出未经验证，请重新通过「另存为」对话框导出".to_owned());
    }
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

/// v1.4 包7：批量导出包——对多条记录逐条调用单条导出包逻辑（同一目录下
/// 每条记录一个子文件夹）。单条失败不中断整体，结果如实回报成功/跳过明细。
/// 与单条导出共用一次性票据门（只有走过保存对话框的导出才签发）。
#[tauri::command]
pub fn export_record_packages(
    state: State<AppState>,
    record_ids: Vec<String>,
    destination_path: String,
    ticket: String,
) -> Result<crate::types::BulkExportResult, String> {
    if record_ids.is_empty() {
        return Err("请先选择要导出的记录".to_owned());
    }
    if record_ids.len() > 100 {
        return Err("一次最多导出 100 条记录，请分批选择".to_owned());
    }
    if !crate::state::consume_export_ticket(&ticket) {
        return Err("导出未经验证，请重新通过导出对话框操作".to_owned());
    }
    let destination = PathBuf::from(destination_path);
    let mut exported = Vec::new();
    let mut skipped = Vec::new();
    for record_id in &record_ids {
        match crate::export::export_record_package(&state.library, record_id, &destination) {
            Ok(folder) => exported.push(crate::types::ExportedPackage {
                record_id: record_id.clone(),
                folder: folder.to_string_lossy().to_string(),
            }),
            Err(error) => skipped.push(crate::types::SkippedPackage {
                record_id: record_id.clone(),
                reason: error.to_frontend(),
            }),
        }
    }
    Ok(crate::types::BulkExportResult {
        exported,
        skipped,
        destination: destination.to_string_lossy().to_string(),
    })
}

/// 前端在弹出保存对话框前调用，拿到一次性导出票据。
#[tauri::command]
pub fn create_export_ticket() -> String {
    crate::state::issue_export_ticket()
}

#[tauri::command]
pub fn update_record_knowledge_base(
    app: AppHandle,
    state: State<AppState>,
    record_id: String,
    knowledge_base_id: Option<String>,
) -> Result<RecordBrief, String> {
    let updated = state
        .library
        .repository()
        .update_record_project(&record_id, knowledge_base_id.as_deref())
        .map_err(|e| e.to_frontend())?;
    spawn_index_metadata_refresh(app, state.library.root().to_path_buf());
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
    candidates.extend(development_mcp_candidates());
    candidates.into_iter().find(|path| path.is_file())
}

/// 开发构建才带源码目录回退。必须用 `#[cfg]` 而不是 `cfg!()`：后者只是运行时判断，
/// `env!("CARGO_MANIFEST_DIR")` 的绝对路径照样会编进 release 二进制（发布审计会拦）。
#[cfg(debug_assertions)]
fn development_mcp_candidates() -> Vec<PathBuf> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    vec![
        manifest_dir.join("target/debug/echo-memory-mcp"),
        manifest_dir.join("target/release/echo-memory-mcp"),
    ]
}

#[cfg(not(debug_assertions))]
fn development_mcp_candidates() -> Vec<PathBuf> {
    Vec::new()
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
pub async fn import_audio(
    state: State<'_, AppState>,
    source_path: String,
    project_id: Option<String>,
    duplicate_confirmed: bool,
) -> Result<IngestResult, String> {
    // 大文件的哈希、时长探测与拷贝都是磁盘 IO，放在后台线程执行，
    // 否则导入 GB 级录音时主线程被占住，界面直接冻住。
    let library = state.inner().library.clone();
    tauri::async_runtime::spawn_blocking(move || {
        library
            .import_audio(
                &PathBuf::from(source_path),
                project_id.as_deref(),
                duplicate_confirmed,
            )
            .map_err(|e| e.to_frontend())
    })
    .await
    .map_err(|_| "导入后台任务异常".to_owned())?
}

#[tauri::command]
pub fn import_document(
    app: AppHandle,
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
        spawn_incremental_index(app.clone(), library_root.clone(), record_id.clone());
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

/// AI 对话导入：同步完成解析与落库，索引与分析在后台按记录串行执行，
/// 全部结束后调度一次记忆更新。逐会话的失败在 skipped 明细里返回。
#[tauri::command]
pub fn import_chat_exports(
    app: AppHandle,
    state: State<AppState>,
    source_paths: Vec<String>,
    project_id: Option<String>,
    duplicate_confirmed: bool,
) -> Result<ChatImportOutcome, String> {
    let paths = source_paths
        .into_iter()
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    if paths.is_empty() {
        return Err("未选择任何文件".to_owned());
    }
    let outcome = crate::chat_import::import_chat_exports(
        &state.library,
        &paths,
        project_id.as_deref(),
        duplicate_confirmed,
    )
    .map_err(|error| error.to_frontend())?;
    spawn_chat_import_pipeline(app, &state, &outcome);
    Ok(outcome)
}

/// 导入后的共享后台管线：逐记录增量索引 + 限流串行分析 + 一次记忆更新调度。
/// AI 对话（source_type=chat）走独立车道：只建检索索引，不做逐条分析，
/// 也不触发记忆链更新——它的结构化产出是「梳理报告」，不是记忆视图。
fn spawn_chat_import_pipeline(app: AppHandle, state: &AppState, outcome: &ChatImportOutcome) {
    let record_ids = outcome
        .imported
        .iter()
        .map(|item| item.record_id.clone())
        .collect::<Vec<_>>();
    if record_ids.is_empty() {
        return;
    }
    let repository = state.library.repository();
    let mut memory_chain_record_ids = Vec::new();
    for record_id in &record_ids {
        let is_chat = repository
            .get_record(record_id)
            .map(|record| record.source_type == "chat")
            .unwrap_or(false);
        if !is_chat {
            memory_chain_record_ids.push(record_id.clone());
        }
    }
    let library_root = state.library.root().to_path_buf();
    let app_state = state.clone();
    for record_id in &record_ids {
        spawn_incremental_index(app.clone(), library_root.clone(), record_id.clone());
    }
    let heavy_jobs = crate::state::heavy_job_limiter().clone();
    let has_memory_chain = !memory_chain_record_ids.is_empty();
    std::thread::spawn(move || {
        for record_id in memory_chain_record_ids {
            let _permit = heavy_jobs.acquire();
            match ManagedLibrary::open(library_root.clone()) {
                Ok(library) => {
                    if analyze_with_library(&library, &record_id).is_err() {
                        mark_processing_failed(
                            &library_root,
                            &record_id,
                            None,
                            "analyze",
                            "AI 对话分析失败",
                        );
                    }
                }
                Err(error) => mark_processing_failed(
                    &library_root,
                    &record_id,
                    None,
                    "analyze",
                    &error.to_string(),
                ),
            }
        }
        if has_memory_chain {
            schedule_memory_update(app_state);
        }
    });
}

/// 扫描本机 Agent 会话与记忆文件（白名单位置，纯本地只读）。
#[tauri::command]
pub fn scan_local_agents() -> Result<crate::agent_scan::AgentScanResult, String> {
    crate::agent_scan::scan_agent_sessions().map_err(|error| error.to_frontend())
}

/// AI 对话档案计数（知识库主页档案卡）。
#[tauri::command]
pub fn chat_archive_count(state: State<AppState>) -> Result<i64, String> {
    state
        .library
        .repository()
        .chat_record_count()
        .map_err(|error| error.to_frontend())
}

/// AI 对话档案梳理：把导入的全部 AI 对话聚合成一份结构化报告（外部 AI，双同意门禁）。
/// 报告以 chat 记录落库：可搜索、可被知识问答引用，不进录音记忆链。
#[tauri::command]
pub async fn generate_chat_digest(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<IngestResult, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let settings = crate::external_ai_gate::require_external_ai_consent(&state.library)
            .map_err(|error| error.to_frontend())?;
        if !settings.enabled {
            return Err("请先在设置中启用外部 AI".to_owned());
        }
        let api_key = crate::external_ai_gate::get_api_key()
            .map_err(|error| error.to_frontend())?
            .ok_or_else(|| "请先配置外部 AI API Key".to_owned())?;
        let client = crate::memory::OpenAiCompatibleClient::new(
            &settings.base_url,
            &settings.model,
            &api_key,
        )
        .map_err(|error| error.to_frontend())?;
        let result = crate::chat_digest::generate(&state.library, &client)
            .map_err(|error| error.to_frontend())?;
        if !result.duplicate {
            spawn_incremental_index(
                app,
                state.library.root().to_path_buf(),
                result.record_id.clone(),
            );
        }
        Ok(result)
    })
    .await
    .map_err(|_| "梳理报告后台任务异常".to_owned())?
}

/// 导入勾选的本机 Agent 会话与记忆/规则文件。
/// 所有路径先过白名单校验，杜绝任意路径读取。
#[tauri::command]
pub fn import_agent_scans(
    app: AppHandle,
    state: State<AppState>,
    sessions: Vec<AgentSessionSelection>,
    memory_paths: Vec<String>,
    project_id: Option<String>,
    duplicate_confirmed: bool,
) -> Result<ChatImportOutcome, String> {
    if sessions.is_empty() && memory_paths.is_empty() {
        return Err("未选择任何内容".to_owned());
    }
    let mut selections = Vec::new();
    for session in &sessions {
        let Some(agent) = crate::agent_scan::AgentKind::parse(&session.agent) else {
            return Err(format!("未知的 Agent 类型：{}", session.agent));
        };
        if !crate::agent_scan::is_session_path_allowed(agent, &session.path) {
            return Err(format!("路径不在允许的扫描范围内：{}", session.path));
        }
        selections.push((agent, session.path.clone()));
    }
    for path in &memory_paths {
        if !crate::agent_scan::is_memory_path_allowed(path) {
            return Err(format!("路径不在允许的扫描范围内：{path}"));
        }
    }

    let mut outcome = if selections.is_empty() {
        ChatImportOutcome {
            imported: Vec::new(),
            skipped: Vec::new(),
        }
    } else {
        crate::chat_import::import_agent_sessions(
            &state.library,
            &selections,
            project_id.as_deref(),
            duplicate_confirmed,
        )
        .map_err(|error| error.to_frontend())?
    };
    for path in &memory_paths {
        match crate::document::import_document(
            &state.library,
            Path::new(path),
            project_id.as_deref(),
            duplicate_confirmed,
        ) {
            Ok(result) => outcome.imported.push(result),
            Err(error) => outcome.skipped.push(crate::types::ChatImportSkip {
                file: path.clone(),
                title: None,
                reason: error.to_string(),
            }),
        }
    }
    spawn_chat_import_pipeline(app, &state, &outcome);
    Ok(outcome)
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
    app: AppHandle,
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
        app,
        state.library.root().to_path_buf(),
        segment.record_id.clone(),
    );
    schedule_memory_update(state.inner().clone());
    Ok(segment)
}

#[tauri::command]
pub fn transcribe_record(
    app: AppHandle,
    state: State<AppState>,
    record_id: String,
) -> Result<(), String> {
    // 与 retranscribe_record 同一道在途检查：收件箱自动导入与手动触发可能
    // 撞在同一条记录上，两个线程会共享同一个预处理文件和 whisperX JSON，
    // 互相删除对方正在读的文件。
    if state
        .library
        .repository()
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
    let job = reserve_transcription(&state.library, &record_id).map_err(|e| e.to_frontend())?;
    emit_processing_progress(&app, &record_id);
    let settings = state
        .library
        .repository()
        .knowledge_settings()
        .map_err(|error| error.to_frontend())?;
    let library_root = state.library.root().to_path_buf();
    let app_state = state.inner().clone();
    let progress_app = app.clone();
    let heavy_jobs = crate::state::heavy_job_limiter().clone();
    std::thread::spawn(move || {
        let _permit = heavy_jobs.acquire();
        match ManagedLibrary::open(library_root.clone()) {
            Ok(library) => {
                let transcription = run_pipeline_with_finite_retry(
                    Some(&progress_app),
                    &library,
                    &record_id,
                    job.clone(),
                    &TRANSCRIPTION_RETRY_DELAYS,
                    |attempt_job| {
                        process_record_with_transcription_job(
                            Some(&progress_app),
                            &library,
                            &record_id,
                            attempt_job,
                            &settings,
                        )
                    },
                );
                if transcription.is_ok() {
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
        emit_processing_progress(&progress_app, &record_id);
    });
    Ok(())
}

/// Runs the local ASR job and persists its normalized segments.
/// Keeping this outside the Tauri wrapper makes the full import-to-database path testable.
pub fn transcribe_with_library(library: &ManagedLibrary, record_id: &str) -> AppResult<()> {
    let job = reserve_transcription(library, record_id)?;
    let settings = library.repository().knowledge_settings()?;
    run_transcription(
        None,
        library,
        record_id,
        &job,
        &settings.transcription_language,
        Some(&settings.whisper_model_path),
    )
}

/// Executes the complete ingest pipeline for a newly imported audio record.
/// Manual imports and watched folders share this entry point so their final state
/// cannot diverge when a later processing stage changes.
pub fn process_record_with_library(library: &ManagedLibrary, record_id: &str) -> AppResult<()> {
    let job = reserve_transcription(library, record_id)?;
    let settings = library.repository().knowledge_settings()?;
    run_pipeline_with_finite_retry(
        None,
        library,
        record_id,
        job,
        &TRANSCRIPTION_RETRY_DELAYS,
        |attempt_job| {
            process_record_with_transcription_job(None, library, record_id, attempt_job, &settings)
        },
    )
}

/* ------------------------- v0.9：转写失败有限自动重试 ------------------------- */

/// 自动重试上限与退避节奏（60 秒、180 秒）：两次之后交还用户手动重试。
const TRANSCRIPTION_AUTO_RETRIES: usize = 2;
const TRANSCRIPTION_RETRY_DELAYS: [std::time::Duration; TRANSCRIPTION_AUTO_RETRIES] = [
    std::time::Duration::from_secs(60),
    std::time::Duration::from_secs(180),
];

/// 哪些失败值得自动重试（白名单 + 永久类关键词，默认不重试）。
///
/// 确定性失败（模型未安装、格式不支持、退化检测、磁盘不足、未配置凭据）重试只会白等；
/// 只兜 I/O 抖动、Ollama/第三方服务暂时不可用、预处理进程偶发失败这类可恢复类别。
fn transcription_failure_is_retryable(error: &AppError) -> bool {
    const PERMANENT_MARKERS: [&str; 14] = [
        "未安装",
        "尚未安装",
        "不存在",
        "不支持",
        "仅支持",
        "退化",
        "校验",
        "无效",
        "损坏",
        "磁盘可用空间不足",
        "API Key",
        "未启用",
        "同意",
        "尚未配置",
    ];
    let message = error.to_string();
    if PERMANENT_MARKERS
        .iter()
        .any(|marker| message.contains(marker))
    {
        return false;
    }
    match error {
        AppError::Io(_) => true,
        AppError::Analysis(_) | AppError::ExternalAi(_) => true,
        // 预处理失败（增强与系统兼容两条路都倒下）多为瞬时资源问题；格式类错误已在上面的关键词里拦截。
        AppError::Import(message) => message.contains("预处理"),
        _ => false,
    }
}

/// 转写流水线的有限自动重试（只针对转写阶段）。
///
/// 用「逐字稿版本是否新增」区分失败阶段：转写成功后再失败的是分析，重跑转写只会白烧
/// 时间，因此这种情况不重试。重试沿用同一个 job（attempt_count 继续累加），
/// 失败期间 last_error 会带上「N 秒后自动重试」，前端轮询可见。
fn run_pipeline_with_finite_retry<F>(
    progress_app: Option<&AppHandle>,
    library: &ManagedLibrary,
    record_id: &str,
    first_job: ProcessingJob,
    delays: &[std::time::Duration],
    mut attempt_once: F,
) -> AppResult<()>
where
    F: FnMut(&ProcessingJob) -> AppResult<()>,
{
    let mut job = first_job;
    let mut retries = 0usize;
    loop {
        let before = library
            .repository()
            .latest_transcript_version(record_id)
            .ok()
            .map(|version| version.id);
        match attempt_once(&job) {
            Ok(()) => return Ok(()),
            Err(error) => {
                let after = library
                    .repository()
                    .latest_transcript_version(record_id)
                    .ok()
                    .map(|version| version.id);
                let transcription_succeeded = after.is_some() && after != before;
                if transcription_succeeded
                    || retries >= delays.len()
                    || !transcription_failure_is_retryable(&error)
                {
                    return Err(error);
                }
                let delay = delays[retries];
                retries += 1;
                let wait_note = format!("{error}（{} 秒后自动重试）", delay.as_secs());
                let repository = library.repository();
                repository.update_job_status(&job.id, "failed", Some(&wait_note))?;
                if let Some(app) = progress_app {
                    emit_processing_progress(app, record_id);
                }
                std::thread::sleep(delay);
                // 重试走同一 job：置回 preparing 会继续累加 attempt_count。
                repository.update_job_status(&job.id, "preparing", None)?;
                repository.update_record_status(record_id, "preparing")?;
                job = repository.get_job(&job.id)?;
                if let Some(app) = progress_app {
                    emit_processing_progress(app, record_id);
                }
            }
        }
    }
}

/// 启动自动重排：把「上次应用退出中断」的转写任务按同一条带重试的管线重跑一次。
/// 串行执行（每条占一个重任务额度），逐条 emit 进度让界面如实反映状态。
pub fn resume_interrupted_transcriptions(app: AppHandle, library_root: PathBuf) {
    std::thread::spawn(move || {
        let Ok(library) = ManagedLibrary::open(library_root) else {
            return;
        };
        let Ok(candidates) = library.repository().interrupted_transcriptions() else {
            return;
        };
        for (record_id, job_id) in candidates {
            let Ok(job) = library.repository().get_job(&job_id) else {
                continue;
            };
            let Ok(settings) = library.repository().knowledge_settings() else {
                continue;
            };
            let repository = library.repository();
            if repository
                .update_job_status(&job_id, "preparing", None)
                .is_err()
            {
                continue;
            }
            let _ = repository.update_record_status(&record_id, "preparing");
            let _permit = crate::state::heavy_job_limiter().acquire();
            emit_processing_progress(&app, &record_id);
            let _ = run_pipeline_with_finite_retry(
                Some(&app),
                &library,
                &record_id,
                job,
                &TRANSCRIPTION_RETRY_DELAYS,
                |attempt_job| {
                    process_record_with_transcription_job(
                        Some(&app),
                        &library,
                        &record_id,
                        attempt_job,
                        &settings,
                    )
                },
            );
            emit_processing_progress(&app, &record_id);
        }
    });
}

fn process_record_with_transcription_job(
    progress_app: Option<&AppHandle>,
    library: &ManagedLibrary,
    record_id: &str,
    job: &ProcessingJob,
    settings: &crate::types::KnowledgeSettings,
) -> AppResult<()> {
    // 第三方通道未配齐时不再拦死导入：转录分流会自动回退内置 Whisper
    //（external_asr_channel_ready），分析阶段由 analyze_with_library 自行判定通道。
    run_transcription(
        progress_app,
        library,
        record_id,
        job,
        &settings.transcription_language,
        Some(&settings.whisper_model_path),
    )?;
    analyze_with_library(library, record_id)
}

#[tauri::command]
pub fn retranscribe_record(
    app: AppHandle,
    state: State<AppState>,
    record_id: String,
    language: String,
    model_path: Option<String>,
    quality_preset: Option<String>,
    engine: Option<String>,
) -> Result<(), String> {
    if let Some(engine) = engine.as_deref() {
        if !matches!(engine, "embedded" | "whisperx") {
            return Err("转写引擎无效".to_owned());
        }
    }
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
    emit_processing_progress(&app, &record_id);
    let library_root = state.library.root().to_path_buf();
    let app_state = state.inner().clone();
    let progress_app = app.clone();
    let heavy_jobs = crate::state::heavy_job_limiter().clone();
    std::thread::spawn(move || {
        let _permit = heavy_jobs.acquire();
        match ManagedLibrary::open(library_root.clone()) {
            Ok(library) => {
                let transcription = run_pipeline_with_finite_retry(
                    Some(&progress_app),
                    &library,
                    &record_id,
                    job.clone(),
                    &TRANSCRIPTION_RETRY_DELAYS,
                    |attempt_job| {
                        run_transcription_with_engine(
                            Some(&progress_app),
                            &library,
                            &record_id,
                            attempt_job,
                            &language,
                            model_path.as_deref(),
                            engine.as_deref(),
                        )
                    },
                );
                if transcription.is_ok() {
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
        emit_processing_progress(&progress_app, &record_id);
    });
    Ok(())
}

/// whisperX 外置引擎路径：整段转写 + 说话人分离，跳过内嵌引擎的分块循环。
fn run_whisperx_transcription(
    progress_app: Option<&AppHandle>,
    library: &ManagedLibrary,
    record_id: &str,
    job: &ProcessingJob,
    preprocessed: &std::path::Path,
    language: &str,
) -> AppResult<()> {
    let repository = library.repository();
    let binary = crate::whisper::whisperx_path().ok_or_else(|| {
        crate::error::AppError::Invalid(
            "已选择 whisperX 引擎，但未检测到 whisperx 命令。请先 pip install whisperx，或在设置中切回内嵌引擎。".to_owned(),
        )
    })?;
    repository.update_job_progress(&job.id, "preprocessing", 0, 1)?;
    let relative_audio = repository.audio_path_for_record(record_id)?;
    let audio_path = library.root().join(relative_audio);
    let output_dir = library.root().join("raw").join(record_id);
    std::fs::create_dir_all(&output_dir)?;
    ensure_preprocess_space(&audio_path, preprocessed)?;
    let metadata = preprocess(&audio_path, preprocessed)?;
    repository.update_job_status(&job.id, "transcribing", None)?;
    repository.update_record_status(record_id, "transcribing")?;
    if let Some(app) = progress_app {
        emit_processing_progress(app, record_id);
    }
    let hf_token = crate::memory::get_hf_token().ok().flatten();
    let segments =
        crate::whisper::transcribe_whisperx(&binary, preprocessed, language, hf_token.as_deref())?;
    let inputs = segments
        .iter()
        .map(|segment| TranscriptSegmentInput {
            start_ms: (segment.start * 1000.0) as i64,
            end_ms: (segment.end * 1000.0) as i64,
            speaker_label: Some(localize_speaker_label(segment.speaker.as_deref())),
            original_text: segment.text.trim().to_owned(),
        })
        .collect::<Vec<_>>();
    if inputs.is_empty() {
        return Err(crate::error::AppError::Import(
            "whisperX 没有生成可用片段".to_owned(),
        ));
    }
    // 与内嵌引擎同一道质量闸门：退化输出不落库，保留此前版本。
    let degeneration = crate::degeneration::assess_transcript_degeneration(
        &inputs
            .iter()
            .map(|segment| segment.original_text.clone())
            .collect::<Vec<_>>(),
        language,
    );
    if degeneration.degenerate {
        return Err(crate::error::AppError::Import(format!(
            "转写质量异常（{}），已保留此前的转写版本。如反复出现，请更换转写模型或切换语言后重试。",
            degeneration.reasons.join("；")
        )));
    }
    repository.update_job_progress(&job.id, "saving", 1, 1)?;
    let metadata_json = serde_json::to_string(&metadata)
        .map_err(|error| crate::error::AppError::Import(format!("预处理信息无效：{error}")))?;
    repository.save_transcript_with_metadata(
        record_id,
        "whisperx",
        "whisperx-large-v2",
        language,
        "whisperx-v1",
        &metadata_json,
        &inputs,
    )?;
    repository.update_job_status(&job.id, "completed", None)?;
    repository.update_record_status(record_id, "completed")?;
    if let Some(app) = progress_app {
        emit_processing_progress(app, record_id);
        spawn_incremental_index(
            app.clone(),
            library.root().to_path_buf(),
            record_id.to_owned(),
        );
    }
    Ok(())
}

/// 预处理会把音频放大成 16kHz 单声道 WAV：写前确认目标卷有余量。
/// 三个转写引擎（内嵌 / whisperX / 第三方）共用这一道预检，避免各自漏检。
fn ensure_preprocess_space(
    audio_path: &std::path::Path,
    output: &std::path::Path,
) -> AppResult<()> {
    let input_bytes = std::fs::metadata(audio_path).map_or(0, |meta| meta.len());
    let target = output.parent().unwrap_or(audio_path);
    crate::disk_space::ensure_free_space(
        target,
        crate::disk_space::required_preprocess_bytes(input_bytes),
    )
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
    progress_app: Option<&AppHandle>,
    library: &ManagedLibrary,
    record_id: &str,
    job: &ProcessingJob,
    language: &str,
    model_path: Option<&str>,
) -> AppResult<()> {
    run_transcription_with_engine(
        progress_app,
        library,
        record_id,
        job,
        language,
        model_path,
        None,
    )
}

fn run_transcription_with_engine(
    progress_app: Option<&AppHandle>,
    library: &ManagedLibrary,
    record_id: &str,
    job: &ProcessingJob,
    language: &str,
    model_path: Option<&str>,
    engine_override: Option<&str>,
) -> AppResult<()> {
    let repository = library.repository();
    // 只有第三方转写通道自身配齐才走第三方；processing_mode 只是通道开关之一，
    // 缺同意/Key/地址时自动回退内置 Whisper，不在导入中途拦死。
    if engine_override.is_none() && crate::external_ai_gate::external_asr_channel_ready(library)? {
        return run_external_transcription(progress_app, library, record_id, job, language);
    }
    // 用户选了第三方转写但没配齐：回退内置是正确行为，但不应无声无息——
    // 发一次轻量事件让界面提示去补配；从未配置过的通道保持静默。
    if engine_override.is_none()
        && crate::external_ai_gate::external_asr_channel_half_configured(library)?
    {
        if let Some(app) = progress_app {
            let _ = app.emit(
                "transcription-fallback-notice",
                serde_json::json!({
                    "recordId": record_id,
                    "message": "第三方转写还没配齐（缺 API Key 或未确认音频上传说明），本次先用本机 Whisper 转写。要补配请到 设置 → AI 模型 → 第三方处理。"
                }),
            );
        }
    }
    let preprocessed = library
        .root()
        .join("raw")
        .join(record_id)
        .join(format!("{}-enhanced.wav", job.id));
    let result = (|| {
        let engine = engine_override
            .map(str::to_owned)
            .or_else(|| {
                repository
                    .setting_value("transcription_engine")
                    .ok()
                    .flatten()
            })
            .unwrap_or_else(|| "embedded".to_owned());
        if engine == "whisperx" {
            return run_whisperx_transcription(
                progress_app,
                library,
                record_id,
                job,
                &preprocessed,
                language,
            );
        }
        let adapter = WhisperAdapter::detect_with_model_path(model_path)?;
        let relative_audio = repository.audio_path_for_record(record_id)?;
        let audio_path = library.root().join(relative_audio);
        let output_dir = library.root().join("raw").join(record_id);
        std::fs::create_dir_all(&output_dir)?;
        repository.update_job_progress(&job.id, "preprocessing", 0, 1)?;
        ensure_preprocess_space(&audio_path, &preprocessed)?;
        let metadata = preprocess(&audio_path, &preprocessed)?;
        repository.update_job_progress(&job.id, "chunking", 1, 1)?;
        let samples = read_normalized_wav(&preprocessed)?;
        let chunks = plan_chunks(&samples);
        repository.update_job_status(&job.id, "transcribing", None)?;
        repository.update_record_status(record_id, "transcribing")?;
        if let Some(app) = progress_app {
            emit_processing_progress(app, record_id);
        }
        let hotword_prefix = repository.hotwords_prompt().unwrap_or_default();
        // 模型只加载一次：每个音频块共用同一个 context，解码状态每块新建
        // （whisper.cpp 的 state 有跨调用残留，复用会改变转写结果）。
        let session = WhisperSession::load(adapter.model_path().ok_or_else(|| {
            crate::error::AppError::Import("找不到 Whisper 模型文件".to_owned())
        })?)?;
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
            let chunk_segments = session.transcribe(
                &samples[chunk.sample_start..chunk.sample_end],
                language,
                (!initial_prompt.is_empty()).then_some(initial_prompt.as_str()),
            )?;
            for mut segment in chunk_segments {
                segment.start_ms += offset_ms;
                segment.end_ms += offset_ms;
                let midpoint = segment.start_ms + (segment.end_ms - segment.start_ms) / 2;
                if midpoint >= chunk.accept_start_ms && midpoint <= chunk.accept_end_ms {
                    // 重叠区里同一句话被两块各解一次：上一条的结尾和本条的开头
                    // 说的是同一段音频。先尝试接续合并，合并不了再按重复段丢弃。
                    let merged = accepted
                        .iter_mut()
                        .rev()
                        .take(OVERLAP_MERGE_SCAN)
                        .any(|previous| merge_overlap_continuation(previous, &segment));
                    if merged {
                        continue;
                    }
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
        normalize_segment_timestamps(&mut accepted);
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
        // 质量闸门：退化输出不落库。失败路径会保留此前的转写版本并把记录
        // 状态回到 completed——最坏结果是显式失败，而不是静默的数据损坏。
        let degeneration = crate::degeneration::assess_transcript_degeneration(
            &inputs
                .iter()
                .map(|segment| segment.original_text.clone())
                .collect::<Vec<_>>(),
            language,
        );
        if degeneration.degenerate {
            return Err(crate::error::AppError::Import(format!(
                "转写质量异常（{}），已保留此前的转写版本。如反复出现，请在设置里更换转写模型或切换语言后重试。",
                degeneration.reasons.join("；")
            )));
        }
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
        if let Some(app) = progress_app {
            emit_processing_progress(app, record_id);
            spawn_incremental_index(
                app.clone(),
                library.root().to_path_buf(),
                record_id.to_owned(),
            );
        }
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
        if let Some(app) = progress_app {
            emit_processing_progress(app, record_id);
        }
        return Err(error);
    }
    // v1.1 块3：内嵌 whisper.cpp 与 whisperX 两条路径在此汇合（外部 ASR 在
    // run_external_transcription 里提前返回、有自己的同款钩子）。转写成功 →
    // 纪要自动入队（后台线程），失败只记 minutes.failed 可重试，
    // 绝不阻塞/影响录音与逐字稿本身。
    crate::minutes::maybe_autogenerate_minutes(
        library.root().to_path_buf(),
        record_id.to_owned(),
    );
    Ok(())
}

fn run_external_transcription(
    progress_app: Option<&AppHandle>,
    library: &ManagedLibrary,
    record_id: &str,
    job: &ProcessingJob,
    language: &str,
) -> AppResult<()> {
    let repository = library.repository();
    let preprocessed = library
        .root()
        .join("raw")
        .join(record_id)
        .join(format!("{}-external.wav", job.id));
    let result = (|| {
        let audio_path = repository.audio_path_for_record(record_id)?;
        let record = repository.get_record(record_id)?;
        let external_result = crate::external_asr::transcribe_with_transport(
            library,
            &audio_path,
            language,
            record.audio_duration_ms,
            |mut request| {
                repository.update_job_progress(&job.id, "preprocessing", 0, 1)?;
                ensure_preprocess_space(&request.audio_path, &preprocessed)?;
                let metadata = preprocess(&request.audio_path, &preprocessed)?;
                request.audio_path = preprocessed.clone();
                repository.update_job_progress(&job.id, "transcribing", 0, 1)?;
                repository.update_job_status(&job.id, "transcribing", None)?;
                repository.update_record_status(record_id, "transcribing")?;
                if let Some(app) = progress_app {
                    emit_processing_progress(app, record_id);
                }
                let cache_dir = library
                    .root()
                    .join("raw")
                    .join(record_id)
                    .join("external-asr-cache")
                    .join(&job.id);
                let segments = crate::external_asr::transcribe_long_audio(
                    &request,
                    &cache_dir,
                    |current, total| {
                        repository.update_job_progress(
                            &job.id,
                            "transcribing",
                            current as i64,
                            total as i64,
                        )?;
                        if let Some(app) = progress_app {
                            emit_processing_progress(app, record_id);
                        }
                        Ok(())
                    },
                )?;
                let preprocessing_json = serde_json::to_string(&metadata).map_err(|error| {
                    crate::error::AppError::Import(format!("预处理信息无效：{error}"))
                })?;
                Ok(crate::external_asr::ExternalAsrResult {
                    segments,
                    preprocessing_json,
                })
            },
        )?;
        if external_result.segments.is_empty() {
            return Err(crate::error::AppError::Import(
                "第三方转写没有生成可用片段".to_owned(),
            ));
        }
        let degeneration = crate::degeneration::assess_transcript_degeneration(
            &external_result
                .segments
                .iter()
                .map(|segment| segment.original_text.clone())
                .collect::<Vec<_>>(),
            language,
        );
        if degeneration.degenerate {
            return Err(crate::error::AppError::Import(format!(
                "转写质量异常（{}），已保留此前的转写版本。如反复出现，请更换转写模型或切换语言后重试。",
                degeneration.reasons.join("；")
            )));
        }
        let external_settings = repository.external_ai_settings(false, false)?;
        repository.update_job_progress(&job.id, "saving", 1, 1)?;
        repository.save_transcript_with_metadata(
            record_id,
            &external_settings.transcription_provider,
            &external_settings.transcription_model,
            language,
            "external-asr-v1",
            &external_result.preprocessing_json,
            &external_result.segments,
        )?;
        let _ = std::fs::remove_dir_all(
            library
                .root()
                .join("raw")
                .join(record_id)
                .join("external-asr-cache")
                .join(&job.id),
        );
        repository.update_job_status(&job.id, "completed", None)?;
        repository.update_record_status(record_id, "completed")?;
        if let Some(app) = progress_app {
            emit_processing_progress(app, record_id);
            spawn_incremental_index(
                app.clone(),
                library.root().to_path_buf(),
                record_id.to_owned(),
            );
        }
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
        if let Some(app) = progress_app {
            emit_processing_progress(app, record_id);
        }
        return Err(error);
    }
    // v1.1 块3：转写成功 → 纪要自动入队（后台线程）。生成失败只记 minutes
    // 的 failed 状态可重试，绝不阻塞/影响录音与逐字稿本身（录音先入库）。
    crate::minutes::maybe_autogenerate_minutes(
        library.root().to_path_buf(),
        record_id.to_owned(),
    );
    Ok(())
}

fn mark_processing_failed(
    library_root: &Path,
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
    library_root: &Path,
    scope_key: &str,
    embedding_model: &str,
    error: &str,
) {
    let Ok(repository) = LibraryRepository::new(library_root.join("memory.db")) else {
        return;
    };
    let _ = repository.mark_knowledge_index_failed(scope_key, embedding_model, error);
}

fn mark_knowledge_indexes_failed(library_root: &Path, error: &str) {
    let Ok(repository) = LibraryRepository::new(library_root.join("memory.db")) else {
        return;
    };
    let _ = repository.mark_knowledge_indexes_failed(error);
}

fn spawn_incremental_index(app: AppHandle, library_root: PathBuf, record_id: String) {
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
        emit_knowledge_index_update(&app);
    });
}

pub(crate) fn spawn_index_metadata_refresh(app: AppHandle, library_root: PathBuf) {
    std::thread::spawn(move || {
        if let Ok(library) = ManagedLibrary::open(library_root) {
            let repository = library.repository();
            if let Ok(settings) = repository.knowledge_settings() {
                let _ = repository.refresh_knowledge_index_counts(&settings.embedding_model);
            }
        }
        emit_knowledge_index_update(&app);
    });
}

#[tauri::command]
pub fn analyze_record(
    app: AppHandle,
    state: State<AppState>,
    record_id: String,
    template_id: Option<String>,
    instruction: Option<String>,
) -> Result<(), String> {
    if let Some(template_id) = template_id.as_deref() {
        state
            .library
            .repository()
            .set_record_analysis_template(&record_id, template_id)
            .map_err(|error| error.to_frontend())?;
    }
    let instruction = crate::analysis::normalize_analysis_instruction(instruction.as_deref());
    let job = reserve_analysis(&state.library, &record_id).map_err(|e| e.to_frontend())?;
    emit_processing_progress(&app, &record_id);
    let library_root = state.library.root().to_path_buf();
    let app_state = state.inner().clone();
    let progress_app = app.clone();
    let heavy_jobs = crate::state::heavy_job_limiter().clone();
    std::thread::spawn(move || {
        let _permit = heavy_jobs.acquire();
        match ManagedLibrary::open(library_root.clone()) {
            Ok(library) => {
                if run_analysis(
                    Some(&progress_app),
                    &library,
                    &record_id,
                    &job,
                    instruction.as_deref(),
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
                    "analyze",
                    &error.to_string(),
                );
            }
        }
        emit_processing_progress(&progress_app, &record_id);
    });
    Ok(())
}

/// Runs the same local analysis path synchronously for explicit real-data regression tests.
pub fn analyze_with_library(library: &ManagedLibrary, record_id: &str) -> AppResult<()> {
    let job = reserve_analysis(library, record_id)?;
    run_analysis(None, library, record_id, &job, None)
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
    draft.quality_severity = None;
    let template = stored
        .template_id
        .as_deref()
        .and_then(|id| repository.get_analysis_template(id).ok())
        .or_else(|| serde_json::from_str(&stored.template_snapshot_json).ok())
        .unwrap_or(repository.get_analysis_template("builtin-standard")?);
    let verified = crate::analysis::finalize_analysis_draft(
        &draft,
        &segments,
        Some(stored.analysis_instruction.as_str()),
        Some(&template),
    );
    if crate::analysis::analysis_is_blocking(&verified) {
        return Err(crate::error::AppError::Analysis(
            "仍有分析条目无法匹配可靠出处".to_owned(),
        ));
    }
    repository.save_analysis_with_provider_and_template(
        &stored.record_id,
        &stored.source_transcript_version_id,
        &stored.provider,
        &stored.model,
        &verified,
        &template,
        &stored.analysis_instruction,
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

fn run_analysis(
    progress_app: Option<&AppHandle>,
    library: &ManagedLibrary,
    record_id: &str,
    job: &ProcessingJob,
    instruction: Option<&str>,
) -> AppResult<()> {
    let repository = library.repository();
    let result = (|| {
        repository.update_job_progress(&job.id, "analyzing", 0, 1)?;
        // 分析通道独立判定：走云端当且仅当理解整理已启用、隐私同意已给。
        // Key 存在性在门禁后确认；processing_mode 不再参与本判定。
        let ext_settings = repository.external_ai_settings(false, false)?;
        let external_client = if ext_settings.enabled && ext_settings.privacy_consent_at.is_some()
        {
            let settings = require_external_ai_consent(library)?;
            let api_key = get_api_key()?.ok_or_else(|| {
                crate::error::AppError::Invalid("请先配置第三方文本理解 API Key".to_owned())
            })?;
            Some(OpenAiCompatibleClient::new(
                &settings.base_url,
                &settings.model,
                &api_key,
            )?)
        } else {
            None
        };
        let version = repository.latest_transcript_version(record_id)?;
        let segments = repository.list_transcript_segments(record_id)?;
        let transcript = segments
            .iter()
            .map(|segment| {
                let speaker = segment.speaker_label.as_deref().unwrap_or("");
                if !speaker.is_empty() && speaker != "未知" {
                    format!(
                        "[{}][{}] {}",
                        segment.sequence,
                        speaker,
                        effective_text(segment)
                    )
                } else {
                    format!("[{}] {}", segment.sequence, effective_text(segment))
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        let record = repository.get_record(record_id)?;
        let template = repository.get_analysis_template(
            record
                .analysis_template_id
                .as_deref()
                .unwrap_or("builtin-standard"),
        )?;
        let progress_job_id = job.id.clone();
        let progress_repository = library.repository();
        let progress = |current: usize, total: usize| {
            progress_repository.update_job_progress(
                &progress_job_id,
                "analyzing",
                current as i64,
                total as i64,
            )?;
            Ok(())
        };
        let (provider, model, draft) = if let Some(client) = external_client {
            let generator = ExternalAnalysisGenerator { client: &client };
            let draft = analyze_with_generator_instruction_progress(
                &generator,
                &transcript,
                Some(&template),
                instruction,
                progress,
            )?;
            let external_settings = repository.external_ai_settings(false, false)?;
            ("external-openai-compatible", external_settings.model, draft)
        } else {
            let model = repository.knowledge_settings()?.analysis_model;
            let draft = OllamaAdapter::detect(&model)?.analyze_with_template_instruction_progress(
                &transcript,
                Some(&template),
                instruction,
                progress,
            )?;
            ("ollama", model, draft)
        };
        repository.update_job_progress(&job.id, "validating", 1, 1)?;
        // 统一收口：别名解析 → 「只保留 X」栏目裁剪 → 引用逐字校验（fail-closed）。
        let verified = crate::analysis::finalize_analysis_draft(
            &draft,
            &segments,
            instruction,
            Some(&template),
        );
        repository.update_job_progress(&job.id, "saving", 1, 1)?;
        repository.save_analysis_with_provider_and_template(
            record_id,
            &version.id,
            provider,
            &model,
            &verified,
            &template,
            instruction.unwrap_or_default(),
        )?;
        repository.update_job_status(&job.id, "completed", None)?;
        repository.update_record_status(record_id, "completed")?;
        if let Some(app) = progress_app {
            emit_processing_progress(app, record_id);
        }
        if crate::dock::auto_export_enabled(library.repository())? {
            let _ = crate::dock::export_analysis_to_output(library, record_id);
        }
        if let Some(app) = progress_app {
            spawn_incremental_index(
                app.clone(),
                library.root().to_path_buf(),
                record_id.to_owned(),
            );
        }
        Ok::<(), crate::error::AppError>(())
    })();
    if let Err(error) = result {
        let message = error.to_string();
        let _ = repository.update_job_status(&job.id, "failed", Some(&message));
        let _ = repository.update_record_status(record_id, "failed");
        if let Some(app) = progress_app {
            emit_processing_progress(app, record_id);
        }
        return Err(error);
    }
    Ok(())
}

struct ExternalAnalysisGenerator<'a> {
    client: &'a OpenAiCompatibleClient,
}

impl AnalysisTextGenerator for ExternalAnalysisGenerator<'_> {
    fn generate_text(&self, prompt: &str) -> AppResult<String> {
        self.client.complete_text(
            "你是结构化录音分析器。仅返回约定的简体中文 JSON，不要使用 Markdown 或额外说明。",
            prompt,
        )
    }
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

pub(crate) fn schedule_memory_update(state: AppState) {
    let revision = state.schedule_memory_update();
    std::thread::spawn(move || {
        std::thread::sleep(AUTO_MEMORY_UPDATE_DELAY);
        if !state.is_latest_memory_update(revision) {
            return;
        }
        if require_external_ai_consent(&state.library).is_err() {
            return;
        }
        let Ok(settings) = external_settings(&state.library) else {
            return;
        };
        if !settings.enabled || !settings.has_api_key {
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
                Ok(snapshot) => {
                    state.finish_generation_job(&generation_id, &snapshot);
                    spawn_memory_index_rebuild(None, state.library.root().to_path_buf());
                }
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
pub fn get_provider_catalog() -> ProviderCatalog {
    provider_catalog::provider_catalog()
}

#[tauri::command]
pub fn get_external_ai_settings(state: State<AppState>) -> Result<ExternalAiSettings, String> {
    external_settings(&state.library).map_err(|error| error.to_frontend())
}

#[tauri::command]
pub fn update_external_ai_settings(
    state: State<AppState>,
    settings: ExternalAiSettings,
) -> Result<ExternalAiSettings, String> {
    let has_api_key = credentials_are_complete(
        &settings.analysis_provider,
        get_analysis_credentials()
            .map_err(|error| error.to_frontend())?
            .as_ref(),
    );
    let transcription_has_api_key = credentials_are_complete(
        &settings.transcription_provider,
        get_asr_credentials()
            .map_err(|error| error.to_frontend())?
            .as_ref(),
    );
    state
        .library
        .repository()
        .update_external_ai_settings(&settings, has_api_key, transcription_has_api_key)
        .map_err(|error| error.to_frontend())
}

#[tauri::command]
pub fn set_external_ai_api_key(
    state: State<AppState>,
    api_key: String,
) -> Result<ExternalAiSettings, String> {
    let settings = external_settings(&state.library).map_err(|error| error.to_frontend())?;
    set_analysis_credentials(
        &settings.analysis_provider,
        BTreeMap::from([("apiKey".to_owned(), api_key)]),
    )
    .map_err(|error| error.to_frontend())?;
    bump_revision_for_target(&state, "analysis");
    external_settings(&state.library).map_err(|error| error.to_frontend())
}

#[tauri::command]
pub fn clear_external_ai_api_key(state: State<AppState>) -> Result<ExternalAiSettings, String> {
    clear_analysis_credentials().map_err(|error| error.to_frontend())?;
    external_settings(&state.library).map_err(|error| error.to_frontend())
}

#[tauri::command]
pub fn set_external_asr_api_key(
    state: State<AppState>,
    api_key: String,
) -> Result<ExternalAiSettings, String> {
    let settings = external_settings(&state.library).map_err(|error| error.to_frontend())?;
    set_transcription_credentials(
        &settings.transcription_provider,
        BTreeMap::from([("apiKey".to_owned(), api_key)]),
    )
    .map_err(|error| error.to_frontend())?;
    bump_revision_for_target(&state, "transcription");
    external_settings(&state.library).map_err(|error| error.to_frontend())
}

#[tauri::command]
pub fn clear_external_asr_api_key(state: State<AppState>) -> Result<ExternalAiSettings, String> {
    clear_transcription_credentials().map_err(|error| error.to_frontend())?;
    external_settings(&state.library).map_err(|error| error.to_frontend())
}

/// 通用凭据保存：目标槽位由 target 决定，字段集合由 Provider Catalog 校验。
#[tauri::command]
pub fn save_external_credentials(
    state: State<AppState>,
    target: String,
    provider_id: String,
    credentials: BTreeMap<String, String>,
) -> Result<ExternalAiSettings, String> {
    let settings = external_settings(&state.library).map_err(|error| error.to_frontend())?;
    match target.as_str() {
        "analysis" => {
            if provider_id != settings.analysis_provider {
                return Err("请先保存分析服务商，再填写凭据".to_owned());
            }
            set_analysis_credentials(&provider_id, credentials)
                .map_err(|error| error.to_frontend())?;
        }
        "transcription" => {
            if provider_id != settings.transcription_provider {
                return Err("请先保存转写服务商，再填写凭据".to_owned());
            }
            set_transcription_credentials(&provider_id, credentials)
                .map_err(|error| error.to_frontend())?;
        }
        _ => return Err("凭据目标无效".to_owned()),
    }
    bump_revision_for_target(&state, &target);
    external_settings(&state.library).map_err(|error| error.to_frontend())
}

#[tauri::command]
pub fn clear_external_credentials(
    state: State<AppState>,
    target: String,
) -> Result<ExternalAiSettings, String> {
    match target.as_str() {
        "analysis" => clear_analysis_credentials(),
        "transcription" => clear_transcription_credentials(),
        _ => return Err("凭据目标无效".to_owned()),
    }
    .map_err(|error| error.to_frontend())?;
    bump_revision_for_target(&state, &target);
    external_settings(&state.library).map_err(|error| error.to_frontend())
}

/// 凭据槽位发生任何写入后递增修订号（旧健康快照随即失效）。
fn bump_revision_for_target(state: &AppState, target: &str) {
    let repository = state.library.repository();
    if let Err(error) = repository.bump_credential_revision(target) {
        eprintln!("凭据修订号更新失败（target={target}）：{error}");
    }
}

/// 在分析/转写两个 Keychain 槽位之间复制同一服务商的凭据。
/// 返回值只有配置状态，秘密不经过前端。
#[tauri::command]
pub fn reuse_external_credentials(
    state: State<AppState>,
    target: String,
) -> Result<ExternalAiSettings, String> {
    let settings = external_settings(&state.library).map_err(|error| error.to_frontend())?;
    let (source_provider, source_credentials) = match target.as_str() {
        "analysis" => (
            settings.transcription_provider.clone(),
            get_asr_credentials().map_err(|error| error.to_frontend())?,
        ),
        "transcription" => (
            settings.analysis_provider.clone(),
            get_analysis_credentials().map_err(|error| error.to_frontend())?,
        ),
        _ => return Err("凭据目标无效".to_owned()),
    };
    let target_provider = if target == "analysis" {
        &settings.analysis_provider
    } else {
        &settings.transcription_provider
    };
    if source_provider != target_provider.as_str() {
        return Err("另一项使用的服务商不同，无法复用凭据".to_owned());
    }
    let credentials = source_credentials
        .filter(|credentials| credentials_are_complete(&source_provider, Some(credentials)))
        .ok_or_else(|| "另一项还没有可复用的完整凭据".to_owned())?;
    if target == "analysis" {
        set_analysis_credentials(target_provider, credentials)
            .map_err(|error| error.to_frontend())?;
    } else {
        set_transcription_credentials(target_provider, credentials)
            .map_err(|error| error.to_frontend())?;
    }
    bump_revision_for_target(&state, &target);
    external_settings(&state.library).map_err(|error| error.to_frontend())
}

/// 只在用户明确点击“显示”时返回当前槽位的完整凭据。
/// 服务商必须与已保存设置一致，避免把上一家的 Key 显示到新表单里。
#[tauri::command]
pub fn reveal_external_credentials(
    state: State<AppState>,
    target: String,
    provider_id: String,
) -> Result<BTreeMap<String, String>, String> {
    let settings = state
        .library
        .repository()
        .external_ai_settings(false, false)
        .map_err(|error| error.to_frontend())?;
    let (saved_provider, credentials) = match target.as_str() {
        "analysis" => (
            settings.analysis_provider,
            get_analysis_credentials().map_err(|error| error.to_frontend())?,
        ),
        "transcription" => (
            settings.transcription_provider,
            get_asr_credentials().map_err(|error| error.to_frontend())?,
        ),
        _ => return Err("凭据目标无效".to_owned()),
    };
    if saved_provider != provider_id {
        return Err("当前显示的服务商与已保存凭据不一致，请先保存配置".to_owned());
    }
    credentials
        .filter(|credentials| credentials_are_complete(&provider_id, Some(credentials)))
        .ok_or_else(|| "没有找到完整凭据，请重新填写".to_owned())
}

#[tauri::command]
pub async fn test_external_ai_connection(state: State<'_, AppState>) -> Result<(), String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || test_external_ai_connection_blocking(&state))
        .await
        .map_err(|_| "外部 AI 连接测试后台任务异常，请重试".to_owned())?
}

#[tauri::command]
pub async fn test_external_asr_connection(state: State<'_, AppState>) -> Result<(), String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        crate::external_asr::probe_external_asr_connection(&state.library)
            .map_err(|error| error.to_frontend())
    })
    .await
    .map_err(|_| "第三方转写连通测试后台任务异常，请重试".to_owned())?
}

/// v1.4 包5：录音中打点——记录当前录音位置（不含暂停），返回已打点清单。
#[tauri::command]
pub fn mark_recording(state: State<AppState>) -> Result<Vec<i64>, String> {
    state
        .recording
        .mark()
        .map_err(|error| error.to_frontend())
}

/// v1.4 包5：读取某条记录的打点标记（相对录音开始的毫秒升序）。
#[tauri::command]
pub fn get_record_marks(state: State<AppState>, recordId: String) -> Result<Vec<i64>, String> {
    state
        .library
        .repository()
        .get_record_marks(&recordId)
        .map_err(|error| error.to_frontend())
}

/// v1.4 包8/9：生成记录产物（口播稿/show notes/社媒文案）的通用实现。
/// 通道选择与既有分析一致：已启用且同意的外部 AI 优先，否则本机 Ollama；
/// 未配置任何模型时如实报错而不是伪造。生成只读取逐字稿，不发送录音音频；
/// 产物落 record_artifacts（同一 kind 重新生成覆盖，不堆积历史）。
fn generate_record_artifact(
    state: &AppState,
    record_id: &str,
    kind: &str,
) -> Result<crate::types::RecordArtifact, String> {
    let (system_prompt, empty_error) = match kind {
        "script" => (
            SCRIPT_SYSTEM_PROMPT,
            "该记录还没有逐字稿，无法改写成稿",
        ),
        "show_notes" => (
            SHOW_NOTES_SYSTEM_PROMPT,
            "该记录还没有逐字稿，无法生成节目简介",
        ),
        "social_post" => (
            SOCIAL_POST_SYSTEM_PROMPT,
            "该记录还没有逐字稿，无法生成社媒文案",
        ),
        other => return Err(format!("不支持的产物类型：{other}")),
    };
    let repository = state.library.repository();
    let segments = repository
        .list_transcript_segments(record_id)
        .map_err(|error| error.to_frontend())?;
    if segments.is_empty() {
        return Err(empty_error.to_owned());
    }
    let transcript = segments
        .iter()
        .map(|segment| {
            let speaker = segment.speaker_label.as_deref().unwrap_or("");
            if !speaker.is_empty() && speaker != "未知" {
                format!("[{}][{}] {}", segment.sequence, speaker, effective_text(segment))
            } else {
                format!("[{}] {}", segment.sequence, effective_text(segment))
            }
        })
        .collect::<Vec<_>>()
        .join("\n");

    // 通道选择（与分析同口径：外部启用+同意 → 云端；否则本机 Ollama）。
    let ext_settings = repository
        .external_ai_settings(false, false)
        .map_err(|error| error.to_frontend())?;
    let (provider, model, content) = if ext_settings.enabled && ext_settings.privacy_consent_at.is_some()
    {
        let settings = require_external_ai_consent(&state.library).map_err(|e| e.to_frontend())?;
        let api_key = get_api_key()
            .map_err(|error| error.to_frontend())?
            .ok_or_else(|| "请先配置第三方文本理解 API Key".to_owned())?;
        let client = OpenAiCompatibleClient::new(&settings.base_url, &settings.model, &api_key)
            .map_err(|error| error.to_frontend())?;
        let content = client
            .complete_text(system_prompt, &transcript)
            .map_err(|error| error.to_frontend())?;
        (
            "external-openai-compatible".to_owned(),
            settings.model,
            content,
        )
    } else {
        let settings = repository
            .knowledge_settings()
            .map_err(|error| error.to_frontend())?;
        let adapter = crate::analysis::OllamaAdapter::detect(&settings.analysis_model)
            .map_err(|error| error.to_frontend())?;
        let content = adapter
            .chat(
                system_prompt,
                &[crate::analysis::ChatMessage {
                    role: "user".to_owned(),
                    content: transcript,
                }],
                0.4,
                4_096,
            )
            .map_err(|error| error.to_frontend())?;
        ("ollama".to_owned(), settings.analysis_model, content)
    };

    repository
        .save_record_artifact(record_id, kind, &content, &provider, &model)
        .map_err(|error| error.to_frontend())
}

/// v1.4 包8：生成口播稿改写——把逐字稿口语转书面，保留口播语气。
#[tauri::command]
pub async fn rewrite_record_script(
    state: State<'_, AppState>,
    record_id: String,
) -> Result<crate::types::RecordArtifact, String> {
    // 生成是几十秒级的 LLM 调用：必须 async + spawn_blocking，
    // 同步命令会阻塞 Tauri 主线程导致界面冻结（commands.rs 顶部约定）。
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        generate_record_artifact(&state, &record_id, "script")
    })
    .await
    .map_err(|_| "改写稿生成任务异常，请重试".to_owned())?
}

/// v1.4 包9：生成节目简介（show notes）。
#[tauri::command]
pub async fn generate_show_notes(
    state: State<'_, AppState>,
    record_id: String,
) -> Result<crate::types::RecordArtifact, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        generate_record_artifact(&state, &record_id, "show_notes")
    })
    .await
    .map_err(|_| "节目简介生成任务异常，请重试".to_owned())?
}

/// v1.4 包9：生成社媒文案。
#[tauri::command]
pub async fn generate_social_post(
    state: State<'_, AppState>,
    record_id: String,
) -> Result<crate::types::RecordArtifact, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        generate_record_artifact(&state, &record_id, "social_post")
    })
    .await
    .map_err(|_| "社媒文案生成任务异常，请重试".to_owned())?
}

/// v1.4 包8：列出某条记录的产物。
#[tauri::command]
pub fn list_record_artifacts(
    state: State<AppState>,
    record_id: String,
) -> Result<Vec<crate::types::RecordArtifact>, String> {
    state
        .library
        .repository()
        .list_record_artifacts(&record_id)
        .map_err(|error| error.to_frontend())
}

const SCRIPT_SYSTEM_PROMPT: &str = "你是一位资深的内容编辑。把下面的口语转写改写成一份顺口、有节奏感的书面稿：保留讲者的原意和个人语气，去掉口头禅、重复和自我纠正，长句拆短，段落之间空行。只输出改写后的稿子本身，不要解释、不要标题、不要加前言后语。";

const SHOW_NOTES_SYSTEM_PROMPT: &str = "你是播客/课程运营。根据下面的内容写一份节目简介（show notes）：3-5 句话概括本期亮点，再加 4-6 条带时间点的要点清单（用 [分:秒] 标注）。语言自然，适合放在播放器简介区。只输出简介本身，不要解释。";

const SOCIAL_POST_SYSTEM_PROMPT: &str = "你是社媒运营。根据下面的内容写一条适合发朋友圈/小红书/微博的短文：开头一句钩子，中间 2-3 个要点或金句，结尾一个互动提问。带 3-5 个相关话题标签。语气真实不像广告。只输出文案本身，不要解释。";

pub(crate) fn test_external_ai_connection_blocking(state: &AppState) -> Result<(), String> {
    let settings =
        require_external_ai_consent(&state.library).map_err(|error| error.to_frontend())?;
    if !settings.enabled {
        return Err("请先启用外部 AI".to_owned());
    }
    let api_key = get_api_key()
        .map_err(|error| error.to_frontend())?
        .ok_or_else(|| "请先配置外部 AI API Key".to_owned())?;
    OpenAiCompatibleClient::new(&settings.base_url, &settings.model, &api_key)
        .and_then(|client| client.test())
        .map_err(|error| error.to_frontend())
}

// ---------------------------------------------------------------------------
// Phase 0：草稿诊断 / 保存并验证 / 验证已保存配置。
// 诊断不落盘；「保存并验证」先核销一次性票据再提交，验证失败原配置不变。
// ---------------------------------------------------------------------------

/// 草稿诊断：用表单凭据（或已保存凭据回退）做一次真实小请求。
/// 新凭据只在本调用栈与短期内存票据状态中存在，不写库、不写钥匙串。
#[tauri::command]
pub async fn diagnose_external_provider(
    state: State<'_, AppState>,
    target: String,
    provider_id: String,
    base_url: String,
    model: String,
    credentials: BTreeMap<String, String>,
) -> Result<crate::diagnosis::DiagnosisReport, String> {
    let library = state.library.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let report = crate::diagnosis::diagnose_external_provider_blocking(
            Some(&library),
            crate::diagnosis::DiagnoseInput {
                target: target.clone(),
                provider_id: provider_id.clone(),
                base_url: base_url.clone(),
                model: model.clone(),
                credentials: credentials.clone(),
            },
        )?;
        // 通过才签发票据；票据绑定「解析后」的地址/模型/凭据摘要，
        // 提交端按同一解析语义重算比对。
        if report.status == "passed" {
            let capability_name = if target == "analysis" {
                "analysis"
            } else {
                "transcription"
            };
            let resolved = provider_catalog::resolve_config(
                capability_name,
                &provider_id,
                &base_url,
                &model,
            )
            .map_err(|error| error.to_frontend())?;
            let ticket_credentials = if credentials.is_empty() {
                crate::diagnosis::resolve_saved_credentials(
                    &library,
                    capability_name,
                    &provider_id,
                )?
            } else {
                credentials
            };
            let ticket = crate::state::issue_verification_ticket(
                &target,
                &resolved.provider_id,
                &resolved.base_url,
                &resolved.model,
                &ticket_credentials,
            );
            return Ok(crate::diagnosis::DiagnosisReport {
                verification_token: Some(ticket),
                ..report
            });
        }
        Ok(report)
    })
    .await
    .map_err(|_| "诊断后台任务异常，请重试".to_owned())?
}

/// 保存并验证的提交参数。
/// camelCase：Tauri 对「结构体参数」不做参数名转换（只对顶层命令参数转），
/// 前端传 providerId 时必须让 serde 按 camelCase 找字段，否则报
/// missing field `provider_id`。
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveAndVerifyArgs {
    pub target: String,
    pub provider_id: String,
    pub base_url: String,
    pub model: String,
    pub credentials: BTreeMap<String, String>,
    pub privacy_consent_at: Option<String>,
    pub audio_upload_consent_at: Option<String>,
    pub enabled: bool,
    pub verification_token: String,
}

/// 保存并验证：先核销一次性票据（草稿已通过诊断且指纹未变），通过才提交。
/// 验证失败或票据无效时，原保存配置完全不变。
#[tauri::command]
pub async fn save_and_verify_provider(
    state: State<'_, AppState>,
    args: SaveAndVerifyArgs,
) -> Result<crate::types::SaveAndVerifyOutcome, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || save_and_verify_provider_blocking(&state, args))
        .await
        .map_err(|_| "保存后台任务异常，请重试".to_owned())?
}

fn save_and_verify_provider_blocking(
    state: &AppState,
    args: SaveAndVerifyArgs,
) -> Result<crate::types::SaveAndVerifyOutcome, String> {
    let capability_name = match args.target.as_str() {
        "analysis" => "analysis",
        "transcription" => "transcription",
        _ => return Err("凭据目标无效".to_owned()),
    };
    let repository = state.library.repository();
    // 1. 解析实际凭据（与诊断端同一语义：空表回退已保存且服务商一致）。
    let credentials = if args.credentials.is_empty() {
        crate::diagnosis::resolve_saved_credentials(
            &state.library,
            capability_name,
            &args.provider_id,
        )?
    } else {
        if !credentials_are_complete(&args.provider_id, Some(&args.credentials)) {
            return Err("请完整填写该服务商要求的凭据".to_owned());
        }
        args.credentials
    };
    // 2. 规范化 provider/base_url/model（与诊断时一致）。
    let resolved = provider_catalog::resolve_config(
        capability_name,
        &args.provider_id,
        &args.base_url,
        &args.model,
    )
    .map_err(|error| error.to_frontend())?;
    // 3. 票据校验（在任何写之前——失败零副作用）。
    if !crate::state::consume_verification_ticket(
        &args.verification_token,
        &args.target,
        &resolved.provider_id,
        &resolved.base_url,
        &resolved.model,
        &credentials,
    ) {
        return Err("验证结果已失效（配置已变化或超过有效期），请重新验证后再保存".to_owned());
    }
    // 4. 读旧值（回滚用）。
    let old_settings = state
        .library
        .repository()
        .external_ai_settings(false, false)
        .map_err(|error| error.to_frontend())?;
    let (old_provider, old_credentials) = if capability_name == "analysis" {
        (
            old_settings.analysis_provider.clone(),
            get_analysis_credentials().map_err(|error| error.to_frontend())?,
        )
    } else {
        (
            old_settings.transcription_provider.clone(),
            get_asr_credentials().map_err(|error| error.to_frontend())?,
        )
    };
    let credentials_from_env = if capability_name == "analysis" {
        crate::external_ai_gate::analysis_credentials_from_env()
    } else {
        crate::external_ai_gate::transcription_credentials_from_env()
    };
    // 5. 写凭据（失败 → 尚无任何落盘，直接返回）。
    let set_result = if capability_name == "analysis" {
        set_analysis_credentials(&resolved.provider_id, credentials.clone())
    } else {
        set_transcription_credentials(&resolved.provider_id, credentials.clone())
    };
    if let Err(error) = set_result {
        return Err(error.to_frontend());
    }
    // 6. DB 单事务：settings + revision + health。
    let now = chrono::Utc::now().to_rfc3339();
    let next_settings = old_settings_with_draft(
        state,
        &args.target,
        &resolved,
        args.privacy_consent_at.clone(),
        args.audio_upload_consent_at.clone(),
        args.enabled,
    )?;
    // 保持另一个槽位现状（draft 只改本槽位字段）。
    // 快照的修订号必须等于事务内 bump 后的值（当前值 + 1），
    // 否则指纹永不相配、「已验证」不可达。
    let next_revision = repository
        .credential_revision(&args.target)
        .map_err(|error| error.to_frontend())?
        + 1;
    let health = crate::types::ProviderHealthSnapshot {
        target: args.target.clone(),
        provider_id: resolved.provider_id.clone(),
        base_url: resolved.base_url.clone(),
        model: resolved.model.clone(),
        credential_revision: next_revision,
        catalog_version: provider_catalog_version(),
        stage: "ok".to_owned(),
        status: "passed".to_owned(),
        error_code: None,
        verified_at: now,
    };
    let bump = Some(args.target.as_str());
    match repository.commit_external_provider(&next_settings, true, true, bump, Some(&health)) {
        Ok(settings) => {
            let saved_health = repository
                .provider_health(&args.target)
                .map_err(|error| error.to_frontend())?
                .ok_or_else(|| "健康快照写入失败".to_owned())?;
            Ok(crate::types::SaveAndVerifyOutcome {
                settings,
                saved: vec!["credentials".to_owned(), "settings".to_owned(), "health".to_owned()],
                rolled_back: Vec::new(),
                partial: false,
                health: saved_health,
            })
        }
        Err(db_error) => {
            // DB 失败：Keychain 已写新凭据——尽力回滚。
            let rollback = if credentials_from_env {
                // env 命中：只删 Keychain 条目，env 语义继续生效；
                // 不走 clear_*_credentials（那会把本进程 env Key 静默禁用）。
                crate::external_ai_gate::rollback_delete_keychain_entry(&args.target)
            } else {
                match (&old_provider, &old_credentials) {
                    (provider, Some(credentials)) if provider == &resolved.provider_id => {
                        if capability_name == "analysis" {
                            set_analysis_credentials(provider, credentials.clone())
                        } else {
                            set_transcription_credentials(provider, credentials.clone())
                        }
                    }
                    _ => if capability_name == "analysis" {
                        clear_analysis_credentials()
                    } else {
                        clear_transcription_credentials()
                    },
                }
            };
            let rolled_back = match rollback {
                Ok(()) => vec!["credentials".to_owned()],
                Err(_) => Vec::new(),
            };
            // DB 事务自身原子：失败时 settings/revision/health 全部未写，
            // 因此「新 provider + 旧 Key」的静默状态不可能出现；
            // Keychain 回滚也完成后返回明确错误。
            let mut error_message = db_error.to_frontend();
            if rolled_back.is_empty() {
                error_message.push_str("；凭据回滚未完成，请到设置页核对当前保存状态");
            }
            Err(error_message)
        }
    }
}

/// 以旧设置为基础，仅覆盖目标槽位的 provider/base_url/model 与同意/启用。
fn old_settings_with_draft(
    state: &AppState,
    target: &str,
    resolved: &provider_catalog::ResolvedProviderConfig,
    privacy_consent_at: Option<String>,
    audio_upload_consent_at: Option<String>,
    enabled: bool,
) -> Result<crate::types::ExternalAiSettings, String> {
    let mut settings = state
        .library
        .repository()
        .external_ai_settings(false, false)
        .map_err(|error| error.to_frontend())?;
    if target == "analysis" {
        settings.analysis_provider = resolved.provider_id.clone();
        settings.base_url = resolved.base_url.clone();
        settings.model = resolved.model.clone();
        if let Some(consent) = privacy_consent_at {
            settings.privacy_consent_at = Some(consent);
        }
        settings.enabled = enabled;
    } else {
        settings.transcription_provider = resolved.provider_id.clone();
        settings.transcription_base_url = resolved.base_url.clone();
        settings.transcription_model = resolved.model.clone();
        if let Some(consent) = audio_upload_consent_at {
            settings.audio_upload_consent_at = Some(consent);
        }
    }
    Ok(settings)
}

fn provider_catalog_version() -> u32 {
    crate::provider_catalog::provider_catalog().version
}

/// 验证当前已保存配置（不写凭据、不改设置）：结果写入健康快照
/// （含失败），前端按指纹比对决定显示「已验证」还是「验证失败」。
#[tauri::command]
pub async fn verify_saved_provider(
    state: State<'_, AppState>,
    target: String,
) -> Result<crate::diagnosis::DiagnosisReport, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || verify_saved_provider_blocking(&state, &target))
        .await
        .map_err(|_| "验证后台任务异常，请重试".to_owned())?
}

fn verify_saved_provider_blocking(
    state: &AppState,
    target: &str,
) -> Result<crate::diagnosis::DiagnosisReport, String> {
    let capability_name = match target {
        "analysis" => "analysis",
        "transcription" => "transcription",
        _ => return Err("凭据目标无效".to_owned()),
    };
    let repository = state.library.repository();
    let settings = repository
        .external_ai_settings(false, false)
        .map_err(|error| error.to_frontend())?;
    let (provider_id, base_url, model) = if capability_name == "analysis" {
        (
            settings.analysis_provider.clone(),
            settings.base_url.clone(),
            settings.model.clone(),
        )
    } else {
        (
            settings.transcription_provider.clone(),
            settings.transcription_base_url.clone(),
            settings.transcription_model.clone(),
        )
    };
    if provider_id.is_empty() || provider_id == "none" {
        return Err("尚未配置该槽位的服务商".to_owned());
    }
    // 凭据回退路径（空凭据表 → 已保存凭据，要求服务商一致）。
    let report = crate::diagnosis::diagnose_external_provider_blocking(
        Some(&state.library),
        crate::diagnosis::DiagnoseInput {
            target: target.to_owned(),
            provider_id: provider_id.clone(),
            base_url: base_url.clone(),
            model: model.clone(),
            credentials: BTreeMap::new(),
        },
    )?;
    // 结果（含失败）写入健康快照：重启后用户仍能看到最后一次验证结论。
    let revision = repository
        .credential_revision(target)
        .map_err(|error| error.to_frontend())?;
    repository
        .save_provider_health(&crate::types::ProviderHealthSnapshot {
            target: target.to_owned(),
            provider_id: provider_id.clone(),
            base_url: base_url.clone(),
            model: model.clone(),
            credential_revision: revision,
            catalog_version: provider_catalog_version(),
            stage: report.stage.as_str().to_owned(),
            status: report.status.clone(),
            error_code: report.error_code.clone(),
            verified_at: chrono::Utc::now().to_rfc3339(),
        })
        .map_err(|error| error.to_frontend())?;
    Ok(report)
}

/// 前端推导六态所需：健康快照（可能不存在，前端区分「无快照」）+ 修订号。
#[tauri::command]
pub fn get_provider_health_status(
    state: State<AppState>,
    target: String,
) -> Result<Option<crate::types::ProviderHealthSnapshot>, String> {
    state
        .library
        .repository()
        .provider_health(&target)
        .map_err(|error| error.to_frontend())
}

/// 凭据修订号：前端用于与快照比对判定「需重新验证」。
#[tauri::command]
pub fn get_credential_revision(state: State<AppState>, target: String) -> Result<i64, String> {
    state
        .library
        .repository()
        .credential_revision(&target)
        .map_err(|error| error.to_frontend())
}

/// 拉取服务商的 OpenAI 兼容模型列表（GET {base}/models）。
///
/// 供分析设置面板在「保存之前」使用：Key 只作为参数在内存中使用，不落盘、
/// 不入钥匙串；请求只携带该 Key 发往用户填写的服务商本身，不含任何用户内容，
/// 因此不走隐私同意门禁。地址仍必须通过 SSRF 安全校验。
#[tauri::command]
pub async fn list_remote_models(
    base_url: String,
    api_key: String,
) -> Result<Vec<String>, String> {
    tauri::async_runtime::spawn_blocking(move || list_remote_models_blocking(base_url, api_key))
        .await
        .map_err(|_| "拉取模型列表后台任务异常，请重试".to_owned())?
}

/// `list_remote_models` 的同步实现，测试直接调用（不经 async runtime）。
pub(crate) fn list_remote_models_blocking(
    base_url: String,
    api_key: String,
) -> Result<Vec<String>, String> {
    let base_url = base_url.trim().trim_end_matches('/').to_owned();
    if base_url.is_empty() || base_url.len() > 500 {
        return Err("接口地址无效".to_owned());
    }
    crate::memory::validate_external_base_url(&base_url).map_err(|error| error.to_string())?;
    if api_key.trim().is_empty() {
        return Err("请先填写 API Key 再拉取模型列表".to_owned());
    }
    // 请求时复校（与 chat/转写路径同规）：保存前的地址可能在发起请求时已被
    // DNS 重定向到内网，发请求前再拦一次。
    let endpoint = format!("{base_url}/models");
    crate::memory::assert_endpoint_still_safe(&endpoint).map_err(|error| error.to_string())?;
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(30))
        .redirects(0)
        .build();
    let response = agent
        .get(&endpoint)
        .set("Authorization", &format!("Bearer {}", api_key.trim()))
        .call()
        .map_err(|error| match error {
            ureq::Error::Status(code, response) => {
                let body = response.into_string().unwrap_or_default();
                let provider_message = truncated_provider_body(&body);
                match code {
                    401 | 403 => format!("Key 无效或无权限（HTTP {code}）：{provider_message}"),
                    _ => format!("服务商返回错误（HTTP {code}）：{provider_message}"),
                }
            }
            ureq::Error::Transport(transport) => {
                format!("无法连接服务商，请检查接口地址与网络：{transport}")
            }
        })?;
    let value: serde_json::Value = response
        .into_json()
        .map_err(|error| format!("模型列表响应无效：{error}"))?;
    parse_remote_models(&value)
}

/// 用当前表单或已保存凭据拉取服务商模型列表。
///
/// 内置服务商只允许目录中的固定地址；自定义接口仍走 SSRF 校验。转写目标只
/// 返回像 ASR 的模型，避免把聊天模型误选成转写模型。
#[tauri::command]
pub async fn list_external_models(
    state: State<'_, AppState>,
    target: String,
    provider_id: String,
    base_url: String,
    api_key: Option<String>,
) -> Result<RemoteModelList, String> {
    let library = state.library.clone();
    tauri::async_runtime::spawn_blocking(move || {
        list_external_models_blocking(&library, &target, &provider_id, &base_url, api_key)
    })
    .await
    .map_err(|_| "拉取模型列表后台任务异常，请重试".to_owned())?
}

pub(crate) fn list_external_models_blocking(
    library: &ManagedLibrary,
    target: &str,
    provider_id: &str,
    base_url: &str,
    api_key: Option<String>,
) -> Result<RemoteModelList, String> {
    let capability_name = match target {
        "analysis" => "analysis",
        "transcription" => "transcription",
        _ => return Err("模型目标无效".to_owned()),
    };
    let capability = provider_catalog::capability(provider_id, capability_name)
        .ok_or_else(|| "该服务商不支持所选能力".to_owned())?;
    if !capability.supports_model_list {
        return Err("该服务商不提供模型列表接口，请使用内置推荐模型或手动填写模型 ID".to_owned());
    }
    let resolved_base_url =
        provider_catalog::resolve_base_url(capability_name, provider_id, base_url)
            .map_err(|error| error.to_frontend())?;
    let key = match api_key.filter(|value| !value.trim().is_empty()) {
        Some(value) => value.trim().to_owned(),
        None => {
            let settings = library
                .repository()
                .external_ai_settings(false, false)
                .map_err(|error| error.to_frontend())?;
            let saved_provider = if capability_name == "analysis" {
                settings.analysis_provider
            } else {
                settings.transcription_provider
            };
            if saved_provider != provider_id {
                return Err("请先保存该服务商的 API Key，再拉取模型列表".to_owned());
            }
            let credentials = if capability_name == "analysis" {
                get_analysis_credentials()
            } else {
                get_asr_credentials()
            }
            .map_err(|error| error.to_frontend())?;
            credentials
                .and_then(|credentials| credentials.get("apiKey").cloned())
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| "请先保存该服务商的 API Key，再拉取模型列表".to_owned())?
        }
    };
    let mut models = list_remote_models_blocking(resolved_base_url, key)?;
    if capability_name == "transcription" {
        models.retain(|model| looks_like_asr_model(model));
        if models.is_empty() {
            return Err(
                "服务商返回了模型，但没有可识别的语音转写模型；请手动填写模型 ID".to_owned(),
            );
        }
        return Ok(RemoteModelList {
            chat_candidates: models,
            other_models: Vec::new(),
            all_unclassified: false,
        });
    }
    // 分析方向：黑名单排除法分组。全部被排除时降级为全量 + 标记，
    // 不让用户面对空列表。
    let chat_candidates: Vec<String> = models
        .iter()
        .filter(|model| !looks_like_non_chat_model(model))
        .cloned()
        .collect();
    if chat_candidates.is_empty() {
        return Ok(RemoteModelList {
            chat_candidates: models,
            other_models: Vec::new(),
            all_unclassified: true,
        });
    }
    let other_models: Vec<String> = models
        .into_iter()
        .filter(|model| looks_like_non_chat_model(model))
        .collect();
    Ok(RemoteModelList {
        chat_candidates,
        other_models,
        all_unclassified: false,
    })
}

fn looks_like_asr_model(model: &str) -> bool {
    let lower = model.to_ascii_lowercase();
    ["asr", "whisper", "transcrib", "paraformer", "sensevoice", "speech"]
        .iter()
        .any(|marker| lower.contains(marker))
}

/// 疑似非文本对话模型（embedding/rerank/ASR/TTS/视觉/审计等）。
/// 排除法：黑名单命中即进「其他模型」折叠组，不作分析推荐；
/// 名单外的模型（可能适用）保持可见。已知会话模型
/// （qwen-max/plus、glm-4-flash、deepseek-chat 等）均不命中。
fn looks_like_non_chat_model(model: &str) -> bool {
    let lower = model.to_ascii_lowercase();
    [
        "embedding", "bge", "rerank", "tts", "sambert", "cosyvoice", "speech", "asr", "whisper",
        "transcrib", "paraformer", "sensevoice", "omni", "audio", "vl", "vision", "image",
        "dall-e", "moderation", "guard", "clip", "babbage", "davinci", "4v",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}

/// 模型列表的分组结果：分析首选文本对话候选，其余折叠为「其他模型」；
/// 全部被排除时降级为全量展示（标记 all_unclassified，前端提示适用性未知）。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteModelList {
    pub chat_candidates: Vec<String>,
    pub other_models: Vec<String>,
    pub all_unclassified: bool,
}

/// 从 OpenAI 兼容的 /models 响应里取模型 id 列表：标准 `data[].id`，
/// 部分中转站直接返回顶层数组，两种都认；空值与重复剔除后排序。
fn parse_remote_models(value: &serde_json::Value) -> Result<Vec<String>, String> {
    let entries = match value.get("data") {
        Some(data) => data.as_array().cloned().unwrap_or_default(),
        None => value.as_array().cloned().unwrap_or_default(),
    };
    let mut models: Vec<String> = entries
        .iter()
        .filter_map(|entry| {
            entry
                .get("id")
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .map(str::to_owned)
        })
        .collect();
    models.sort();
    models.dedup();
    if models.is_empty() {
        return Err("该服务商没有返回任何模型，可能不支持自动拉取，请手动填写模型名".to_owned());
    }
    Ok(models)
}

/// 服务商错误体原样透传前做净化与截断：去掉控制字符，最多保留 300 字节，
/// 避免超长 HTML 错误页把界面错误条撑爆。
fn truncated_provider_body(body: &str) -> String {
    let cleaned: String = body
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.chars().count() <= 300 {
        trimmed.to_owned()
    } else {
        let mut end = 0;
        for (index, _) in trimmed.char_indices() {
            if index >= 300 {
                break;
            }
            end = index;
        }
        format!("{}…", &trimmed[..end])
    }
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
    app: AppHandle,
    state: State<'_, AppState>,
    request: MemoryGenerationRequest,
) -> Result<MemorySnapshot, String> {
    let state = state.inner().clone();
    state.clear_generation_cancel(&request.generation_id);
    tauri::async_runtime::spawn_blocking(move || {
        memory::generate_snapshot(&state, &request)
            .inspect(|_| {
                spawn_memory_index_rebuild(Some(app.clone()), state.library.root().to_path_buf());
            })
            .map_err(|error| error.to_frontend())
    })
    .await
    .map_err(|_| "外部 AI 生成后台任务异常，请重试".to_owned())?
}

#[tauri::command]
pub fn start_memory_generation(
    app: AppHandle,
    state: State<AppState>,
    request: MemoryGenerationRequest,
) -> Result<MemoryGenerationJob, String> {
    start_memory_generation_with_state(state.inner().clone(), app, request)
        .map_err(|error| error.to_frontend())
}

fn start_memory_generation_with_state(
    state: AppState,
    app: AppHandle,
    request: MemoryGenerationRequest,
) -> crate::error::AppResult<MemoryGenerationJob> {
    let job = state.start_generation_job(&request.generation_id)?;
    std::thread::spawn(move || {
        match memory::generate_snapshot(&state, &request) {
            Ok(snapshot) => {
                state.finish_generation_job(&request.generation_id, &snapshot);
                spawn_memory_index_rebuild(Some(app.clone()), state.library.root().to_path_buf());
            }
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

/// 最新 L3 个人总述（含快照元数据；未生成过返回 null profile）。
#[tauri::command]
pub fn get_memory_profile(state: State<AppState>) -> Result<ProfileSnapshot, String> {
    match state
        .library
        .repository()
        .latest_profile_snapshot()
        .map_err(|error| error.to_frontend())?
    {
        Some((snapshot, profile)) => Ok(ProfileSnapshot {
            id: snapshot.id,
            status: snapshot.status.as_str().to_owned(),
            is_stale: snapshot.is_stale,
            model: snapshot.model,
            error_message: snapshot.error_message,
            updated_at: snapshot.updated_at,
            profile: Some(profile),
        }),
        None => Ok(ProfileSnapshot {
            id: String::new(),
            status: "not_built".to_owned(),
            is_stale: false,
            model: String::new(),
            error_message: None,
            updated_at: String::new(),
            profile: None,
        }),
    }
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
    app: AppHandle,
    state: State<AppState>,
    snapshot_id: String,
    item_id: String,
    decision: String,
    note: String,
) -> Result<MemoryFeedback, String> {
    let feedback = state
        .library
        .repository()
        .update_memory_feedback(&snapshot_id, &item_id, &decision, &note)
        .map_err(|error| error.to_frontend())?;
    // 单条确认/驳回同样改变记忆块收录集合；与批量确认行为保持一致。
    spawn_memory_index_rebuild(Some(app), state.library.root().to_path_buf());
    Ok(feedback)
}

#[tauri::command]
pub fn update_memory_feedback_batch(
    app: AppHandle,
    state: State<AppState>,
    snapshot_id: String,
    entries: Vec<(String, String, String)>,
) -> Result<Vec<MemoryFeedback>, String> {
    let feedback = state
        .library
        .repository()
        .update_memory_feedback_batch(&snapshot_id, &entries)
        .map_err(|error| error.to_frontend())?;
    // 确认/驳回会改变记忆块索引的收录集合，提交后立即重建。
    spawn_memory_index_rebuild(Some(app), state.library.root().to_path_buf());
    Ok(feedback)
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

/// Ollama 安装检测：CLI 可执行 + 本机服务可达。
/// 面板用它区分「未安装」与「装了但没启动」，并据此轮询等待用户装好。
#[tauri::command]
pub fn get_ollama_install_status() -> Result<OllamaInstallStatus, String> {
    let installed = std::process::Command::new("ollama")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false);
    Ok(OllamaInstallStatus {
        installed,
        running: ollama_models().is_ok(),
    })
}

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
    let index_root = library_root.clone();
    std::thread::spawn(move || {
        let result = (|| -> Result<u32, String> {
            let library = ManagedLibrary::open(library_root).map_err(|error| error.to_string())?;
            let _permit = crate::state::heavy_job_limiter().acquire();
            crate::analysis::correct_transcript_with_library(&library, &record_id)
                .map_err(|error| error.to_string())
        })();
        // 校对改写了逐字稿，知识索引里还是旧文本，重建一次。
        if result.is_ok() {
            spawn_incremental_index(app.clone(), index_root, record_id.clone());
        }
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
            crate::dock::ask_external(library, mode, &history, &message, record_id.as_deref())
                .map_err(|error| error.to_frontend())?
        }
        _ => crate::dock::ask_local(library, mode, &history, &message, record_id.as_deref())
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

#[tauri::command]
pub fn app_info(state: State<AppState>) -> AppInfo {
    AppInfo {
        version: env!("CARGO_PKG_VERSION").to_owned(),
        library_path: state.library.root().to_string_lossy().to_string(),
    }
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiskSpaceStatus {
    pub available_bytes: u64,
    pub low_space_threshold_bytes: u64,
}

/// 资料库存储占用报表：分类体量 + 磁盘可用 + 可清理预览（不删任何东西）。
#[tauri::command]
pub fn get_storage_report(state: State<AppState>) -> Result<crate::storage::StorageReport, String> {
    crate::storage::storage_report(&state.library).map_err(|error| error.to_frontend())
}

/// 无损清理可再生的中间文件（raw/ 下的预处理产物），返回释放体量。
/// 原件与产出永不自动删除——音轨是本地资产，删掉就失去回听与重新分析能力。
#[tauri::command]
pub fn cleanup_storage(
    state: State<AppState>,
) -> Result<crate::storage::StorageCleanupResult, String> {
    crate::storage::run_cleanup(&state.library).map_err(|error| error.to_frontend())
}

/// 资料库所在卷的可用空间与提醒阈值：前端据此显示低空间横幅（只提醒，不阻塞使用）。
#[tauri::command]
pub fn get_disk_space_status(state: State<AppState>) -> Result<DiskSpaceStatus, String> {
    let available_bytes = crate::disk_space::available_bytes(state.library.root())
        .map_err(|error| error.to_frontend())?;
    Ok(DiskSpaceStatus {
        available_bytes,
        low_space_threshold_bytes: crate::disk_space::LOW_SPACE_WARNING_BYTES,
    })
}

/* ------------------------------ v0.5.0：说话人分离 ------------------------------ */

#[tauri::command]
pub fn get_transcription_engine_status(
    state: State<AppState>,
) -> Result<TranscriptionEngineStatus, String> {
    let repository = state.library.repository();
    let engine = repository
        .setting_value("transcription_engine")
        .map_err(|error| error.to_frontend())?
        .unwrap_or_else(|| "embedded".to_owned());
    let whisperx = crate::whisper::whisperx_path();
    Ok(TranscriptionEngineStatus {
        engine,
        whisperx_available: whisperx.is_some(),
        whisperx_path: whisperx.map(|path| path.to_string_lossy().to_string()),
        hf_token_set: crate::memory::get_hf_token()
            .map_err(|error| error.to_frontend())?
            .is_some(),
    })
}

#[tauri::command]
pub fn set_transcription_engine(state: State<AppState>, engine: String) -> Result<(), String> {
    if !matches!(engine.as_str(), "embedded" | "whisperx") {
        return Err("转写引擎无效".to_owned());
    }
    state
        .library
        .repository()
        .set_setting_value("transcription_engine", &engine)
        .map_err(|error| error.to_frontend())
}

#[tauri::command]
pub fn set_hf_token(token: String) -> Result<(), String> {
    crate::memory::set_hf_token(&token).map_err(|error| error.to_frontend())
}

#[tauri::command]
pub fn clear_hf_token() -> Result<(), String> {
    crate::memory::clear_hf_token().map_err(|error| error.to_frontend())
}

#[tauri::command]
pub fn get_record_speakers(
    state: State<AppState>,
    record_id: String,
) -> Result<Vec<SpeakerSummary>, String> {
    state
        .library
        .repository()
        .list_record_speakers(&record_id)
        .map_err(|error| error.to_frontend())
}

#[tauri::command]
pub fn rename_record_speaker(
    state: State<AppState>,
    record_id: String,
    from_label: String,
    to_label: String,
    add_hotword: bool,
) -> Result<u32, String> {
    let repository = state.library.repository();
    let changed = repository
        .rename_record_speaker(&record_id, &from_label, &to_label)
        .map_err(|error| error.to_frontend())?;
    if add_hotword {
        let _ = repository.add_hotword(&to_label, "说话人名称");
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::thread;
    use std::time::Duration;

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn scratch_library(name: &str) -> ManagedLibrary {
        let root = std::env::temp_dir()
            .join("echo_memory_commands_tests")
            .join(format!("{name}_{}", COUNTER.fetch_add(1, Ordering::SeqCst)));
        let _ = std::fs::remove_dir_all(&root);
        ManagedLibrary::open(root).unwrap()
    }

    fn serve_one_response(response_body: &str) -> (String, thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let response_body = response_body.to_owned();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            let mut buffer = [0_u8; 8_192];
            loop {
                let read = stream.read(&mut buffer).unwrap();
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..read]);
                let Some(header_end) = request.windows(4).position(|window| window == b"\r\n\r\n")
                else {
                    continue;
                };
                let headers = String::from_utf8_lossy(&request[..header_end]).to_ascii_lowercase();
                let content_length = headers
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length:"))
                    .and_then(|value| value.trim().parse::<usize>().ok())
                    .unwrap_or(0);
                if request.len() >= header_end + 4 + content_length {
                    break;
                }
            }
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response_body.len(),
                response_body
            );
            stream.write_all(response.as_bytes()).unwrap();
            stream.flush().unwrap();
            String::from_utf8_lossy(&request).into_owned()
        });
        (format!("http://{address}"), handle)
    }

    #[test]
    fn transcription_model_filter_keeps_asr_models_only() {
        assert!(looks_like_asr_model("whisper-1"));
        assert!(looks_like_asr_model("gpt-4o-transcribe"));
        assert!(looks_like_asr_model("qwen3-asr-flash"));
        assert!(looks_like_asr_model("speech-to-text"));
        assert!(!looks_like_asr_model("gpt-4.1-mini"));
        assert!(!looks_like_asr_model("deepseek-chat"));
    }

    #[test]
    fn providers_without_model_endpoint_fail_before_network_or_key_lookup() {
        let library = scratch_library("models-unsupported");
        let error = list_external_models_blocking(
            &library,
            "transcription",
            "tencent",
            "https://asr.cloud.tencent.com/asr/flash/v1",
            None,
        )
        .unwrap_err();
        assert!(error.contains("不提供模型列表接口"), "{error}");
        drop(library);
    }

    #[test]
    fn custom_analysis_model_list_does_not_require_a_preselected_model() {
        let library = scratch_library("models-custom-analysis");
        let (base_url, server) = serve_one_response(
            r#"{"data":[{"id":"deepseek-chat"},{"id":"deepseek-reasoner"}]}"#,
        );
        let models = list_external_models_blocking(
            &library,
            "analysis",
            provider_catalog::CUSTOM_PROVIDER_ID,
            &base_url,
            Some("form-key".to_owned()),
        )
        .unwrap();

        assert_eq!(
            models.chat_candidates,
            vec!["deepseek-chat".to_owned(), "deepseek-reasoner".to_owned()]
        );
        assert!(models.other_models.is_empty());
        assert!(!models.all_unclassified);
        let request = server.join().unwrap();
        assert!(request.starts_with("GET /models "), "{request}");
        assert!(
            request
                .to_ascii_lowercase()
                .contains("authorization: bearer form-key"),
            "{request}"
        );
        drop(library);
    }

    #[test]
    fn analysis_model_list_partitions_chat_from_non_chat_models() {
        let library = scratch_library("models-analysis-filter");
        let (base_url, server) = serve_one_response(
            r#"{"data":[{"id":"qwen-max"},{"id":"qwen-turbo"},{"id":"text-embedding-v3"},{"id":"BAAI/bge-reranker-v2-m3"},{"id":"qwen3-asr-flash"},{"id":"qwen2.5-omni"},{"id":"qwen-tts"},{"id":"glm-4v"},{"id":"qwen-vl-plus"},{"id":"deepseek-chat"}]}"#,
        );
        let models = list_external_models_blocking(
            &library,
            "analysis",
            provider_catalog::CUSTOM_PROVIDER_ID,
            &base_url,
            Some("form-key".to_owned()),
        )
        .unwrap();
        server.join().unwrap();

        // 已知文本对话模型全部留在候选，embedding/rerank/ASR/TTS/omni/视觉
        // （含 glm-4v、qwen-vl）进折叠组。
        assert_eq!(
            models.chat_candidates,
            vec!["deepseek-chat".to_owned(), "qwen-max".to_owned(), "qwen-turbo".to_owned()]
        );
        assert_eq!(
            models.other_models,
            vec![
                "BAAI/bge-reranker-v2-m3".to_owned(),
                "glm-4v".to_owned(),
                "qwen-tts".to_owned(),
                "qwen-vl-plus".to_owned(),
                "qwen2.5-omni".to_owned(),
                "qwen3-asr-flash".to_owned(),
                "text-embedding-v3".to_owned(),
            ]
        );
        assert!(!models.all_unclassified);
        drop(library);
    }

    #[test]
    fn analysis_model_list_degrades_to_full_list_when_everything_is_filtered() {
        let library = scratch_library("models-analysis-degraded");
        let (base_url, server) = serve_one_response(
            r#"{"data":[{"id":"text-embedding-v3"},{"id":"qwen-tts"}]}"#,
        );
        let models = list_external_models_blocking(
            &library,
            "analysis",
            provider_catalog::CUSTOM_PROVIDER_ID,
            &base_url,
            Some("form-key".to_owned()),
        )
        .unwrap();
        server.join().unwrap();

        // 全部被排除：降级为全量 + 标记（前端提示适用性未知），不是空列表。
        assert_eq!(
            models.chat_candidates,
            vec!["qwen-tts".to_owned(), "text-embedding-v3".to_owned()]
        );
        assert!(models.other_models.is_empty());
        assert!(models.all_unclassified);
        drop(library);
    }

    fn prepare_record_with_transcript(
        library: &ManagedLibrary,
        name: &str,
    ) -> (crate::types::RecordBrief, crate::types::TranscriptVersion) {
        let repository = library.repository();
        let (record, _) = repository
            .create_record_with_transcription_job(
                name,
                &format!("外部分析-{name}"),
                None,
                Path::new("audio/external.wav"),
                &format!("hash-{name}"),
                1_000,
            )
            .unwrap();
        let (version, _) = repository
            .save_transcript(
                &record.id,
                "test",
                "test-model",
                &[crate::types::TranscriptSegmentInput {
                    start_ms: 0,
                    end_ms: 1_000,
                    speaker_label: Some("未知".to_owned()),
                    original_text: "我们确认使用第三方模型完成结构化整理。".to_owned(),
                }],
            )
            .unwrap();
        (record, version)
    }

    fn save_analysis(
        library: &ManagedLibrary,
        record_id: &str,
        version_id: &str,
        draft: crate::analysis::AnalysisDraft,
    ) -> crate::types::StoredAnalysis {
        library
            .repository()
            .save_analysis(record_id, version_id, "test-model", &draft)
            .unwrap()
    }

    fn valid_analysis_draft(
        segment_id: &str,
        quote: &str,
        quality_warning: Option<String>,
        quality_severity: Option<crate::analysis::AnalysisQualitySeverity>,
    ) -> crate::analysis::AnalysisDraft {
        crate::analysis::AnalysisDraft {
            summary: "这是一段足够完整的摘要，能够清楚说明讨论背景、主要结论和后续方向。"
                .to_owned(),
            key_points: vec![crate::analysis::AnalysisItemDraft {
                owner: None,
                text: "本次讨论确认了第三方模型的结构化整理方案".to_owned(),
                citation_segment_ids: vec![segment_id.to_owned()],
                quote_text: quote.to_owned(),
                start_ms: Some(0),
                end_ms: Some(1_000),
            }],
            decisions: Vec::new(),
            action_items: Vec::new(),
            open_questions: Vec::new(),
            custom_sections: Vec::new(),
            chapters: Vec::new(),
            highlights: Vec::new(),
            quality_warning,
            quality_severity,
        }
    }

    #[test]
    fn revalidation_recomputes_legacy_quality_fields_and_keeps_advisory_completed() {
        let library = scratch_library("revalidate-advisory");
        let (record, version) = prepare_record_with_transcript(&library, "advisory");
        let segments = library
            .repository()
            .list_transcript_segments(&record.id)
            .unwrap();
        let quote = segments[0].original_text.clone();
        let stored = save_analysis(
            &library,
            &record.id,
            &version.id,
            valid_analysis_draft(
                &segments[0].id,
                &quote,
                Some("旧版本阻断提醒".to_owned()),
                Some(crate::analysis::AnalysisQualitySeverity::Blocking),
            ),
        );

        let revalidated = revalidate_analysis_with_library(&library, &stored.id).unwrap();
        let draft: crate::analysis::AnalysisDraft =
            serde_json::from_str(&revalidated.content_json).unwrap();

        assert_eq!(revalidated.status, "completed");
        assert_eq!(
            draft.quality_severity,
            Some(crate::analysis::AnalysisQualitySeverity::Advisory)
        );
        assert!(!draft
            .quality_warning
            .unwrap_or_default()
            .contains("旧版本阻断提醒"));
    }

    #[test]
    fn revalidation_rejects_analysis_when_evidence_cannot_be_verified() {
        let library = scratch_library("revalidate-blocking");
        let (record, version) = prepare_record_with_transcript(&library, "blocking");
        let segments = library
            .repository()
            .list_transcript_segments(&record.id)
            .unwrap();
        let mut draft =
            valid_analysis_draft(&segments[0].id, "逐字稿中不存在的引用原文", None, None);
        draft.key_points[0].quote_text = "逐字稿中不存在的引用原文".to_owned();
        let stored = save_analysis(&library, &record.id, &version.id, draft);

        let error = revalidate_analysis_with_library(&library, &stored.id).unwrap_err();

        assert!(error.to_string().contains("仍有分析条目无法匹配可靠出处"));
        assert_eq!(
            library
                .repository()
                .get_analysis(&stored.id)
                .unwrap()
                .status,
            "completed"
        );
    }

    #[test]
    fn analysis_without_consent_stays_local_and_sends_nothing() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let library = scratch_library("external-analysis-consent");
        let (record, _) = prepare_record_with_transcript(&library, "consent");
        let repository = library.repository();
        // 理解整理已启用，但隐私同意未给：判定不走云端，也不得有任何外发。
        // 分析模型指向必然不存在的名字，让本机通道确定性地失败。
        repository
            .set_setting_value("analysis_model", "echo-test-absent-model")
            .unwrap();
        repository
            .set_setting_value("external_ai_enabled", "true")
            .unwrap();
        repository
            .set_setting_value("external_ai_base_url", &base_url)
            .unwrap();
        repository
            .set_setting_value("external_ai_model", "test-model")
            .unwrap();
        repository
            .set_setting_value("external_ai_privacy_consent_at", "")
            .unwrap();

        let _error = analyze_with_library(&library, &record.id).unwrap_err();
        assert!(repository.latest_analysis(&record.id).unwrap().is_none());

        listener.set_nonblocking(true).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_millis(200);
        let mut accepted = false;
        while std::time::Instant::now() < deadline {
            match listener.accept() {
                Ok(_) => {
                    accepted = true;
                    break;
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
                {
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("本地测试服务异常：{error}"),
            }
        }
        assert!(!accepted, "未同意时不得建立第三方分析连接");
        let _ = std::fs::remove_dir_all(library.root());
    }

    #[test]
    fn asr_channel_readiness_requires_full_configuration() {
        // 只覆盖不读取 Key 的维度：Key 存在性受进程级环境变量影响，
        // 并发测试下不可假设；该维度由 external_mode_with_ready_asr_channel_routes_to_third_party
        // （在 AsrApiKeyEnvGuard 锁内）覆盖。
        let library = scratch_library("asr-channel-ready");
        let repository = library.repository();
        let ready = || crate::external_ai_gate::external_asr_channel_ready(&library).unwrap();
        // 默认本机模式：判定必须为否，与转录分流一致。
        assert!(!ready());
        repository
            .set_setting_value("processing_mode", "external")
            .unwrap();
        // 缺同意：不配齐。
        assert!(!ready());
        repository
            .set_setting_value("cloud_audio_upload_consent_at", "2026-09-26T00:00:00Z")
            .unwrap();
        // 地址清空：不配齐。
        repository
            .set_setting_value("cloud_transcription_base_url", "")
            .unwrap();
        assert!(!ready());
        // 提供方置 none：不配齐。
        repository
            .set_setting_value("cloud_transcription_base_url", "https://api.example.com/v1")
            .unwrap();
        repository
            .set_setting_value("cloud_transcription_provider", "none")
            .unwrap();
        assert!(!ready());
        let _ = std::fs::remove_dir_all(library.root());
    }

    #[test]
    fn parse_remote_models_reads_data_and_top_level_arrays() {
        let standard: serde_json::Value = serde_json::json!({
            "data": [
                {"id": "glm-4-flash"},
                {"id": "glm-4-plus"},
                {"id": "" },
                {"id": "glm-4-flash"}
            ]
        });
        let mut models = parse_remote_models(&standard).unwrap();
        models.sort();
        assert_eq!(models, vec!["glm-4-flash", "glm-4-plus"]);

        let top_level: serde_json::Value =
            serde_json::json!([{"id": "deepseek-chat"}, {"id": "deepseek-reasoner"}]);
        let models = parse_remote_models(&top_level).unwrap();
        assert_eq!(models, vec!["deepseek-chat", "deepseek-reasoner"]);

        let empty: serde_json::Value = serde_json::json!({"data": []});
        let error = parse_remote_models(&empty).unwrap_err();
        assert!(error.contains("手动填写"), "{error}");
    }

    #[test]
    fn list_remote_models_maps_provider_errors_to_chinese() {
        // 200：标准 OpenAI 兼容响应 → 模型列表；复用现有 mock 服务基建。
        let body = r#"{"data":[{"id":"mock-model-b"},{"id":"mock-model-a"}]}"#;
        let (base_url, server) = serve_one_response(body);
        let models = list_remote_models_blocking(
            base_url.clone(),
            "mock-list-key".to_owned(),
        )
        .unwrap();
        let request = server.join().unwrap();
        assert!(request.contains("GET /models "), "{request}");
        assert!(
            request.to_ascii_lowercase().contains("authorization: bearer mock-list-key"),
            "{request}"
        );
        assert_eq!(models, vec!["mock-model-a", "mock-model-b"]);

        // 401：Key 无效 → 中文错误 + 服务商原话保留。
        // mock 服务必须先读走请求、写完响应后再读到 EOF 才关闭连接：
        // macOS 上带未读数据关 socket 会发 RST，把 client 读响应体的过程打断。
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut buffer = [0_u8; 4_096];
            let _ = stream.read(&mut buffer);
            let body = r#"{"error":{"message":"Invalid API key"}}"#;
            use std::io::Write as _;
            write!(
                stream,
                "HTTP/1.1 401 Unauthorized\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .unwrap();
            stream.flush().unwrap();
            let _ = stream.read(&mut buffer);
        });
        let error = list_remote_models_blocking(
            format!("http://{addr}"),
            "wrong-key".to_owned(),
        )
        .unwrap_err();
        server.join().unwrap();
        assert!(error.contains("Key 无效或无权限"), "{error}");
        assert!(error.contains("Invalid API key"), "{error}");

        // 空 Key：在发请求前就被挡下。
        let error = list_remote_models_blocking(
            "https://api.example.com/v1".to_owned(),
            "   ".to_owned(),
        )
        .unwrap_err();
        assert!(error.contains("先填写 API Key"), "{error}");
    }

    #[test]
    fn asr_half_configured_only_when_provider_chosen() {
        // Key 存在性读进程级环境变量，并发测试下不可假设——放进 env guard
        // 串行域内，让 Key 恒定存在，用同意的有无驱动「半配置」判定。
        use crate::external_ai_gate::AsrApiKeyEnvGuard;
        let _key = AsrApiKeyEnvGuard::set("test-asr-token");
        let library = scratch_library("asr-half-configured");
        let repository = library.repository();
        let half = || crate::external_ai_gate::external_asr_channel_half_configured(&library).unwrap();
        // ① 从未配置转写通道（提供方未选/none，模式也默认本机）：静默回退内置 Whisper 是对的。
        assert!(!half());
        // ② 提供方已选、模式切到第三方，但未确认音频上传说明：应判定为「半配置」，
        //    回退内置时要给一次明确提示，而不是无声无息。
        repository
            .set_setting_value("processing_mode", "external")
            .unwrap();
        repository
            .set_setting_value("cloud_transcription_provider", "openai-compatible")
            .unwrap();
        assert!(half());
        // ③ 同意补上、地址模型用非空默认值、Key 在 env 里：通道配齐，不再是半配置。
        repository
            .set_setting_value("cloud_audio_upload_consent_at", "2026-10-25T00:00:00Z")
            .unwrap();
        assert!(!half());
        let _ = std::fs::remove_dir_all(library.root());
    }

    #[test]
    fn external_mode_without_ready_asr_channel_falls_back_to_local_whisper() {
        let library = scratch_library("external-transcription-fallback");
        let (record, job) = library
            .repository()
            .create_record_with_transcription_job(
                "external-transcription",
                "第三方模式未配齐回退",
                None,
                Path::new("audio/external.wav"),
                "external-transcription-hash",
                1_000,
            )
            .unwrap();
        library
            .repository()
            .set_setting_value("processing_mode", "external")
            .unwrap();

        // 通道未配齐（无同意、无 Key）：必须回退内置 Whisper，而不是被 ASR 门禁拦死。
        // 用一个必然不存在的模型路径，让「模型文件不存在」成为走到内置路径的确定性证据。
        let error = run_transcription_with_engine(
            None,
            &library,
            &record.id,
            &job,
            "zh",
            Some("/nonexistent/echo-whisper.bin"),
            None,
        )
        .unwrap_err();

        assert!(
            error.to_string().contains("选择的 Whisper 模型文件不存在"),
            "转写通道未配齐时必须回退内置 Whisper：{error}"
        );
        let _ = std::fs::remove_dir_all(library.root());
    }

    #[test]
    fn external_mode_with_ready_asr_channel_routes_to_third_party() {
        use crate::external_ai_gate::AsrApiKeyEnvGuard;

        let _key = AsrApiKeyEnvGuard::set("test-asr-token");
        let library = scratch_library("external-transcription-ready");
        let repository = library.repository();
        let (record, job) = repository
            .create_record_with_transcription_job(
                "external-transcription",
                "第三方通道配齐分流",
                None,
                Path::new("audio/external.wav"),
                "external-transcription-ready-hash",
                1_000,
            )
            .unwrap();
        let mut settings = repository.external_ai_settings(false, true).unwrap();
        settings.transcription_mode = "external".to_owned();
        settings.audio_upload_consent_at = Some("2026-09-26T00:00:00Z".to_owned());
        repository
            .update_external_ai_settings(&settings, false, true)
            .unwrap();

        // 同意、地址、模型、Key 都齐：必须进入第三方转写链路（在音频缺失处失败），
        // 而不是被同意门禁拦下或回退内置 Whisper。
        let error =
            run_transcription_with_engine(None, &library, &record.id, &job, "zh", None, None)
                .unwrap_err();

        let message = error.to_string();
        assert!(
            !message.contains("音频上传说明"),
            "通道配齐时不得被同意门禁拦下：{message}"
        );
        assert!(
            !message.contains("尚未安装转写模型") && !message.contains("Whisper 模型文件不存在"),
            "通道配齐时不得回退内置 Whisper：{message}"
        );
        assert!(
            !message.contains("请先配置第三方转写 API Key"),
            "Key 已在环境变量中：{message}"
        );
        assert_eq!(
            repository.get_job(&job.id).unwrap().status,
            "failed"
        );
        let _ = std::fs::remove_dir_all(library.root());
    }

    #[test]
    fn external_import_with_ready_asr_is_not_blocked_by_analysis_channel() {
        use crate::external_ai_gate::AsrApiKeyEnvGuard;

        let _key = AsrApiKeyEnvGuard::set("test-asr-token");
        let library = scratch_library("external-import-asr-ready");
        let repository = library.repository();
        let (record, job) = repository
            .create_record_with_transcription_job(
                "external-import-asr-ready",
                "第三方导入按通道分流",
                None,
                Path::new("audio/external.wav"),
                "external-import-asr-ready-hash",
                1_000,
            )
            .unwrap();
        // ASR 通道配齐（同意 + Key + 默认地址模型），理解整理未配置：
        // 导入必须继续按通道走第三方转写，不得被「文本发送说明」类错误拦死。
        let mut settings = repository.external_ai_settings(false, true).unwrap();
        settings.transcription_mode = "external".to_owned();
        settings.audio_upload_consent_at = Some("2026-09-26T00:00:00Z".to_owned());
        repository
            .update_external_ai_settings(&settings, false, true)
            .unwrap();
        let knowledge = repository.knowledge_settings().unwrap();

        let error = process_record_with_transcription_job(
            None,
            &library,
            &record.id,
            &job,
            &knowledge,
        )
        .unwrap_err();

        let message = error.to_string();
        assert!(
            !message.contains("文本发送说明"),
            "理解未配置不得拦死导入：{message}"
        );
        assert!(
            !message.contains("请先配置第三方转写 API Key"),
            "Key 已在环境变量中：{message}"
        );
        assert!(
            !message.contains("请先启用第三方文本理解服务"),
            "理解未配置不得拦死导入：{message}"
        );
        let jobs = repository.list_jobs_for_record(&record.id).unwrap();
        assert!(
            jobs.iter()
                .any(|job| job.job_type == "transcribe" && job.status == "failed"),
            "转写应按通道推进并在音频缺失处失败"
        );
        assert!(
            !jobs.iter().any(|job| job.job_type == "analyze"),
            "转写失败后不得进入分析"
        );
        let _ = std::fs::remove_dir_all(library.root());
    }

    #[test]
    fn external_analysis_persists_provider_and_model() {
        let transcript = "我们确认使用第三方模型完成结构化整理。";
        let analysis = serde_json::json!({
            "summary": "本段录音确认使用第三方模型完成结构化整理，并明确了后续分析方向。",
            "key_points": [
                {"text":"使用第三方大模型服务", "citation_segment_ids":["segment"], "quote_text":transcript},
                {"text":"完成逐字稿结构化整理", "citation_segment_ids":["segment"], "quote_text":transcript},
                {"text":"明确后续分析处理方向", "citation_segment_ids":["segment"], "quote_text":transcript}
            ],
            "decisions": [],
            "action_items": [],
            "open_questions": [],
            "custom_sections": []
        });
        let content = serde_json::to_string(&analysis).unwrap();
        let response = serde_json::json!({
            "choices": [{"message": {"content": content}}]
        })
        .to_string();
        let (base_url, server) = serve_one_response(&response);
        let _key = crate::external_ai_gate::ApiKeyEnvGuard::set("text-secret");
        let library = scratch_library("external-analysis-success");
        let (record, _) = prepare_record_with_transcript(&library, "success");
        let repository = library.repository();
        let mut settings = repository.external_ai_settings(false, false).unwrap();
        settings.analysis_provider = crate::provider_catalog::CUSTOM_PROVIDER_ID.to_owned();
        settings.transcription_mode = "external".to_owned();
        settings.enabled = false;
        settings.base_url = base_url;
        settings.model = "test-model".to_owned();
        settings.privacy_consent_at = None;
        repository
            .update_external_ai_settings(&settings, false, false)
            .unwrap();

        let mut settings = repository.external_ai_settings(false, false).unwrap();
        settings.enabled = true;
        settings.privacy_consent_at = Some("2026-09-25T00:00:00Z".to_owned());
        repository
            .update_external_ai_settings(&settings, false, false)
            .unwrap();

        let analysis_result = analyze_with_library(&library, &record.id);
        let received = server.join().unwrap();
        analysis_result.unwrap_or_else(|error| {
            panic!("第三方分析失败：{error}\n收到请求：{received}");
        });

        let stored = repository.latest_analysis(&record.id).unwrap().unwrap();
        assert_eq!(stored.provider, "external-openai-compatible");
        assert_eq!(stored.model, "test-model");
        let received_lower = received.to_ascii_lowercase();
        assert!(received_lower.starts_with("post /chat/completions http/1.1"));
        assert!(received_lower.contains("authorization: bearer text-secret"));
        assert!(received.contains("test-model"));
        let _ = std::fs::remove_dir_all(library.root());
    }

    /// 下载完成必须顺手登记为当前模型。少了这一步，用户在引导里下完 574MB 之后
    /// 第一次转写仍然会报「尚未安装转写模型」——引导里的下载按钮等于白点。
    #[test]
    fn downloading_a_model_registers_it_as_current() {
        let library = scratch_library("register");
        let models_dir = library.root().join("models");
        std::fs::create_dir_all(&models_dir).unwrap();
        let model = models_dir.join(crate::whisper::recommended_model_file());
        std::fs::write(&model, b"lmgg").unwrap();

        assert_eq!(
            library
                .repository()
                .knowledge_settings()
                .unwrap()
                .whisper_model_path,
            ""
        );

        register_downloaded_whisper_model(&library, &models_dir).unwrap();

        let settings = library.repository().knowledge_settings().unwrap();
        assert_eq!(settings.whisper_model_path, model.to_string_lossy());
        // 登记完就得真的能被用上，用户回设置页看到的必须是同一个模型。
        assert!(WhisperAdapter::detect_with_model_path(Some(&settings.whisper_model_path)).is_ok());
        let _ = std::fs::remove_dir_all(library.root());
    }

    /// 模型文件不在时不许谎报成功，否则用户以为装好了却用不了。
    #[test]
    fn registering_a_missing_model_fails_loudly() {
        let library = scratch_library("missing");
        let models_dir = library.root().join("models");
        std::fs::create_dir_all(&models_dir).unwrap();
        assert!(register_downloaded_whisper_model(&library, &models_dir).is_err());
        let _ = std::fs::remove_dir_all(library.root());
    }

    #[test]
    fn pending_download_reports_the_resume_point() {
        let library = scratch_library("pending");
        let models_dir = library.root().join("models");
        std::fs::create_dir_all(&models_dir).unwrap();
        assert!(pending_whisper_download(&models_dir).is_none());

        let partial = models_dir.join(crate::whisper::recommended_model_partial());
        std::fs::write(&partial, vec![0_u8; 2048]).unwrap();
        let pending = pending_whisper_download(&models_dir).expect("半成品应被识别");
        assert_eq!(pending.model, RECOMMENDED_MODEL_ID);
        assert_eq!(pending.bytes, 2048);
        // 半成品不能出现在可选模型列表里，否则用户会以为已经下好了。
        assert!(discover_whisper_models(&models_dir, "").is_empty());
        let _ = std::fs::remove_dir_all(library.root());
    }

    #[test]
    fn model_list_covers_every_file_in_the_models_directory() {
        let library = scratch_library("discover");
        let models_dir = library.root().join("models");
        std::fs::create_dir_all(&models_dir).unwrap();
        std::fs::write(models_dir.join("ggml-small.bin"), vec![0_u8; 64]).unwrap();
        std::fs::write(
            models_dir.join(crate::whisper::recommended_model_file()),
            vec![0_u8; 128],
        )
        .unwrap();
        std::fs::write(models_dir.join("notes.md"), b"x").unwrap();

        let models = discover_whisper_models(&models_dir, "");
        let ids: Vec<&str> = models.iter().map(|model| model.id.as_str()).collect();
        assert_eq!(ids, vec!["ggml-large-v3-turbo-q5_0", "ggml-small"]);
        let _ = std::fs::remove_dir_all(library.root());
    }

    #[test]
    fn model_source_label_tells_the_truth() {
        let models_dir = PathBuf::from("/tmp/echo-memory-models");
        let downloaded = models_dir.join(crate::whisper::recommended_model_file());
        assert_eq!(
            whisper_model_source_label(&downloaded, "", &models_dir),
            "应用内下载"
        );
        assert_eq!(
            whisper_model_source_label(&downloaded, &downloaded.to_string_lossy(), &models_dir),
            "手动选择"
        );
        assert_eq!(
            whisper_model_source_label(&PathBuf::from("/opt/models/mine.bin"), "", &models_dir),
            "环境变量指定"
        );
    }

    #[test]
    fn cancellation_requests_are_per_model() {
        request_cancellation("qwen2.5:7b");
        assert!(is_cancelled("qwen2.5:7b"));
        assert!(!is_cancelled("qwen3-embedding:0.6b"));
        clear_cancellation("qwen2.5:7b");
        assert!(!is_cancelled("qwen2.5:7b"));
        assert!(cancel_model_download("   ".to_owned()).is_err());
    }

    #[test]
    fn failed_transcription_does_not_start_analysis() {
        let library = scratch_library("pipeline-failure");
        let (record, _) = library
            .repository()
            .create_record_with_transcription_job(
                "pipeline-failure-record",
                "missing-model",
                None,
                Path::new("audio/missing.wav"),
                "pipeline-failure-hash",
                1_000,
            )
            .unwrap();
        library
            .repository()
            .set_setting_value(
                "whisper_model_path",
                &library
                    .root()
                    .join("models")
                    .join("missing-model.bin")
                    .to_string_lossy(),
            )
            .unwrap();

        assert!(process_record_with_library(&library, &record.id).is_err());

        let jobs = library
            .repository()
            .list_jobs_for_record(&record.id)
            .unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].job_type, "transcribe");
        assert_eq!(jobs[0].status, "failed");
        assert!(
            !jobs.iter().any(|job| job.job_type == "analyze"),
            "转写失败后不得继续创建分析任务"
        );
        let _ = std::fs::remove_dir_all(library.root());
    }

    #[test]
    fn retry_classification_separates_transient_from_permanent() {
        // 瞬时：值得自动重试。
        assert!(transcription_failure_is_retryable(&AppError::Io(
            std::io::Error::other("disk hiccup")
        )));
        assert!(transcription_failure_is_retryable(&AppError::Analysis(
            "Ollama 请求失败。请确认本机服务仍在运行。".to_owned()
        )));
        assert!(transcription_failure_is_retryable(&AppError::ExternalAi(
            "第三方转写请求超时".to_owned()
        )));
        assert!(transcription_failure_is_retryable(&AppError::Import(
            "增强与系统兼容预处理均失败，原始音频仍已保留".to_owned()
        )));
        // 永久：重试只会白等。
        assert!(!transcription_failure_is_retryable(&AppError::Analysis(
            "未安装模型“qwen2.5:7b”。请先在本机执行 ollama pull qwen2.5:7b。".to_owned()
        )));
        assert!(!transcription_failure_is_retryable(&AppError::Import(
            "仅支持 MP3、M4A、WAV、AAC 文件".to_owned()
        )));
        assert!(!transcription_failure_is_retryable(&AppError::Storage(
            "磁盘可用空间不足：本次操作约需 1.0 GB，当前可用 200.0 MB。".to_owned()
        )));
        assert!(!transcription_failure_is_retryable(&AppError::ExternalAi(
            "请先配置第三方转写 API Key".to_owned()
        )));
        assert!(!transcription_failure_is_retryable(&AppError::Analysis(
            "转写结果疑似退化，已保留旧转写".to_owned()
        )));
        assert!(!transcription_failure_is_retryable(&AppError::Invalid(
            "没有可重试的转写任务".to_owned()
        )));
    }

    fn retry_test_setup(library: &ManagedLibrary, name: &str) -> (String, ProcessingJob) {
        let repository = library.repository();
        let (record, job) = repository
            .create_record_with_transcription_job(
                name,
                &format!("自动重试-{name}"),
                None,
                Path::new("audio/retry.wav"),
                &format!("hash-{name}"),
                1_000,
            )
            .unwrap();
        repository
            .update_job_status(&job.id, "preparing", None)
            .unwrap();
        let job = repository.get_job(&job.id).unwrap();
        (record.id, job)
    }

    #[test]
    fn transient_failure_is_retried_once_then_succeeds() {
        let library = scratch_library("retry-transient");
        let (record_id, job) = retry_test_setup(&library, "transient");
        let calls = std::cell::Cell::new(0);
        let delays = [Duration::from_millis(1), Duration::from_millis(1)];
        run_pipeline_with_finite_retry(None, &library, &record_id, job, &delays, |_job| {
            calls.set(calls.get() + 1);
            if calls.get() == 1 {
                Err(AppError::Io(std::io::Error::other("transient")))
            } else {
                Ok(())
            }
        })
        .unwrap();
        assert_eq!(calls.get(), 2);
        let _ = std::fs::remove_dir_all(library.root());
    }

    #[test]
    fn permanent_failure_is_not_retried() {
        let library = scratch_library("retry-permanent");
        let (record_id, job) = retry_test_setup(&library, "permanent");
        let calls = std::cell::Cell::new(0);
        let delays = [Duration::from_millis(1), Duration::from_millis(1)];
        let error =
            run_pipeline_with_finite_retry(None, &library, &record_id, job, &delays, |_job| {
                calls.set(calls.get() + 1);
                Err(AppError::Analysis("未安装模型“small”。".to_owned()))
            })
            .unwrap_err();
        assert_eq!(calls.get(), 1);
        assert!(error.to_string().contains("未安装"));
        let _ = std::fs::remove_dir_all(library.root());
    }

    #[test]
    fn retry_budget_is_finite() {
        let library = scratch_library("retry-budget");
        let (record_id, job) = retry_test_setup(&library, "budget");
        let calls = std::cell::Cell::new(0);
        let delays = [Duration::from_millis(1), Duration::from_millis(1)];
        assert!(
            run_pipeline_with_finite_retry(None, &library, &record_id, job, &delays, |_job| {
                calls.set(calls.get() + 1);
                Err(AppError::Io(std::io::Error::other("always")))
            })
            .is_err()
        );
        // 首次 + 2 次重试后停止。
        assert_eq!(calls.get(), 3);
        let _ = std::fs::remove_dir_all(library.root());
    }

    #[test]
    fn failure_after_transcription_succeeded_is_not_retried() {
        let library = scratch_library("retry-stage");
        let (record_id, job) = retry_test_setup(&library, "stage");
        let calls = std::cell::Cell::new(0);
        let delays = [Duration::from_millis(1), Duration::from_millis(1)];
        let repository = library.repository();
        let result =
            run_pipeline_with_finite_retry(None, &library, &record_id, job, &delays, |_job| {
                calls.set(calls.get() + 1);
                // 模拟：转写已落库（新增逐字稿版本），随后分析阶段失败。
                repository
                    .save_transcript(
                        &record_id,
                        "whisper.cpp",
                        "small",
                        &[crate::types::TranscriptSegmentInput {
                            start_ms: 0,
                            end_ms: 1_000,
                            speaker_label: None,
                            original_text: "转写已经成功落库的逐字稿".to_owned(),
                        }],
                    )
                    .unwrap();
                Err(AppError::Analysis(
                    "Ollama 请求失败。请确认本机服务仍在运行。".to_owned(),
                ))
            });
        assert!(result.is_err());
        // 转写已经成功，失败属于分析阶段：不得重跑转写。
        assert_eq!(calls.get(), 1);
        let _ = std::fs::remove_dir_all(library.root());
    }

    #[test]
    fn transcription_engine_switch_is_controllable_and_defaults_to_embedded() {
        // 块6：实验开关的契约——默认 embedded（关）、set/get 往返一致。
        // 非法值的拒绝在命令层，走 repository 直读时以默认值兜底。
        let library = scratch_library("engine_switch");
        let repository = library.repository();
        assert_eq!(
            repository
                .setting_value("transcription_engine")
                .unwrap()
                .as_deref(),
            None,
            "未设置时命令层回退 embedded（默认关）"
        );
        repository
            .set_setting_value("transcription_engine", "whisperx")
            .unwrap();
        assert_eq!(
            repository.setting_value("transcription_engine").unwrap().as_deref(),
            Some("whisperx")
        );
        repository
            .set_setting_value("transcription_engine", "embedded")
            .unwrap();
        assert_eq!(
            repository.setting_value("transcription_engine").unwrap().as_deref(),
            Some("embedded")
        );
        let _ = std::fs::remove_dir_all(library.root());
    }

    // ---- Phase 0：保存并验证 / 草稿诊断的副作用边界 ----

    fn save_and_verify_args(token: &str) -> SaveAndVerifyArgs {
        SaveAndVerifyArgs {
            target: "analysis".to_owned(),
            provider_id: "dashscope".to_owned(),
            base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1".to_owned(),
            model: "qwen-turbo".to_owned(),
            credentials: BTreeMap::from([("apiKey".to_owned(), "test-key-fake".to_owned())]),
            privacy_consent_at: Some("2026-11-11T00:00:00Z".to_owned()),
            audio_upload_consent_at: None,
            enabled: true,
            verification_token: token.to_owned(),
        }
    }

    fn snapshot_provider_keys(library: &ManagedLibrary) -> Vec<(String, Option<String>)> {
        let repository = library.repository();
        [
            "external_ai_provider",
            "external_ai_base_url",
            "external_ai_model",
            "external_ai_enabled",
            "external_ai_privacy_consent_at",
            "external_ai_credential_revision.analysis",
        ]
        .iter()
        .map(|key| ((*key).to_owned(), repository.setting_value(key).unwrap()))
        .collect()
    }

    #[test]
    fn save_and_verify_rejects_invalid_ticket_without_any_side_effects() {
        let library = scratch_library("save_verify_no_ticket");
        let state = crate::state::AppState::initialize(library.root().to_path_buf()).unwrap();
        let before = snapshot_provider_keys(&state.library);

        let error = save_and_verify_provider_blocking(&state, save_and_verify_args("bogus-token"))
            .unwrap_err();
        assert!(error.contains("重新验证"), "拒绝话术要引导重新测试：{error}");

        let after = snapshot_provider_keys(&state.library);
        assert_eq!(before, after, "票据无效时不得有任何落盘变化");
        let _ = std::fs::remove_dir_all(state.library.root());
    }

    #[test]
    fn save_and_verify_args_deserialize_from_camel_case_frontend_payload() {
        // 前端（tauri.ts saveAndVerifyProvider）发的是 camelCase 载荷。
        // Tauri 只对顶层命令参数做 snake→camel 转换，结构体参数不会——
        // 少了 rename_all 就会在真机上报 missing field `provider_id`
        // （UI mock 测试抓不到这类契约错误，故在此锁定）。
        // 测试用假凭据：仅供断言的占位值，不是真密钥。
        let draft_credentials = BTreeMap::from([("apiKey".to_owned(), "mock-draft-value".to_owned())]);
        let payload = serde_json::json!({
            "target": "analysis",
            "providerId": "dashscope",
            "baseUrl": "https://dashscope.aliyuncs.com/compatible-mode/v1",
            "model": "qwen-turbo",
            "credentials": draft_credentials,
            "privacyConsentAt": "2026-11-11T00:00:00Z",
            "audioUploadConsentAt": null,
            "enabled": true,
            "verificationToken": "test-token",
        });
        let args: SaveAndVerifyArgs = serde_json::from_value(payload).unwrap();
        assert_eq!(args.target, "analysis");
        assert_eq!(args.provider_id, "dashscope");
        assert_eq!(args.model, "qwen-turbo");
        assert_eq!(
            args.credentials.get("apiKey").map(String::as_str),
            Some("mock-draft-value")
        );
        assert_eq!(args.verification_token, "test-token");
        assert!(args.enabled);
    }

    #[test]
    fn draft_diagnosis_with_explicit_credentials_never_touches_database() {
        let library = scratch_library("diagnose_no_db");
        let state = crate::state::AppState::initialize(library.root().to_path_buf()).unwrap();
        state
            .library
            .repository()
            .set_setting_value("external_ai_provider", "zhipu")
            .unwrap();
        state
            .library
            .repository()
            .set_setting_value("external_ai_model", "glm-4-flash")
            .unwrap();
        let before = snapshot_provider_keys(&state.library);

        // 起 fixture：诊断应通过，且全程不写任何设置。
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let body = serde_json::json!({
                "choices": [{"message": {"content": "{\"ok\":true}"}}]
            })
            .to_string();
            let (mut stream, _) = listener.accept().unwrap();
            let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
            let mut request = Vec::new();
            let mut buffer = [0_u8; 65_536];
            loop {
                match stream.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(read) => request.extend_from_slice(&buffer[..read]),
                }
            }
            let _ = request;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes());
        });
        let report = crate::diagnosis::diagnose_external_provider_blocking(
            Some(&state.library),
            crate::diagnosis::DiagnoseInput {
                target: "analysis".to_owned(),
                provider_id: "custom".to_owned(),
                base_url: format!("http://{address}"),
                model: "test-model".to_owned(),
                credentials: BTreeMap::from([("apiKey".to_owned(), "test-key-fake".to_owned())]),
            },
        )
        .unwrap();
        handle.join().unwrap();
        assert_eq!(report.status, "passed");

        let after = snapshot_provider_keys(&state.library);
        assert_eq!(before, after, "草稿诊断不得写数据库");
        let _ = std::fs::remove_dir_all(state.library.root());
    }
}
