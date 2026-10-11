use super::{DecisionReferenceRow, LibraryRepository};
use crate::error::{AppError, AppResult};
use crate::types::KnowledgeIndexStatus;
use chrono::Utc;
use rusqlite::{params, OptionalExtension, Row};

impl LibraryRepository {
    pub fn replace_knowledge_chunks(
        &self,
        record_id: &str,
        chunks: &[crate::types::KnowledgeChunkInput],
    ) -> AppResult<()> {
        let mut connection = self.connect()?;
        let transaction = connection.transaction()?;
        transaction.execute(
            "DELETE FROM knowledge_chunks WHERE record_id = ?1",
            params![record_id],
        )?;
        let now = Utc::now().to_rfc3339();
        for chunk in chunks {
            let segment_ids_json = serde_json::to_string(&chunk.segment_ids)
                .map_err(|error| AppError::Invalid(format!("知识片段引用无效: {error}")))?;
            let embedding_blob = Self::encode_embedding(&chunk.embedding);
            transaction.execute(
                "INSERT INTO knowledge_chunks (id, record_id, project_id, transcript_version_id, segment_ids_json, body, start_ms, end_ms, speaker_label, content_hash, embedding_model, embedding_dimensions, embedding_blob, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
                params![chunk.id, chunk.record_id, chunk.project_id, chunk.transcript_version_id, segment_ids_json, chunk.body, chunk.start_ms, chunk.end_ms, chunk.speaker_label, chunk.content_hash, chunk.embedding_model, chunk.embedding.len() as i64, embedding_blob, now],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn list_knowledge_chunks(
        &self,
        project_id: Option<&str>,
        unfiled_only: bool,
        embedding_model: &str,
    ) -> AppResult<Vec<crate::types::KnowledgeChunkRecord>> {
        let connection = self.connect()?;
        let mut statement = connection.prepare(
            "SELECT chunks.id, chunks.record_id, records.title, chunks.project_id, chunks.segment_ids_json, chunks.body, chunks.start_ms, chunks.end_ms, chunks.embedding_model, chunks.embedding_dimensions, chunks.embedding_blob \
             FROM knowledge_chunks AS chunks JOIN records ON records.id = chunks.record_id \
             WHERE records.archived_at IS NULL AND chunks.embedding_model = ?1 AND (?2 IS NULL OR chunks.project_id = ?2) AND (?3 = 0 OR chunks.project_id IS NULL)",
        )?;
        let rows = statement
            .query_map(
                params![embedding_model, project_id, i64::from(unfiled_only)],
                |row| {
                    let segment_ids_json: String = row.get(4)?;
                    let dimensions: i64 = row.get(9)?;
                    let blob: Vec<u8> = row.get(10)?;
                    let segment_ids = serde_json::from_str(&segment_ids_json).map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(
                            4,
                            rusqlite::types::Type::Text,
                            Box::new(error),
                        )
                    })?;
                    let embedding =
                        Self::decode_embedding(&blob, dimensions as usize).map_err(|message| {
                            rusqlite::Error::FromSqlConversionFailure(
                                10,
                                rusqlite::types::Type::Blob,
                                Box::new(std::io::Error::new(
                                    std::io::ErrorKind::InvalidData,
                                    message,
                                )),
                            )
                        })?;
                    Ok(crate::types::KnowledgeChunkRecord {
                        id: row.get(0)?,
                        record_id: row.get(1)?,
                        record_title: row.get(2)?,
                        project_id: row.get(3)?,
                        segment_ids,
                        body: row.get(5)?,
                        start_ms: row.get(6)?,
                        end_ms: row.get(7)?,
                        embedding_model: row.get(8)?,
                        embedding,
                    })
                },
            )?
            .collect::<Result<Vec<_>, rusqlite::Error>>()?;
        Ok(rows)
    }

    pub fn get_knowledge_index_status(
        &self,
        scope_key: &str,
        embedding_model: &str,
    ) -> AppResult<KnowledgeIndexStatus> {
        if let Some(status) = self
            .connect()?
            .query_row(
                "SELECT scope_key, status, total_records, processed_records, chunk_count, embedding_model, last_error, updated_at FROM knowledge_index_state WHERE scope_key = ?1",
                params![scope_key],
                Self::map_knowledge_index_status,
            )
            .optional()?
        {
            return Ok(status);
        }
        Ok(KnowledgeIndexStatus {
            scope_key: scope_key.to_owned(),
            status: "not_built".to_owned(),
            total_records: 0,
            processed_records: 0,
            chunk_count: 0,
            embedding_model: embedding_model.to_owned(),
            last_error: None,
            updated_at: Utc::now().to_rfc3339(),
        })
    }

    pub fn save_knowledge_index_status(&self, status: &KnowledgeIndexStatus) -> AppResult<()> {
        self.connect()?.execute(
            "INSERT INTO knowledge_index_state (scope_key, status, total_records, processed_records, chunk_count, embedding_model, last_error, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) ON CONFLICT(scope_key) DO UPDATE SET status = excluded.status, total_records = excluded.total_records, processed_records = excluded.processed_records, chunk_count = excluded.chunk_count, embedding_model = excluded.embedding_model, last_error = excluded.last_error, updated_at = excluded.updated_at",
            params![status.scope_key, status.status, status.total_records, status.processed_records, status.chunk_count, status.embedding_model, status.last_error, status.updated_at],
        )?;
        Ok(())
    }

    /// 将后台索引任务标记为失败。若索引状态行尚不存在，也创建一条可见的失败状态，
    /// 避免后台线程启动失败后前端永久停留在“尚未建立”或处理中。
    pub fn mark_knowledge_index_failed(
        &self,
        scope_key: &str,
        embedding_model: &str,
        error: &str,
    ) -> AppResult<()> {
        let now = Utc::now().to_rfc3339();
        self.connect()?.execute(
            "INSERT INTO knowledge_index_state (scope_key, status, total_records, processed_records, chunk_count, embedding_model, last_error, updated_at) VALUES (?1, 'failed', 0, 0, 0, ?2, ?3, ?4) ON CONFLICT(scope_key) DO UPDATE SET status = 'failed', last_error = excluded.last_error, updated_at = excluded.updated_at",
            params![scope_key, embedding_model, error, now],
        )?;
        Ok(())
    }

    /// 将所有已存在的索引范围标记为失败，用于增量索引后台线程无法完成时的兜底。
    pub fn mark_knowledge_indexes_failed(&self, error: &str) -> AppResult<()> {
        self.connect()?.execute(
            "UPDATE knowledge_index_state SET status = 'failed', last_error = ?1, updated_at = ?2 WHERE status IN ('indexing', 'stale')",
            params![error, Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    pub fn recover_interrupted_knowledge_indexes(&self) -> AppResult<usize> {
        let changed = self.connect()?.execute(
            "UPDATE knowledge_index_state SET status = 'failed', last_error = ?1, updated_at = ?2 WHERE status = 'indexing'",
            params![
                "上次索引因应用退出而中断，请重新建立",
                Utc::now().to_rfc3339()
            ],
        )?;
        Ok(changed)
    }

    pub fn has_knowledge_index(&self, embedding_model: &str) -> AppResult<bool> {
        Ok(self.connect()?.query_row(
            "SELECT EXISTS(SELECT 1 FROM knowledge_index_state WHERE embedding_model = ?1 AND status != 'not_built')",
            params![embedding_model],
            |row| row.get(0),
        )?)
    }

    pub fn refresh_knowledge_index_counts(&self, embedding_model: &str) -> AppResult<()> {
        let mut connection = self.connect()?;
        let scope_keys = {
            let mut statement = connection.prepare(
                "SELECT scope_key FROM knowledge_index_state WHERE embedding_model = ?1",
            )?;
            let rows = statement
                .query_map(params![embedding_model], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, rusqlite::Error>>()?;
            rows
        };
        let transaction = connection.transaction()?;
        for scope_key in scope_keys {
            let (project_id, unfiled_only) = if scope_key == "all" {
                (None, false)
            } else if scope_key == "unfiled" {
                (None, true)
            } else if let Some(id) = scope_key.strip_prefix("project:") {
                (Some(id), false)
            } else {
                continue;
            };
            let (total_records, processed_records, chunk_count): (i64, i64, i64) = transaction
                .query_row(
                    "SELECT COUNT(*), COALESCE(SUM(EXISTS(SELECT 1 FROM knowledge_chunks AS chunks WHERE chunks.record_id = records.id AND chunks.embedding_model = ?3)), 0), COALESCE((SELECT COUNT(*) FROM knowledge_chunks AS chunks JOIN records AS chunk_records ON chunk_records.id = chunks.record_id WHERE chunk_records.archived_at IS NULL AND chunks.embedding_model = ?3 AND (?1 IS NULL OR chunks.project_id = ?1) AND (?2 = 0 OR chunks.project_id IS NULL)), 0) FROM records WHERE records.archived_at IS NULL AND EXISTS(SELECT 1 FROM transcript_versions WHERE transcript_versions.record_id = records.id AND transcript_versions.status = 'completed') AND (?1 IS NULL OR records.project_id = ?1) AND (?2 = 0 OR records.project_id IS NULL)",
                    params![project_id, i64::from(unfiled_only), embedding_model],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )?;
            let status = if processed_records == total_records {
                "completed"
            } else {
                "stale"
            };
            transaction.execute(
                "UPDATE knowledge_index_state SET status = ?2, total_records = ?3, processed_records = ?4, chunk_count = ?5, last_error = NULL, updated_at = ?6 WHERE scope_key = ?1",
                params![scope_key, status, total_records, processed_records, chunk_count, Utc::now().to_rfc3339()],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn knowledge_overview(
        &self,
        project_id: Option<&str>,
        unfiled_only: bool,
    ) -> AppResult<crate::types::KnowledgeOverview> {
        let connection = self.connect()?;
        let (record_count, transcript_count, analyzed_count) = connection.query_row(
            "SELECT COUNT(*), SUM(EXISTS(SELECT 1 FROM transcript_versions WHERE transcript_versions.record_id = records.id AND status = 'completed')), SUM(EXISTS(SELECT 1 FROM analyses WHERE analyses.record_id = records.id AND status = 'completed' AND analyses.id = (SELECT id FROM analyses AS latest WHERE latest.record_id = records.id ORDER BY created_at DESC LIMIT 1))) FROM records WHERE records.archived_at IS NULL AND (?1 IS NULL OR project_id = ?1) AND (?2 = 0 OR project_id IS NULL)",
            params![project_id, i64::from(unfiled_only)],
            |row| Ok((row.get(0)?, row.get::<_, Option<i64>>(1)?.unwrap_or(0), row.get::<_, Option<i64>>(2)?.unwrap_or(0))),
        )?;
        let decisions = self
            .list_decisions(project_id, 8)?
            .into_iter()
            .filter(|value| {
                !unfiled_only
                    || value
                        .get("recordId")
                        .and_then(serde_json::Value::as_str)
                        .and_then(|id| self.get_record(id).ok())
                        .is_some_and(|record| record.project_id.is_none())
            })
            .filter_map(|value| serde_json::from_value::<DecisionReferenceRow>(value).ok())
            .map(|item| crate::types::KnowledgeReference {
                record_id: item.record_id,
                record_title: item.record_title,
                text: item.text,
                quote_text: item.quote_text,
                segment_id: item.citation_segment_ids.first().cloned(),
                start_ms: item.start_ms,
                end_ms: item.end_ms,
            })
            .collect();
        let mut statement = connection.prepare(
            "SELECT items.record_id, records.title, items.title, COALESCE(segments.edited_text, segments.normalized_text, segments.original_text, ''), items.source_segment_id, segments.start_ms, segments.end_ms FROM action_items AS items JOIN records ON records.id = items.record_id LEFT JOIN transcript_segments AS segments ON segments.id = items.source_segment_id WHERE records.archived_at IS NULL AND (?1 IS NULL OR items.project_id = ?1) AND (?2 = 0 OR items.project_id IS NULL) ORDER BY items.rowid DESC LIMIT 8",
        )?;
        let action_items = statement
            .query_map(params![project_id, i64::from(unfiled_only)], |row| {
                Ok(crate::types::KnowledgeReference {
                    record_id: row.get(0)?,
                    record_title: row.get(1)?,
                    text: row.get(2)?,
                    quote_text: row.get(3)?,
                    segment_id: row.get(4)?,
                    start_ms: row.get(5)?,
                    end_ms: row.get(6)?,
                })
            })?
            .collect::<Result<Vec<_>, rusqlite::Error>>()?;
        Ok(crate::types::KnowledgeOverview {
            record_count,
            transcript_count,
            analyzed_count,
            decisions,
            action_items,
        })
    }

    fn encode_embedding(values: &[f32]) -> Vec<u8> {
        values
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect()
    }

    fn decode_embedding(bytes: &[u8], dimensions: usize) -> Result<Vec<f32>, String> {
        if bytes.len() != dimensions * 4 {
            return Err("向量维度与存储长度不一致".to_owned());
        }
        Ok(bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|chunk| f32::from_le_bytes(*chunk))
            .collect())
    }

    fn map_knowledge_index_status(row: &Row<'_>) -> rusqlite::Result<KnowledgeIndexStatus> {
        Ok(KnowledgeIndexStatus {
            scope_key: row.get(0)?,
            status: row.get(1)?,
            total_records: row.get(2)?,
            processed_records: row.get(3)?,
            chunk_count: row.get(4)?,
            embedding_model: row.get(5)?,
            last_error: row.get(6)?,
            updated_at: row.get(7)?,
        })
    }
}

/* ------------------------- v0.8：确认记忆的检索索引 ------------------------- */

impl LibraryRepository {
    /// 整体替换记忆块（含 FTS 同步）。记忆块以「最新快照全量重投影」方式维护，
    /// 每次重建都替换全部行，旧快照的残留块随事务一并清除。
    pub fn replace_all_memory_chunks(
        &self,
        chunks: &[crate::types::MemoryChunkInput],
    ) -> AppResult<()> {
        let mut connection = self.connect()?;
        let transaction = connection.transaction()?;
        transaction.execute(
            "DELETE FROM memory_chunks_search WHERE chunk_id IN (SELECT id FROM memory_chunks)",
            [],
        )?;
        transaction.execute("DELETE FROM memory_chunks", [])?;
        let now = Utc::now().to_rfc3339();
        for chunk in chunks {
            let sources_json = serde_json::to_string(&chunk.sources)
                .map_err(|error| AppError::Invalid(format!("记忆来源序列化失败: {error}")))?;
            let embedding_blob = Self::encode_embedding(&chunk.embedding);
            transaction.execute(
                "INSERT INTO memory_chunks (id, snapshot_id, view_kind, item_id, item_type, topic, before_text, after_text, body, occurred_at, sources_json, content_hash, embedding_model, embedding_dimensions, embedding_blob, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
                params![
                    chunk.id,
                    chunk.snapshot_id,
                    chunk.view_kind,
                    chunk.item_id,
                    chunk.item_type,
                    chunk.topic,
                    chunk.before_text,
                    chunk.after_text,
                    chunk.body,
                    chunk.occurred_at,
                    sources_json,
                    chunk.content_hash,
                    chunk.embedding_model,
                    chunk.embedding.len() as i64,
                    embedding_blob,
                    now,
                ],
            )?;
            transaction.execute(
                "INSERT INTO memory_chunks_search (chunk_id, body) VALUES (?1, ?2)",
                params![chunk.id, chunk.body],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn list_memory_chunks(
        &self,
        embedding_model: &str,
    ) -> AppResult<Vec<crate::types::MemoryChunkRecord>> {
        let connection = self.connect()?;
        let mut statement = connection.prepare(
            "SELECT id, snapshot_id, item_id, item_type, topic, before_text, after_text, body, occurred_at, sources_json, embedding_model, embedding_dimensions, embedding_blob FROM memory_chunks WHERE embedding_model = ?1",
        )?;
        let rows = statement
            .query_map(params![embedding_model], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, String>(9)?,
                    row.get::<_, String>(10)?,
                    row.get::<_, i64>(11)?,
                    row.get::<_, Vec<u8>>(12)?,
                ))
            })?
            .collect::<Result<Vec<_>, rusqlite::Error>>()?;
        let mut chunks = Vec::with_capacity(rows.len());
        for (
            id,
            snapshot_id,
            item_id,
            item_type,
            topic,
            before_text,
            after_text,
            body,
            occurred_at,
            sources_json,
            embedding_model,
            dimensions,
            blob,
        ) in rows
        {
            let sources = serde_json::from_str(&sources_json).unwrap_or_default();
            let embedding = Self::decode_embedding(&blob, dimensions as usize).map_err(|message| {
                rusqlite::Error::FromSqlConversionFailure(
                    12,
                    rusqlite::types::Type::Blob,
                    Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, message)),
                )
            })?;
            chunks.push(crate::types::MemoryChunkRecord {
                id,
                snapshot_id,
                item_id,
                item_type,
                topic,
                before_text,
                after_text,
                body,
                occurred_at,
                sources,
                embedding_model,
                embedding,
            });
        }
        Ok(chunks)
    }

    /// 记忆块关键词召回：与 record_search 同款 trigram FTS，短查询退回 LIKE。
    pub fn search_memory_chunks(&self, query: &str, limit: usize) -> AppResult<Vec<String>> {
        let query = query.trim();
        if query.is_empty() {
            return Ok(Vec::new());
        }
        let connection = self.connect()?;
        let fts_query = Self::build_fts_query(query);
        let use_fts = query.chars().count() >= 3 && fts_query.is_some();
        let sql = if use_fts {
            "SELECT chunk_id FROM memory_chunks_search WHERE memory_chunks_search MATCH ?1 ORDER BY rank LIMIT ?2"
        } else {
            "SELECT chunk_id FROM memory_chunks_search WHERE body LIKE '%' || ?1 || '%' LIMIT ?2"
        };
        let mut statement = connection.prepare(sql)?;
        let query_param = fts_query.as_deref().filter(|_| use_fts).unwrap_or(query);
        let rows = statement
            .query_map(params![query_param, limit as i64], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, rusqlite::Error>>()?;
        Ok(rows)
    }

    /// 每个 (view_kind, scope_key, range) 只取最新 completed 版本的快照，
    /// 记忆块投影只认这些版本。返回 (快照 ID, view_kind, result_json)。
    pub fn latest_completed_memory_snapshots(&self) -> AppResult<Vec<(String, String, String)>> {
        let connection = self.connect()?;
        let mut statement = connection.prepare(
            "SELECT outer_snapshot.id, outer_snapshot.view_kind, outer_snapshot.result_json FROM memory_snapshots AS outer_snapshot WHERE outer_snapshot.status = 'completed' AND outer_snapshot.version = ( \
                SELECT MAX(version) FROM memory_snapshots AS inner_snapshot \
                WHERE inner_snapshot.view_kind = outer_snapshot.view_kind \
                  AND inner_snapshot.scope_key = outer_snapshot.scope_key \
                  AND inner_snapshot.range_start IS outer_snapshot.range_start \
                  AND inner_snapshot.range_end IS outer_snapshot.range_end \
                  AND inner_snapshot.status = 'completed' \
             ) ORDER BY outer_snapshot.updated_at DESC",
        )?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?
            .collect::<Result<Vec<_>, rusqlite::Error>>()?;
        Ok(rows)
    }

    /// 同一条目（稳定内容 ID）跨快照版本的最近一次用户裁决：后写覆盖先写。
    pub fn latest_memory_feedback_decisions(
        &self,
    ) -> AppResult<std::collections::HashMap<String, String>> {
        let connection = self.connect()?;
        let mut statement = connection.prepare(
            "SELECT feedback.item_id, feedback.decision FROM memory_feedback AS feedback \
             JOIN memory_snapshots AS snapshots ON snapshots.id = feedback.snapshot_id \
             ORDER BY feedback.updated_at ASC",
        )?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<Result<Vec<_>, rusqlite::Error>>()?;
        Ok(rows.into_iter().collect())
    }
}
