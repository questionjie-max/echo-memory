use super::LibraryRepository;
use crate::error::{AppError, AppResult};
use crate::types::{
    MemoryFeedback, MemoryGenerationStatus, MemoryScope, MemorySnapshot, MemorySnapshotResult,
    MemoryViewKind,
};
use chrono::Utc;
use rusqlite::{params, OptionalExtension, Row};
use uuid::Uuid;

impl LibraryRepository {
    pub fn create_memory_snapshot(
        &self,
        view_kind: &MemoryViewKind,
        scope: &MemoryScope,
        range: (Option<&str>, Option<&str>),
        model: &str,
        source_record_ids: &[String],
        request_hash: &str,
    ) -> AppResult<MemorySnapshot> {
        let (range_start, range_end) = range;
        if !matches!(scope.kind.as_str(), "all" | "project" | "unfiled") {
            return Err(AppError::Invalid("记忆视图作用域无效".to_owned()));
        }
        if scope.kind == "project" && scope.project_id.as_deref().unwrap_or("").is_empty() {
            return Err(AppError::Invalid("项目作用域缺少项目 ID".to_owned()));
        }
        let connection = self.connect()?;
        let scope_key = scope.key();
        let version: i64 = connection.query_row(
            "SELECT COALESCE(MAX(version), 0) + 1 FROM memory_snapshots WHERE view_kind = ?1 AND scope_key = ?2 AND range_start IS ?3 AND range_end IS ?4",
            params![view_kind.as_str(), scope_key, range_start, range_end],
            |row| row.get(0),
        )?;
        let id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        connection.execute(
            "INSERT INTO memory_snapshots (id, view_kind, scope_kind, scope_key, range_start, range_end, status, provider, model, source_record_ids_json, request_hash, result_json, version, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'generating', 'openai-compatible', ?7, ?8, ?9, '{}', ?10, ?11, ?11)",
            params![id, view_kind.as_str(), scope.kind, scope_key, range_start, range_end, model, serde_json::to_string(source_record_ids).map_err(|error| AppError::Invalid(error.to_string()))?, request_hash, version, now],
        )?;
        self.get_memory_snapshot(&id)
    }

    pub fn recover_interrupted_memory_snapshots(&self) -> AppResult<usize> {
        let changed = self.connect()?.execute(
            "UPDATE memory_snapshots SET status = 'failed', error_message = COALESCE(error_message, ?1), updated_at = ?2 WHERE status = 'generating'",
            params![
                "上次生成因应用退出而中断，请重新生成",
                Utc::now().to_rfc3339()
            ],
        )?;
        Ok(changed)
    }

    pub fn finish_memory_snapshot(
        &self,
        id: &str,
        status: MemoryGenerationStatus,
        result: &MemorySnapshotResult,
        quality_warning: Option<&str>,
        error_message: Option<&str>,
    ) -> AppResult<MemorySnapshot> {
        let changed = self.connect()?.execute(
            "UPDATE memory_snapshots SET status = ?2, result_json = ?3, quality_warning = ?4, error_message = ?5, updated_at = ?6 WHERE id = ?1",
            params![id, status.as_str(), serde_json::to_string(result).map_err(|error| AppError::Invalid(format!("记忆快照结果无效: {error}")))?, quality_warning, error_message, Utc::now().to_rfc3339()],
        )?;
        if changed == 0 {
            return Err(AppError::NotFound(format!("memory snapshot {id}")));
        }
        self.get_memory_snapshot(id)
    }

    pub fn list_memory_snapshots(
        &self,
        view_kind: &MemoryViewKind,
        scope: &MemoryScope,
        range_start: Option<&str>,
        range_end: Option<&str>,
    ) -> AppResult<Vec<MemorySnapshot>> {
        let connection = self.connect()?;
        let mut statement = connection.prepare(
            "SELECT id, view_kind, scope_kind, scope_key, range_start, range_end, status, provider, model, source_record_ids_json, request_hash, result_json, quality_warning, error_message, is_stale, version, created_at, updated_at FROM memory_snapshots WHERE view_kind = ?1 AND scope_key = ?2 AND range_start IS ?3 AND range_end IS ?4 ORDER BY version DESC",
        )?;
        let rows = statement
            .query_map(
                params![view_kind.as_str(), scope.key(), range_start, range_end],
                Self::map_memory_snapshot,
            )?
            .collect::<Result<Vec<_>, rusqlite::Error>>()?;
        Ok(rows)
    }

    pub fn get_memory_snapshot(&self, id: &str) -> AppResult<MemorySnapshot> {
        self.connect()?
            .query_row(
                "SELECT id, view_kind, scope_kind, scope_key, range_start, range_end, status, provider, model, source_record_ids_json, request_hash, result_json, quality_warning, error_message, is_stale, version, created_at, updated_at FROM memory_snapshots WHERE id = ?1",
                params![id],
                Self::map_memory_snapshot,
            )
            .optional()?
            .ok_or_else(|| AppError::NotFound(format!("memory snapshot {id}")))
    }

    pub fn update_memory_feedback(
        &self,
        snapshot_id: &str,
        item_id: &str,
        decision: &str,
        note: &str,
    ) -> AppResult<MemoryFeedback> {
        if !matches!(decision, "confirmed" | "rejected") {
            return Err(AppError::Invalid(
                "反馈状态必须为 confirmed 或 rejected".to_owned(),
            ));
        }
        let snapshot = self.get_memory_snapshot(snapshot_id)?;
        let item_exists = snapshot
            .result
            .timeline_items
            .iter()
            .any(|item| item.id == item_id)
            || snapshot.result.nodes.iter().any(|item| item.id == item_id)
            || snapshot.result.edges.iter().any(|item| item.id == item_id)
            || snapshot
                .result
                .evolution_items
                .iter()
                .any(|item| item.id == item_id);
        if !item_exists {
            return Err(AppError::Invalid("反馈条目不存在于该记忆快照".to_owned()));
        }
        let connection = self.connect()?;
        let existing: Option<(String, String)> = connection
            .query_row(
                "SELECT id, created_at FROM memory_feedback WHERE snapshot_id = ?1 AND item_id = ?2",
                params![snapshot_id, item_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let now = Utc::now().to_rfc3339();
        let (id, created_at) =
            existing.unwrap_or_else(|| (Uuid::new_v4().to_string(), now.clone()));
        connection.execute(
            "INSERT INTO memory_feedback (id, snapshot_id, item_id, decision, note, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) ON CONFLICT(snapshot_id, item_id) DO UPDATE SET decision = excluded.decision, note = excluded.note, updated_at = excluded.updated_at",
            params![id, snapshot_id, item_id, decision, note.trim(), created_at, now],
        )?;
        connection.query_row(
            "SELECT id, snapshot_id, item_id, decision, note, created_at, updated_at FROM memory_feedback WHERE snapshot_id = ?1 AND item_id = ?2",
            params![snapshot_id, item_id],
            |row| Ok(MemoryFeedback { id: row.get(0)?, snapshot_id: row.get(1)?, item_id: row.get(2)?, decision: row.get(3)?, note: row.get(4)?, created_at: row.get(5)?, updated_at: row.get(6)? }),
        ).map_err(AppError::from)
    }

    pub fn list_memory_feedback(&self, snapshot_id: &str) -> AppResult<Vec<MemoryFeedback>> {
        let connection = self.connect()?;
        let mut statement = connection.prepare(
            "SELECT id, snapshot_id, item_id, decision, note, created_at, updated_at FROM memory_feedback WHERE snapshot_id = ?1 ORDER BY updated_at DESC",
        )?;
        let rows = statement
            .query_map(params![snapshot_id], |row| {
                Ok(MemoryFeedback {
                    id: row.get(0)?,
                    snapshot_id: row.get(1)?,
                    item_id: row.get(2)?,
                    decision: row.get(3)?,
                    note: row.get(4)?,
                    created_at: row.get(5)?,
                    updated_at: row.get(6)?,
                })
            })?
            .collect::<Result<Vec<_>, rusqlite::Error>>()?;
        Ok(rows)
    }

    pub fn feedback_context(
        &self,
        view_kind: &MemoryViewKind,
        scope: &MemoryScope,
    ) -> AppResult<Vec<MemoryFeedback>> {
        let connection = self.connect()?;
        let mut statement = connection.prepare(
            "SELECT feedback.id, feedback.snapshot_id, feedback.item_id, feedback.decision, feedback.note, feedback.created_at, feedback.updated_at FROM memory_feedback feedback JOIN memory_snapshots snapshots ON snapshots.id = feedback.snapshot_id WHERE snapshots.view_kind = ?1 AND snapshots.scope_key = ?2 ORDER BY feedback.updated_at DESC LIMIT 200",
        )?;
        let rows = statement
            .query_map(params![view_kind.as_str(), scope.key()], |row| {
                Ok(MemoryFeedback {
                    id: row.get(0)?,
                    snapshot_id: row.get(1)?,
                    item_id: row.get(2)?,
                    decision: row.get(3)?,
                    note: row.get(4)?,
                    created_at: row.get(5)?,
                    updated_at: row.get(6)?,
                })
            })?
            .collect::<Result<Vec<_>, rusqlite::Error>>()?;
        Ok(rows)
    }

    fn map_memory_snapshot(row: &Row<'_>) -> rusqlite::Result<MemorySnapshot> {
        let view: String = row.get(1)?;
        let status: String = row.get(6)?;
        let source_json: String = row.get(9)?;
        let result_json: String = row.get(11)?;
        let view_kind = match view.as_str() {
            "timeline" => MemoryViewKind::Timeline,
            "map" => MemoryViewKind::Map,
            "evolution" => MemoryViewKind::Evolution,
            _ => return Err(rusqlite::Error::InvalidQuery),
        };
        let generation_status = match status.as_str() {
            "generating" => MemoryGenerationStatus::Generating,
            "completed" => MemoryGenerationStatus::Completed,
            "partial" => MemoryGenerationStatus::Partial,
            "failed" => MemoryGenerationStatus::Failed,
            "cancelled" => MemoryGenerationStatus::Cancelled,
            _ => return Err(rusqlite::Error::InvalidQuery),
        };
        let project_id = row
            .get::<_, Option<String>>(3)?
            .and_then(|key| key.strip_prefix("project:").map(ToOwned::to_owned));
        Ok(MemorySnapshot {
            id: row.get(0)?,
            view_kind,
            scope: MemoryScope {
                kind: row.get(2)?,
                project_id,
            },
            range_start: row.get(4)?,
            range_end: row.get(5)?,
            status: generation_status,
            provider: row.get(7)?,
            model: row.get(8)?,
            source_record_ids: serde_json::from_str(&source_json).unwrap_or_default(),
            request_hash: row.get(10)?,
            result: serde_json::from_str(&result_json).unwrap_or_default(),
            quality_warning: row.get(12)?,
            error_message: row.get(13)?,
            is_stale: row.get::<_, i64>(14)? != 0,
            version: row.get(15)?,
            created_at: row.get(16)?,
            updated_at: row.get(17)?,
        })
    }
}
