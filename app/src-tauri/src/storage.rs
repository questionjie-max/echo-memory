//! 资料库存储可见性与无损清理。
//!
//! 设计取舍（对齐得到大脑逆向报告的对照结论）：不做「原件自动过期删除」。对方的录音原件
//! 只是云端副本的本地缓存，7 天过期删掉没有损失；我们的音频是本地资产——删掉就永久失去
//! 回听与重新分析能力。因此这里的清理只动**可再生的中间文件**（raw/ 下的预处理产物），
//! 原件库存用「可见 + 可控 + 低空间告警」来管理，而不是自动删除。

use crate::error::AppResult;
use crate::library::ManagedLibrary;
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageReport {
    pub library_path: String,
    pub total_bytes: u64,
    pub audio_bytes: u64,
    pub raw_bytes: u64,
    pub models_bytes: u64,
    pub exports_bytes: u64,
    pub database_bytes: u64,
    pub available_bytes: u64,
    /// 低空间提醒阈值（与磁盘横幅同源，避免前端各自硬编码）。
    pub low_space_threshold_bytes: u64,
    /// 可清理的中间文件体量（预览值；预览不删除任何东西）。
    pub cleanup_bytes: u64,
    pub cleanup_files: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageCleanupResult {
    pub removed_bytes: u64,
    pub removed_files: u64,
}

struct CleanupCandidate {
    path: PathBuf,
    bytes: u64,
    files: u64,
}

/// 递归统计目录体量；单项读取失败按 0 计（统计不因个别文件报错而失败）。
pub fn directory_size(path: &Path) -> u64 {
    let Ok(entries) = fs::read_dir(path) else {
        return 0;
    };
    let mut total = 0_u64;
    for entry in entries.flatten() {
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if metadata.is_dir() {
            total = total.saturating_add(directory_size(&entry.path()));
        } else {
            total = total.saturating_add(metadata.len());
        }
    }
    total
}

pub fn storage_report(library: &ManagedLibrary) -> AppResult<StorageReport> {
    let root = library.root();
    let (cleanup_bytes, cleanup_files) = preview_cleanup(library)?;
    let database_bytes = ["memory.db", "memory.db-wal", "memory.db-shm"]
        .iter()
        .map(|name| fs::metadata(root.join(name)).map_or(0, |meta| meta.len()))
        .sum();
    let audio_bytes = directory_size(&root.join("audio"));
    let raw_bytes = directory_size(&root.join("raw"));
    let models_bytes = directory_size(&root.join("models"));
    let exports_bytes = directory_size(&root.join("exports"));
    let available_bytes = crate::disk_space::available_bytes(root).unwrap_or(0);
    Ok(StorageReport {
        library_path: root.to_string_lossy().to_string(),
        total_bytes: audio_bytes + raw_bytes + models_bytes + exports_bytes + database_bytes,
        audio_bytes,
        raw_bytes,
        models_bytes,
        exports_bytes,
        database_bytes,
        available_bytes,
        low_space_threshold_bytes: crate::disk_space::LOW_SPACE_WARNING_BYTES,
        cleanup_bytes,
        cleanup_files,
    })
}

/// 无损清理：删除可再生的中间文件，返回释放体量。原件与产出永不自动删除。
pub fn run_cleanup(library: &ManagedLibrary) -> AppResult<StorageCleanupResult> {
    let candidates = cleanup_candidates(library)?;
    let mut removed_bytes = 0_u64;
    let mut removed_files = 0_u64;
    for candidate in candidates {
        // 候选都是 raw/ 下的记录目录：整个目录删除（重试/重转写会重新创建并预处理）。
        if fs::remove_dir_all(&candidate.path).is_ok() {
            removed_bytes = removed_bytes.saturating_add(candidate.bytes);
            removed_files = removed_files.saturating_add(candidate.files);
        }
    }
    Ok(StorageCleanupResult {
        removed_bytes,
        removed_files,
    })
}

/// 清理预览：与 run_cleanup 同一套候选判定，但不删除任何东西。
pub fn preview_cleanup(library: &ManagedLibrary) -> AppResult<(u64, u64)> {
    let candidates = cleanup_candidates(library)?;
    let bytes = candidates.iter().map(|item| item.bytes).sum();
    let files = candidates.iter().map(|item| item.files).sum();
    Ok((bytes, files))
}

/// 候选判定（保守）：
/// - 只处理 `raw/` 下以 UUID 命名的目录（记录目录的命名约定），其它内容一概不碰；
/// - 记录已删除 → 整个目录是无主残留；
/// - 记录存在但没有任何活跃任务（preparing/transcribing/analyzing）→ 目录内文件都是
///   上一次失败或崩溃留下的可再生产物，删除不影响重试（重试会重新预处理）。
fn cleanup_candidates(library: &ManagedLibrary) -> AppResult<Vec<CleanupCandidate>> {
    let repository = library.repository();
    let raw_root = library.root().join("raw");
    let Ok(entries) = fs::read_dir(&raw_root) else {
        return Ok(Vec::new());
    };
    let mut candidates = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !path.is_dir() || uuid::Uuid::parse_str(name).is_err() {
            continue;
        }
        let record_exists = repository.get_record(name).is_ok();
        if record_exists {
            let has_active_job = repository.list_jobs_for_record(name)?.iter().any(|job| {
                matches!(
                    job.status.as_str(),
                    "preparing" | "transcribing" | "analyzing"
                )
            });
            if has_active_job {
                continue;
            }
        }
        let (bytes, files) = directory_usage(&path);
        if files == 0 {
            continue;
        }
        candidates.push(CleanupCandidate { path, bytes, files });
    }
    Ok(candidates)
}

fn directory_usage(path: &Path) -> (u64, u64) {
    let Ok(entries) = fs::read_dir(path) else {
        return (0, 0);
    };
    let mut bytes = 0_u64;
    let mut files = 0_u64;
    for entry in entries.flatten() {
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if metadata.is_dir() {
            let (child_bytes, child_files) = directory_usage(&entry.path());
            bytes = bytes.saturating_add(child_bytes);
            files = files.saturating_add(child_files);
        } else {
            bytes = bytes.saturating_add(metadata.len());
            files = files.saturating_add(1);
        }
    }
    (bytes, files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::TranscriptSegmentInput;

    fn temp_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "echo_memory_storage_tests/{name}-{}",
            uuid::Uuid::new_v4()
        ));
        let _ = fs::remove_dir_all(&root);
        root
    }

    fn write_file(path: &Path, bytes: usize) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, vec![b'x'; bytes]).unwrap();
    }

    fn prepare_record(library: &ManagedLibrary, title: &str) -> String {
        let record = library
            .repository()
            .create_record(title, None, Path::new("audio/x.wav"), title, 1_000)
            .unwrap();
        library
            .repository()
            .save_transcript(
                &record.id,
                "whisper.cpp",
                "small",
                &[TranscriptSegmentInput {
                    start_ms: 0,
                    end_ms: 1_000,
                    speaker_label: None,
                    original_text: "存储测试逐字稿".into(),
                }],
            )
            .unwrap();
        record.id
    }

    #[test]
    fn cleanup_removes_regenerable_raw_but_never_audio() {
        let root = temp_root("cleanup");
        let library = ManagedLibrary::open(&root).unwrap();
        let record_id = prepare_record(&library, "清理测试");
        // 失败/崩溃留下的中间产物 + 用户原件（后者永不该被清理）。
        write_file(
            &root
                .join("raw")
                .join(&record_id)
                .join("leftover-enhanced.wav"),
            4_096,
        );
        write_file(
            &root.join("audio/2026/10").join(format!("{record_id}.wav")),
            8_192,
        );

        let (preview_bytes, preview_files) = preview_cleanup(&library).unwrap();
        assert_eq!(preview_files, 1);
        assert_eq!(preview_bytes, 4_096);
        // 预览不删除任何东西。
        assert!(root
            .join("raw")
            .join(&record_id)
            .join("leftover-enhanced.wav")
            .exists());

        let result = run_cleanup(&library).unwrap();
        assert_eq!(result.removed_files, 1);
        assert_eq!(result.removed_bytes, 4_096);
        assert!(!root.join("raw").join(&record_id).exists());
        // 原件仍在。
        assert!(root
            .join("audio/2026/10")
            .join(format!("{record_id}.wav"))
            .exists());
        // 第二次清理没有新增候选。
        let (again_bytes, again_files) = preview_cleanup(&library).unwrap();
        assert_eq!((again_bytes, again_files), (0, 0));
    }

    #[test]
    fn active_jobs_and_unknown_directories_are_protected() {
        let root = temp_root("protect");
        let library = ManagedLibrary::open(&root).unwrap();
        let record_id = prepare_record(&library, "活跃任务");
        let job = library
            .repository()
            .create_job(&record_id, "transcribe")
            .unwrap();
        library
            .repository()
            .update_job_status(&job.id, "transcribing", None)
            .unwrap();
        write_file(
            &root
                .join("raw")
                .join(&record_id)
                .join("active-enhanced.wav"),
            2_048,
        );
        // 非 UUID 目录属于未知内容：一概不碰。
        write_file(&root.join("raw").join("manual-notes").join("keep.txt"), 128);
        // 已删除记录的无主目录：整目录清理。
        let orphan_id = uuid::Uuid::new_v4().to_string();
        write_file(&root.join("raw").join(&orphan_id).join("orphan.wav"), 1_024);

        let (bytes, files) = preview_cleanup(&library).unwrap();
        assert_eq!(bytes, 1_024);
        assert_eq!(files, 1);

        let result = run_cleanup(&library).unwrap();
        assert_eq!(result.removed_bytes, 1_024);
        assert!(root
            .join("raw")
            .join(&record_id)
            .join("active-enhanced.wav")
            .exists());
        assert!(root
            .join("raw")
            .join("manual-notes")
            .join("keep.txt")
            .exists());
        assert!(!root.join("raw").join(&orphan_id).exists());
    }

    #[test]
    fn storage_report_counts_categories_and_herds_database_files() {
        let root = temp_root("report");
        let library = ManagedLibrary::open(&root).unwrap();
        let record_id = prepare_record(&library, "报表测试");
        write_file(
            &root.join("audio/2026/10").join(format!("{record_id}.wav")),
            8_192,
        );
        write_file(&root.join("models").join("ggml-test.bin"), 4_096);
        write_file(&root.join("exports").join("out.md"), 256);

        let report = storage_report(&library).unwrap();
        assert_eq!(report.audio_bytes, 8_192);
        assert_eq!(report.models_bytes, 4_096);
        assert_eq!(report.exports_bytes, 256);
        assert!(report.database_bytes > 0);
        assert!(report.available_bytes > 0);
        assert_eq!(
            report.total_bytes,
            report.audio_bytes
                + report.raw_bytes
                + report.models_bytes
                + report.exports_bytes
                + report.database_bytes
        );
    }
}
