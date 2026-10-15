//! 磁盘可用空间探测与写前预检。
//!
//! 设计取舍（对齐得到大脑逆向报告里同类实践的经验）：宁可提前拒绝、给出明确报错，
//! 也不要在拷贝/转码/下载写到一半时因空间不足留下半成品——半成品比失败更难收拾。

use crate::error::{AppError, AppResult};
use std::path::Path;

/// 导入预检的保底余量：除源文件副本外，还要留出数据库、索引与后续中间产物的余量。
pub const IMPORT_FREE_SPACE_FLOOR_BYTES: u64 = 256 * 1024 * 1024;
/// 预处理预检的保底余量。
pub const PREPROCESS_FREE_SPACE_FLOOR_BYTES: u64 = 256 * 1024 * 1024;
/// 模型下载预检的保底余量（临时文件与最终文件同目录，校验通过才改名）。
pub const MODEL_DOWNLOAD_FREE_SPACE_FLOOR_BYTES: u64 = 512 * 1024 * 1024;
/// 低于该可用空间时向用户显示提醒横幅（只提醒，不阻塞既有资料库的使用）。
pub const LOW_SPACE_WARNING_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// 逐级向上找到第一个真实存在的祖先目录：调用方给的常是尚未创建的目录
/// （如 `models/`、`raw/{record_id}/`），statfs 需要一个存在的路径。
fn nearest_existing(path: &Path) -> &Path {
    let mut current = path;
    while !current.exists() {
        match current.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => current = parent,
            _ => break,
        }
    }
    current
}

/// 查询路径所在卷的可用字节（非 unix 平台不做预检，返回上限值）。
#[cfg(unix)]
pub fn available_bytes(path: &Path) -> AppResult<u64> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let target = nearest_existing(path);
    let c_path = CString::new(target.as_os_str().as_bytes())
        .map_err(|_| AppError::Storage(format!("无法解析路径：{}", target.display())))?;
    let mut stats: libc::statfs = unsafe { std::mem::zeroed() };
    let result = unsafe { libc::statfs(c_path.as_ptr(), &mut stats) };
    if result != 0 {
        return Err(AppError::Storage(format!(
            "无法查询磁盘可用空间：{}",
            std::io::Error::last_os_error()
        )));
    }
    Ok((stats.f_bavail as u64).saturating_mul(stats.f_bsize as u64))
}

#[cfg(not(unix))]
pub fn available_bytes(_path: &Path) -> AppResult<u64> {
    Ok(u64::MAX)
}

/// 导入音频需要的空间：源文件大小 + 20% 浮动 + 保底余量。
pub fn required_import_bytes(source_bytes: u64) -> u64 {
    source_bytes
        .saturating_add(source_bytes / 5)
        .saturating_add(IMPORT_FREE_SPACE_FLOOR_BYTES)
}

/// 预处理需要的空间：产物是 16kHz 单声道 16bit WAV，压缩源（mp3/m4a）约放大 2 倍，
/// 这里按 3 倍估算再留保底余量。
pub fn required_preprocess_bytes(input_bytes: u64) -> u64 {
    input_bytes
        .saturating_mul(3)
        .saturating_add(PREPROCESS_FREE_SPACE_FLOOR_BYTES)
}

/// 模型下载需要的空间：模型大小 + 20% + 保底余量。
pub fn required_model_download_bytes(model_bytes: u64) -> u64 {
    model_bytes
        .saturating_add(model_bytes / 5)
        .saturating_add(MODEL_DOWNLOAD_FREE_SPACE_FLOOR_BYTES)
}

/// 写前预检：空间不足时返回明确的存储错误，调用方据此中止操作。
pub fn ensure_free_space(target: &Path, required_bytes: u64) -> AppResult<()> {
    let available = available_bytes(target)?;
    if available < required_bytes {
        return Err(AppError::Storage(format!(
            "磁盘可用空间不足：本次操作约需 {}，当前可用 {}。请先释放磁盘空间再试。",
            human_bytes(required_bytes),
            human_bytes(available)
        )));
    }
    Ok(())
}

/// 人类可读的体量文案（用于错误消息与存储面板）。
pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn required_space_includes_floor_and_headroom() {
        assert_eq!(
            required_import_bytes(100 * 1024 * 1024),
            120 * 1024 * 1024 + IMPORT_FREE_SPACE_FLOOR_BYTES
        );
        assert_eq!(
            required_preprocess_bytes(10 * 1024 * 1024),
            30 * 1024 * 1024 + PREPROCESS_FREE_SPACE_FLOOR_BYTES
        );
        assert_eq!(
            required_model_download_bytes(574_041_195),
            574_041_195 + 574_041_195 / 5 + MODEL_DOWNLOAD_FREE_SPACE_FLOOR_BYTES
        );
    }

    #[test]
    fn huge_requirement_is_rejected_with_storage_error() {
        let target = std::env::temp_dir();
        let error = ensure_free_space(&target, u64::MAX / 2).unwrap_err();
        let message = error.to_string();
        assert!(message.contains("磁盘可用空间不足"), "{message}");
    }

    #[test]
    fn small_requirement_passes_on_real_volume() {
        let target = std::env::temp_dir();
        ensure_free_space(&target, 1024).unwrap();
        let available = available_bytes(&target).unwrap();
        assert!(available > 0);
    }

    #[test]
    fn nearest_existing_walks_up_missing_directories() {
        let missing = std::env::temp_dir()
            .join("echo-disk-space-tests")
            .join("not-created-yet")
            .join("models");
        let resolved = nearest_existing(&missing);
        assert!(resolved.exists());
        assert!(available_bytes(&missing).is_ok());
    }

    #[test]
    fn human_bytes_formats_units() {
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(2 * 1024 * 1024), "2.0 MB");
        assert_eq!(human_bytes(3 * 1024 * 1024 * 1024), "3.0 GB");
    }
}
