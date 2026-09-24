//! 资料库仓库层（M1a）。
//! 封装 projects / records / processing_jobs 三张表的全量 CRUD。
//! UI 与 MCP 都经由此层访问 SQLite，不直接触碰连接（见「架构与数据.md」边界约定）。
//!
//! 连接策略：每次操作打开一个短连接并开启外键约束。桌面单进程写入场景下足够，
//! 且避免长连接跨线程共享的复杂度。

use crate::error::{AppError, AppResult};
use chrono::Utc;
use rusqlite::{params, Connection};
use serde::Deserialize;
use std::path::{Path, PathBuf};

mod action_dashboard;
mod analysis;
mod dock;
mod hotwords;
mod inbox;
mod knowledge;
mod mcp;
mod memory_snapshots;
mod processing_jobs;
mod projects;
mod records;
mod search;
mod settings;
mod speakers;
mod transcripts;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DecisionReferenceRow {
    record_id: String,
    record_title: String,
    text: String,
    citation_segment_ids: Vec<String>,
    quote_text: String,
    start_ms: Option<i64>,
    end_ms: Option<i64>,
}

#[derive(Clone)]
pub struct LibraryRepository {
    db_path: PathBuf,
}

impl LibraryRepository {
    /// 打开（或创建）资料库，并确保 schema 已迁移到最新。
    pub fn new(db_path: impl Into<PathBuf>) -> AppResult<Self> {
        let db_path = db_path.into();
        super::run_migrations(&db_path)?;
        let repository = Self { db_path };
        repository.backfill_normalized_transcripts()?;
        Ok(repository)
    }

    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    fn connect(&self) -> AppResult<Connection> {
        let connection = Connection::open(&self.db_path)?;
        // busy_timeout：转写/分析/dock/MCP 多线程并发写时，等锁而不是立刻
        // 报「database is locked」（rusqlite 默认超时为 0，一撞就失败）。
        connection.execute_batch("PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 5000;")?;
        Ok(connection)
    }

    /* ------------------------------ 私有辅助 ---------------------------- */

    fn invalidate_record_knowledge(
        transaction: &rusqlite::Transaction<'_>,
        record_id: &str,
        stale_analysis: bool,
    ) -> AppResult<()> {
        if stale_analysis {
            transaction.execute(
                "UPDATE analyses SET status = 'stale' WHERE record_id = ?1 AND status IN ('completed', 'incomplete')",
                params![record_id],
            )?;
            transaction.execute(
                "DELETE FROM record_search WHERE source_id IN (SELECT id FROM analyses WHERE record_id = ?1)",
                params![record_id],
            )?;
            transaction.execute(
                "DELETE FROM action_items WHERE record_id = ?1",
                params![record_id],
            )?;
        }
        transaction.execute(
            "DELETE FROM knowledge_chunks WHERE record_id = ?1",
            params![record_id],
        )?;
        transaction.execute(
            "UPDATE knowledge_index_state SET status = 'stale', last_error = NULL, updated_at = ?1 WHERE status != 'not_built'",
            params![Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    fn validate_processing_status(status: &str) -> AppResult<()> {
        const ALLOWED: [&str; 6] = [
            "queued",
            "preparing",
            "transcribing",
            "analyzing",
            "completed",
            "failed",
        ];
        if ALLOWED.contains(&status) {
            Ok(())
        } else {
            Err(AppError::Invalid(format!("非法处理状态: {status}")))
        }
    }
}
