//! Tauri 托管应用状态。资料库只包含本地文件与 SQLite。

use crate::error::AppResult;
use crate::library::ManagedLibrary;
use std::path::PathBuf;

pub struct AppState {
    pub library: ManagedLibrary,
}

impl AppState {
    /// 依据资料库根目录初始化迁移和受管理目录。
    pub fn initialize(library_root: PathBuf) -> AppResult<Self> {
        Ok(Self {
            library: ManagedLibrary::open(library_root)?,
        })
    }
}

/// 默认资料库根目录：优先 `ECHO_LIBRARY_ROOT`，否则用户主目录下
/// `Library/Application Support/回声记忆`（对齐「架构与数据.md」macOS 约定）。
pub fn default_library_root() -> PathBuf {
    if let Ok(custom) = std::env::var("ECHO_LIBRARY_ROOT") {
        return PathBuf::from(custom);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join("Library")
        .join("Application Support")
        .join("回声记忆")
}
