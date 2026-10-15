//! 应用错误类型。M1a 起用于仓库层与命令层。

use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("数据库错误: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("文件系统错误: {0}")]
    Io(#[from] std::io::Error),
    #[error("未找到: {0}")]
    NotFound(String),
    #[error("参数无效: {0}")]
    Invalid(String),
    #[error("音频导入错误: {0}")]
    Import(String),
    /// 磁盘空间不足等存储前置条件不满足：明确归类，便于调用方判定「重试无用」。
    #[error("存储不可用: {0}")]
    Storage(String),
    #[error("本地分析错误: {0}")]
    Analysis(String),
    #[error("外部 AI 错误: {0}")]
    ExternalAi(String),
}

/// 后端统一结果别名。
pub type AppResult<T> = Result<T, AppError>;

impl AppError {
    /// 转为可跨 Tauri 边界返回给前端的字符串（命令层统一用 Result<T, String>）。
    pub fn to_frontend(&self) -> String {
        self.to_string()
    }
}
