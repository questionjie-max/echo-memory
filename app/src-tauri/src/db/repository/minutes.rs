use super::LibraryRepository;
use crate::error::{AppError, AppResult};
use crate::minutes::{MinutesDocument, MinutesItemKind, MinutesItemStatus};
use chrono::Utc;
use rusqlite::{params, OptionalExtension};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct MinutesRow {
    pub id: String,
    pub record_id: String,
    pub status: String,
    pub content_json: String,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub error_message: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone)]
pub struct MinutesItemRow {
    pub id: String,
    pub minutes_id: String,
    pub record_id: String,
    pub kind: String,
    pub text: String,
    pub owner_text: Option<String>,
    pub evidence_json: String,
    pub status: String,
    pub updated_at: String,
}

fn now() -> String {
    Utc::now().to_rfc3339()
}

impl LibraryRepository {
    /// 取某记录的纪要行（不含条目两态，文档本体在 content_json）。
    pub fn get_minutes(&self, record_id: &str) -> AppResult<Option<MinutesRow>> {
        self.connect()?
            .query_row(
                "SELECT id, record_id, status, content_json, provider, model, error_message, created_at, updated_at
                 FROM minutes WHERE record_id = ?1 ORDER BY created_at DESC LIMIT 1",
                params![record_id],
                |row| {
                    Ok(MinutesRow {
                        id: row.get(0)?,
                        record_id: row.get(1)?,
                        status: row.get(2)?,
                        content_json: row.get(3)?,
                        provider: row.get(4)?,
                        model: row.get(5)?,
                        error_message: row.get(6)?,
                        created_at: row.get(7)?,
                        updated_at: row.get(8)?,
                    })
                },
            )
            .optional()
            .map_err(|error| AppError::Invalid(format!("读取纪要失败：{error}")))
    }

    /// 幂等建 pending 行（转写完成后自动入队时调用；重复调用不覆盖已有行）。
    pub fn ensure_minutes_pending(&self, record_id: &str) -> AppResult<String> {
        let mut connection = self.connect()?;
        let transaction = connection.transaction().map_err(|e| AppError::Invalid(e.to_string()))?;
        let existing: Option<String> = transaction
            .query_row(
                "SELECT id FROM minutes WHERE record_id = ?1 LIMIT 1",
                params![record_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| AppError::Invalid(format!("读取纪要失败：{error}")))?;
        if let Some(id) = existing {
            return Ok(id);
        }
        let id = Uuid::new_v4().to_string();
        let now = now();
        transaction
            .execute(
                "INSERT INTO minutes (id, record_id, status, content_json, created_at, updated_at)
                 VALUES (?1, ?2, 'pending', '{}', ?3, ?3)",
                params![id, record_id, now],
            )
            .map_err(|error| AppError::Invalid(format!("创建纪要失败：{error}")))?;
        transaction.commit().map_err(|e| AppError::Invalid(e.to_string()))?;
        Ok(id)
    }

    /// 生成闩（原子）：仅当当前不在 generating、或 generating 已陈旧
    /// （超过 MINUTES_GENERATION_STALE_SECS 无心跳，属崩溃残留）时才能拿到。
    /// 返回 false 表示另一路生成正在进行，调用方不得重复启动。
    pub fn start_minutes_generation(&self, record_id: &str) -> AppResult<bool> {
        let stale_before = (Utc::now()
            - chrono::Duration::seconds(crate::minutes::MINUTES_GENERATION_STALE_SECS))
        .to_rfc3339();
        let affected = self
            .connect()?
            .execute(
                "UPDATE minutes SET status = 'generating', error_message = NULL, updated_at = ?2
                 WHERE record_id = ?1 AND (status != 'generating' OR updated_at < ?3)",
                params![record_id, now(), stale_before],
            )
            .map_err(|error| AppError::Invalid(format!("更新纪要状态失败：{error}")))?;
        Ok(affected > 0)
    }

    /// 启动恢复：崩溃/强退遗留的 generating 行复位为 failed（可重试）。
    pub fn recover_stale_minutes_generating(&self) -> AppResult<usize> {
        let stale_before = (Utc::now()
            - chrono::Duration::seconds(crate::minutes::MINUTES_GENERATION_STALE_SECS))
        .to_rfc3339();
        self.connect()?
            .execute(
                "UPDATE minutes SET status = 'failed', error_message = ?1, updated_at = ?2
                 WHERE status = 'generating' AND updated_at < ?3",
                params!["生成被中断（应用退出或异常），可重试", now(), stale_before],
            )
            .map_err(|error| AppError::Invalid(format!("恢复纪要状态失败：{error}")))
    }

    /// 生成心跳：分块间刷新 updated_at，防止长生成（多块 × 模型超时上限）
    /// 被陈旧抢占逻辑误伤成双跑。
    pub fn touch_minutes_generation(&self, record_id: &str) -> AppResult<()> {
        self.connect()?
            .execute(
                "UPDATE minutes SET updated_at = ?2 WHERE record_id = ?1 AND status = 'generating'",
                params![record_id, now()],
            )
            .map_err(|error| AppError::Invalid(format!("刷新纪要心跳失败：{error}")))?;
        Ok(())
    }

    /// 保存生成结果：文档 + 全量重建条目两态行（新条目一律 provisional）。
    /// 确认状态不跨生成保留：重新生成后条目是新的结论，必须重新确认（隐私红线）。
    pub fn save_minutes_document(
        &self,
        record_id: &str,
        provider: &str,
        model: &str,
        document: &MinutesDocument,
    ) -> AppResult<()> {
        let content_json = serde_json::to_string(document)
            .map_err(|error| AppError::Invalid(format!("纪要序列化失败：{error}")))?;
        let mut connection = self.connect()?;
        let transaction = connection.transaction().map_err(|e| AppError::Invalid(e.to_string()))?;
        let minutes_id: String = transaction
            .query_row(
                "SELECT id FROM minutes WHERE record_id = ?1 LIMIT 1",
                params![record_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| AppError::Invalid(format!("读取纪要失败：{error}")))?
            .ok_or_else(|| AppError::Invalid("纪要行不存在，无法保存".to_owned()))?;
        transaction
            .execute(
                "UPDATE minutes SET status = 'completed', content_json = ?2, provider = ?3, model = ?4,
                 error_message = NULL, updated_at = ?5 WHERE id = ?1",
                params![minutes_id, content_json, provider, model, now()],
            )
            .map_err(|error| AppError::Invalid(format!("保存纪要失败：{error}")))?;
        transaction
            .execute("DELETE FROM minutes_items WHERE minutes_id = ?1", params![minutes_id])
            .map_err(|error| AppError::Invalid(format!("清理旧条目失败：{error}")))?;
        for item in document
            .conclusions
            .iter()
            .chain(document.action_items.iter())
        {
            let evidence_json = serde_json::to_string(&item.evidence)
                .map_err(|error| AppError::Invalid(format!("证据序列化失败：{error}")))?;
            transaction
                .execute(
                    "INSERT INTO minutes_items
                     (id, minutes_id, record_id, kind, text, owner_text, evidence_json, status, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'provisional', ?8, ?8)",
                    params![
                        item.id,
                        minutes_id,
                        record_id,
                        match item.kind {
                            MinutesItemKind::Conclusion => "conclusion",
                            MinutesItemKind::Action => "action",
                        },
                        item.text,
                        item.owner,
                        evidence_json,
                        now()
                    ],
                )
                .map_err(|error| AppError::Invalid(format!("写入纪要条目失败：{error}")))?;
        }
        transaction.commit().map_err(|e| AppError::Invalid(e.to_string()))?;
        Ok(())
    }

    pub fn fail_minutes(&self, record_id: &str, message: &str) -> AppResult<()> {
        self.connect()?.execute(
            "UPDATE minutes SET status = 'failed', error_message = ?2, updated_at = ?3
             WHERE record_id = ?1",
            params![record_id, message, now()],
        )
        .map_err(|error| AppError::Invalid(format!("记录纪要失败状态：{error}")))?;
        Ok(())
    }

    pub fn get_minutes_item(&self, item_id: &str) -> AppResult<MinutesItemRow> {
        self.connect()?
            .query_row(
                "SELECT id, minutes_id, record_id, kind, text, owner_text, evidence_json, status, updated_at
                 FROM minutes_items WHERE id = ?1",
                params![item_id],
                |row| {
                    Ok(MinutesItemRow {
                        id: row.get(0)?,
                        minutes_id: row.get(1)?,
                        record_id: row.get(2)?,
                        kind: row.get(3)?,
                        text: row.get(4)?,
                        owner_text: row.get(5)?,
                        evidence_json: row.get(6)?,
                        status: row.get(7)?,
                        updated_at: row.get(8)?,
                    })
                },
            )
            .optional()
            .map_err(|error| AppError::Invalid(format!("读取纪要条目失败：{error}")))?
            .ok_or_else(|| AppError::Invalid("纪要条目不存在".to_owned()))
    }

    /// 条目两态切换。返回更新后的状态；确认/取消确认都是显式动作。
    pub fn set_minutes_item_status(
        &self,
        item_id: &str,
        status: MinutesItemStatus,
    ) -> AppResult<MinutesItemRow> {
        let status_text = match status {
            MinutesItemStatus::Provisional => "provisional",
            MinutesItemStatus::Confirmed => "confirmed",
        };
        let affected = self
            .connect()?
            .execute(
                "UPDATE minutes_items SET status = ?2, updated_at = ?3 WHERE id = ?1",
                params![item_id, status_text, now()],
            )
            .map_err(|error| AppError::Invalid(format!("更新条目状态失败：{error}")))?;
        if affected == 0 {
            return Err(AppError::Invalid("纪要条目不存在".to_owned()));
        }
        self.get_minutes_item(item_id)
    }

    /// 某记录下全部已确认条目（MCP get_my_context 只读这里）。
    pub fn list_confirmed_minutes_items(&self, limit: usize) -> AppResult<Vec<MinutesItemRow>> {
        let connection = self.connect()?;
        let mut statement = connection
            .prepare(
                "SELECT id, minutes_id, record_id, kind, text, owner_text, evidence_json, status, updated_at
                 FROM minutes_items WHERE status = 'confirmed' ORDER BY updated_at DESC LIMIT ?1",
            )
            .map_err(|error| AppError::Invalid(format!("读取已确认条目失败：{error}")))?;
        let rows = statement
            .query_map(params![limit as i64], |row| {
                Ok(MinutesItemRow {
                    id: row.get(0)?,
                    minutes_id: row.get(1)?,
                    record_id: row.get(2)?,
                    kind: row.get(3)?,
                    text: row.get(4)?,
                    owner_text: row.get(5)?,
                    evidence_json: row.get(6)?,
                    status: row.get(7)?,
                    updated_at: row.get(8)?,
                })
            })
            .map_err(|error| AppError::Invalid(format!("读取已确认条目失败：{error}")))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| AppError::Invalid(format!("读取已确认条目失败：{error}")))
    }

    /// 某记录下全部条目（前端两态展示用）。
    pub fn list_minutes_items(&self, record_id: &str) -> AppResult<Vec<MinutesItemRow>> {
        let connection = self.connect()?;
        let mut statement = connection
            .prepare(
                "SELECT id, minutes_id, record_id, kind, text, owner_text, evidence_json, status, updated_at
                 FROM minutes_items WHERE record_id = ?1 ORDER BY created_at",
            )
            .map_err(|error| AppError::Invalid(format!("读取纪要条目失败：{error}")))?;
        let rows = statement
            .query_map(params![record_id], |row| {
                Ok(MinutesItemRow {
                    id: row.get(0)?,
                    minutes_id: row.get(1)?,
                    record_id: row.get(2)?,
                    kind: row.get(3)?,
                    text: row.get(4)?,
                    owner_text: row.get(5)?,
                    evidence_json: row.get(6)?,
                    status: row.get(7)?,
                    updated_at: row.get(8)?,
                })
            })
            .map_err(|error| AppError::Invalid(format!("读取纪要条目失败：{error}")))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| AppError::Invalid(format!("读取纪要条目失败：{error}")))
    }
}
