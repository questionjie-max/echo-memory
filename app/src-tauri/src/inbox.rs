//! 音频收件箱服务：监听用户指定的文件夹与 USB 卷，把新出现的音频文件
//! 自动导入资料库并送入转写分析队列。所有硬件保持开放——只要录音设备
//! 能把文件落到 Mac 上（U 盘直插、iCloud、AirDrop、微信另存等），收件箱就会接住。
//!
//! 采用轻量轮询（默认 4 秒）而非 FSEvents：音频文件不是高频事件，
//! 轮询同时天然承担“文件写稳检测”（连续两轮大小与修改时间不变才入队），
//! 且避免了外部依赖与权限边界的复杂度。

use crate::error::AppResult;
use crate::library::ManagedLibrary;
use crate::state::heavy_job_limiter;
use crate::types::InboxSeenFile;
use chrono::Utc;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tauri::Emitter;

const SCAN_INTERVAL: Duration = Duration::from_secs(4);
const MAX_FILES_PER_SCAN: usize = 200;
const SUPPORTED_EXTENSIONS: [&str; 4] = ["mp3", "m4a", "wav", "aac"];

static RESCAN_REQUESTED: AtomicBool = AtomicBool::new(false);

/// 立即触发一次扫描（用户点了“立即扫描”或改了监听配置）。
pub fn request_rescan() {
    RESCAN_REQUESTED.store(true, Ordering::SeqCst);
}

/// 启动收件箱协调线程。整个应用生命周期只启动一次。
pub fn start(app: tauri::AppHandle, library_root: PathBuf) {
    std::thread::Builder::new()
        .name("inbox-coordinator".to_owned())
        .spawn(move || {
            // 上一轮看到的候选文件（路径 → 大小+mtime），用于写稳检测。
            let mut previous: HashMap<PathBuf, (u64, i64)> = HashMap::new();
            loop {
                if RESCAN_REQUESTED.swap(false, Ordering::SeqCst) {
                    previous.clear();
                }
                let candidates = collect_stable_candidates(&library_root, &mut previous);
                for seen in candidates {
                    spawn_import_worker(app.clone(), library_root.clone(), seen);
                }
                std::thread::sleep(SCAN_INTERVAL);
            }
        })
        .expect("启动收件箱协调线程失败");
}

/// 收集本轮“已写稳且从未见过”的音频文件，写入 inbox_seen_files（pending）。
fn collect_stable_candidates(
    library_root: &Path,
    previous: &mut HashMap<PathBuf, (u64, i64)>,
) -> Vec<InboxSeenFile> {
    let mut candidates = Vec::new();
    let Ok(library) = ManagedLibrary::open(library_root.to_path_buf()) else {
        return candidates;
    };
    let repository = library.repository();
    let folders = repository.list_watch_folders().unwrap_or_default();
    let usb_enabled = repository.inbox_usb_detection().unwrap_or(false);

    let mut sources: Vec<(String, String)> = folders
        .into_iter()
        .filter(|folder| folder.enabled)
        .map(|folder| ("folder".to_owned(), folder.path))
        .collect();
    if usb_enabled {
        if let Ok(entries) = std::fs::read_dir("/Volumes") {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    sources.push(("volume".to_owned(), path.to_string_lossy().to_string()));
                }
            }
        }
    }

    // 本轮观察到的全部音频文件快照。
    let mut current: HashMap<PathBuf, (u64, i64)> = HashMap::new();
    for (_source_kind, source_path) in &sources {
        let root = PathBuf::from(source_path);
        for file_path in walk_audio_files(&root, 0) {
            let Ok(metadata) = std::fs::metadata(&file_path) else {
                continue;
            };
            let size = metadata.len();
            let Ok(modified) = metadata.modified() else {
                continue;
            };
            let mtime_ms = modified
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_millis() as i64)
                .unwrap_or(0);
            current.insert(file_path, (size, mtime_ms));
        }
    }

    let now = Utc::now().to_rfc3339();
    for (file_path, (size, mtime_ms)) in &current {
        if candidates.len() >= MAX_FILES_PER_SCAN {
            break;
        }
        let file_name = file_path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_default();
        if file_name.is_empty() {
            continue;
        }
        // 与上一轮一致 → 文件已写稳；上一轮没有 → 等下一轮再判断。
        let stable = previous
            .get(file_path)
            .is_some_and(|(prev_size, prev_mtime)| prev_size == size && prev_mtime == mtime_ms);
        if !stable {
            continue;
        }
        let source = sources
            .iter()
            .find(|(_, source_path)| file_path.starts_with(PathBuf::from(source_path)));
        let (source_kind, source_path) = match source {
            Some((kind, path)) => (kind.clone(), path.clone()),
            None => continue,
        };
        // 已处理过的文件永不重复入队。
        if repository
            .seen_file_by_path(&file_path.to_string_lossy())
            .ok()
            .flatten()
            .is_some()
        {
            continue;
        }
        let seen = InboxSeenFile {
            id: uuid::Uuid::new_v4().to_string(),
            source_kind,
            source_path,
            file_path: file_path.to_string_lossy().to_string(),
            file_name,
            file_size: *size as i64,
            mtime_ms: *mtime_ms,
            sha256: None,
            status: "pending".to_owned(),
            record_id: None,
            error_message: None,
            seen_at: now.clone(),
            updated_at: now.clone(),
        };
        if repository.insert_seen_file(&seen).is_ok() {
            candidates.push(seen);
        }
    }

    *previous = current;
    candidates
}

/// 深度受限的音频文件遍历（U 盘录音笔常见 RECORD/FOLDERxxx/ 层级）。
fn walk_audio_files(directory: &Path, depth: usize) -> Vec<PathBuf> {
    let mut files = Vec::new();
    if depth > 3 {
        return files;
    }
    let Ok(entries) = std::fs::read_dir(directory) else {
        return files;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            files.extend(walk_audio_files(&path, depth + 1));
            continue;
        }
        let extension = path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(str::to_ascii_lowercase)
            .unwrap_or_default();
        // iCloud 未下载完成的占位文件（*.icloud）一律跳过，等物化后按真实文件名接住。
        if path.to_string_lossy().ends_with(".icloud") {
            continue;
        }
        if SUPPORTED_EXTENSIONS.contains(&extension.as_str()) {
            files.push(path);
        }
    }
    files
}

fn spawn_import_worker(app: tauri::AppHandle, library_root: PathBuf, seen: InboxSeenFile) {
    let limiter = heavy_job_limiter().clone();
    std::thread::Builder::new()
        .name("inbox-import".to_owned())
        .spawn(move || {
            let _permit = limiter.acquire();
            let status = process_file(&library_root, &seen);
            if let Ok(library) = ManagedLibrary::open(library_root) {
                let repository = library.repository();
                match &status {
                    Ok(record_id) => {
                        let _ = repository.update_seen_file_status(
                            &seen.id,
                            "imported",
                            Some(record_id),
                            None,
                        );
                    }
                    Err((status_label, message)) => {
                        let _ = repository.update_seen_file_status(
                            &seen.id,
                            status_label,
                            None,
                            Some(message),
                        );
                    }
                }
                let _ = app.emit("inbox-update", ());
            }
        })
        .expect("启动收件箱导入线程失败");
}

/// 单文件完整管线：去重 → 导入 → 转写 → 分析 →（可选）校对。
/// 返回 Err(状态标签, 错误信息) 供收件箱面板展示。
fn process_file(library_root: &Path, seen: &InboxSeenFile) -> Result<String, (String, String)> {
    let library = ManagedLibrary::open(library_root.to_path_buf())
        .map_err(|error| ("failed".to_owned(), error.to_string()))?;
    let repository = library.repository();
    let source = PathBuf::from(&seen.file_path);

    let hash = crate::library::sha256_file_public(&source)
        .map_err(|error| ("failed".to_owned(), error.to_string()))?;
    if repository
        .find_record_by_hash(&hash)
        .map_err(|error| ("failed".to_owned(), error.to_string()))?
        .is_some()
    {
        return Err(("duplicate".to_owned(), "内容已存在于资料库".to_owned()));
    }

    let ingest = library
        .import_audio(&source, None, false)
        .map_err(|error| ("failed".to_owned(), error.to_string()))?;
    let record_id = ingest.record_id;

    let _ = repository.update_seen_file_status(&seen.id, "importing", Some(&record_id), None);
    if let Err(error) = crate::commands::transcribe_with_library(&library, &record_id) {
        return Err(("failed".to_owned(), format!("转写失败：{error}")));
    }
    if let Err(error) = crate::commands::analyze_with_library(&library, &record_id) {
        return Err(("failed".to_owned(), format!("分析失败：{error}")));
    }
    if repository
        .setting_value("transcript_correction_enabled")
        .ok()
        .flatten()
        .is_some_and(|value| value == "true")
    {
        let _ = crate::analysis::correct_transcript_with_library(&library, &record_id);
    }
    Ok(record_id)
}

/// 推荐的监听目录（引导向导用）：下载文件夹与 iCloud 云盘根目录。
pub fn suggest_watch_folders() -> Vec<(String, String)> {
    let mut suggestions = Vec::new();
    if let Some(home) = std::env::var_os("HOME") {
        let downloads = PathBuf::from(&home).join("Downloads");
        if downloads.is_dir() {
            suggestions.push((
                "下载文件夹（AirDrop / 浏览器下载落点）".to_owned(),
                downloads.to_string_lossy().to_string(),
            ));
        }
        let cloud_root = PathBuf::from(&home)
            .join("Library")
            .join("Mobile Documents")
            .join("com~apple~CloudDocs");
        if cloud_root.is_dir() {
            suggestions.push((
                "iCloud 云盘（iPhone 文件无线同步）".to_owned(),
                cloud_root.to_string_lossy().to_string(),
            ));
        }
    }
    suggestions
}

/// 校验一个候选监听目录。
pub fn validate_watch_folder(path: &str) -> AppResult<PathBuf> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err(crate::error::AppError::Invalid("目录不能为空".to_owned()));
    }
    let candidate = PathBuf::from(trimmed);
    if !candidate.is_dir() {
        return Err(crate::error::AppError::Invalid(
            "目录不存在，请检查路径".to_owned(),
        ));
    }
    Ok(candidate)
}
