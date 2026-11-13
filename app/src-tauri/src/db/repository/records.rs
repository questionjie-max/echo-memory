use super::LibraryRepository;
use crate::db::record_ops;
use crate::error::{AppError, AppResult};
use crate::transcript::{normalize_chinese, NORMALIZATION_VERSION};
use crate::types::{ProcessingJob, RecordBrief, TranscriptSegmentInput};
use chrono::Utc;
use rusqlite::{params, OptionalExtension, Row};
use std::path::{Path, PathBuf};
use uuid::Uuid;

const RECORD_SELECT: &str = "SELECT records.id, records.title, records.project_id, projects.name, \
    records.audio_hash, records.audio_duration_ms, records.imported_at, records.processing_status, \
    EXISTS(SELECT 1 FROM transcript_versions WHERE transcript_versions.record_id = records.id AND transcript_versions.status = 'completed'), \
    (SELECT status FROM analyses WHERE analyses.record_id = records.id ORDER BY created_at DESC LIMIT 1), \
    (SELECT last_error FROM processing_jobs WHERE processing_jobs.record_id = records.id AND job_type = 'analyze' AND status = 'failed' ORDER BY updated_at DESC LIMIT 1), \
    records.analysis_template_id, \
    (SELECT stage FROM processing_jobs WHERE processing_jobs.record_id = records.id ORDER BY updated_at DESC LIMIT 1), \
    COALESCE((SELECT progress_current FROM processing_jobs WHERE processing_jobs.record_id = records.id ORDER BY updated_at DESC LIMIT 1), 0), \
    COALESCE((SELECT progress_total FROM processing_jobs WHERE processing_jobs.record_id = records.id ORDER BY updated_at DESC LIMIT 1), 0), \
    records.source_type, records.archived_at, \
    COALESCE(( \
        SELECT json_extract(content_json, '$.quality_warning') IS NOT NULL \
        FROM analyses WHERE analyses.record_id = records.id \
        ORDER BY created_at DESC LIMIT 1 \
    ), 0), \
    records.import_confirmed_at \
    FROM records LEFT JOIN projects ON projects.id = records.project_id";

impl LibraryRepository {
    pub fn create_record(
        &self,
        title: &str,
        project_id: Option<&str>,
        audio_path: &Path,
        audio_hash: &str,
        audio_duration_ms: i64,
    ) -> AppResult<RecordBrief> {
        let title = title.trim();
        if title.is_empty() {
            return Err(AppError::Invalid("记录标题不能为空".into()));
        }
        if audio_hash.trim().is_empty() {
            return Err(AppError::Invalid("audio_hash 不能为空".into()));
        }
        let now = Utc::now().to_rfc3339();
        let record = RecordBrief {
            id: Uuid::new_v4().to_string(),
            title: title.to_owned(),
            source_type: "import".to_owned(),
            project_id: project_id.map(str::to_owned),
            project_name: None,
            audio_hash: audio_hash.to_owned(),
            audio_duration_ms,
            imported_at: now.clone(),
            status: "queued".to_owned(),
            has_transcript: false,
            has_analysis: false,
            analysis_status: None,
            analysis_has_quality_warning: false,
            last_analysis_error: None,
            analysis_template_id: Some("builtin-standard".to_owned()),
            processing_stage: None,
            progress_current: 0,
            progress_total: 0,
            archived_at: None,
            import_confirmed_at: None,
        };
        self.connect()?.execute(
            "INSERT INTO records \
             (id, title, project_id, source_type, audio_path, audio_hash, audio_duration_ms, imported_at, processing_status, created_at, updated_at) \
             VALUES (?1, ?2, NULLIF(?3, ''), 'import', ?4, ?5, ?6, ?7, ?8, ?7, ?7)",
            params![
                record.id,
                record.title,
                project_id,
                audio_path.to_string_lossy(),
                record.audio_hash,
                record.audio_duration_ms,
                record.imported_at,
                record.status
            ],
        )?;
        self.get_record(&record.id)
    }

    /// 原子地写入外部导入记录和首个转写任务。
    /// 文件复制由上层服务负责；若写库失败，调用方据此删除刚复制的文件，
    /// 避免无法解释的孤儿音频。
    pub fn create_record_with_transcription_job(
        &self,
        record_id: &str,
        title: &str,
        project_id: Option<&str>,
        audio_path: &Path,
        audio_hash: &str,
        audio_duration_ms: i64,
    ) -> AppResult<(RecordBrief, ProcessingJob)> {
        self.create_record_with_transcription_job_source(
            record_id,
            title,
            project_id,
            audio_path,
            audio_hash,
            audio_duration_ms,
            "import",
        )
    }

    /// 与外部文件导入共用落库事务，但保留应用内录音的来源语义。
    #[allow(clippy::too_many_arguments)]
    pub fn create_record_with_transcription_job_source(
        &self,
        record_id: &str,
        title: &str,
        project_id: Option<&str>,
        audio_path: &Path,
        audio_hash: &str,
        audio_duration_ms: i64,
        source_type: &str,
    ) -> AppResult<(RecordBrief, ProcessingJob)> {
        if !matches!(source_type, "import" | "recording") {
            return Err(AppError::Invalid(format!("非法音频来源: {source_type}")));
        }
        let title = title.trim();
        if title.is_empty() {
            return Err(AppError::Invalid("记录标题不能为空".into()));
        }
        if audio_hash.trim().is_empty() {
            return Err(AppError::Invalid("audio_hash 不能为空".into()));
        }

        let now = Utc::now().to_rfc3339();
        let record = RecordBrief {
            id: record_id.to_owned(),
            title: title.to_owned(),
            source_type: source_type.to_owned(),
            project_id: project_id.map(str::to_owned),
            project_name: None,
            audio_hash: audio_hash.to_owned(),
            audio_duration_ms,
            imported_at: now.clone(),
            status: "queued".to_owned(),
            has_transcript: false,
            has_analysis: false,
            analysis_status: None,
            analysis_has_quality_warning: false,
            last_analysis_error: None,
            analysis_template_id: Some("builtin-standard".to_owned()),
            processing_stage: None,
            progress_current: 0,
            progress_total: 0,
            archived_at: None,
            import_confirmed_at: None,
        };
        let job = ProcessingJob {
            id: Uuid::new_v4().to_string(),
            record_id: record.id.clone(),
            job_type: "transcribe".to_owned(),
            status: "queued".to_owned(),
            attempt_count: 0,
            last_error: None,
            stage: None,
            progress_current: 0,
            progress_total: 0,
            created_at: now.clone(),
            updated_at: now,
        };
        let mut connection = self.connect()?;
        let transaction = connection.transaction()?;
        transaction.execute(
            "INSERT INTO records \
             (id, title, project_id, source_type, audio_path, audio_hash, audio_duration_ms, imported_at, processing_status, created_at, updated_at) \
             VALUES (?1, ?2, NULLIF(?3, ''), ?4, ?5, ?6, ?7, ?8, ?9, ?8, ?8)",
            params![
                record.id,
                record.title,
                project_id,
                source_type,
                audio_path.to_string_lossy(),
                record.audio_hash,
                record.audio_duration_ms,
                record.imported_at,
                record.status
            ],
        )?;
        transaction.execute(
            "INSERT INTO processing_jobs \
             (id, record_id, job_type, status, attempt_count, last_error, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                job.id,
                job.record_id,
                job.job_type,
                job.status,
                job.attempt_count,
                job.last_error,
                job.created_at,
                job.updated_at
            ],
        )?;
        transaction.commit()?;
        Ok((self.get_record(record_id)?, job))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_document_record(
        &self,
        record_id: &str,
        title: &str,
        project_id: Option<&str>,
        document_path: &Path,
        document_hash: &str,
        source_format: &str,
        segments: &[TranscriptSegmentInput],
    ) -> AppResult<RecordBrief> {
        self.create_text_record(
            record_id,
            title,
            project_id,
            document_path,
            document_hash,
            "document",
            "document-import",
            "document-import-v1",
            source_format,
            None,
            segments,
        )
    }

    /// AI 对话导入的落库入口：与文档导入同构，仅 source_type / 管线标识不同，
    /// 便于按来源查重（find_record_by_source_hash）和前端区分展示。
    /// occurred_at 是对话的真实时间（导出元数据或文件 mtime）；None 时用现在。
    #[allow(clippy::too_many_arguments)]
    pub fn create_chat_record(
        &self,
        record_id: &str,
        title: &str,
        project_id: Option<&str>,
        document_path: &Path,
        document_hash: &str,
        source_format: &str,
        occurred_at: Option<&str>,
        segments: &[TranscriptSegmentInput],
    ) -> AppResult<RecordBrief> {
        self.create_text_record(
            record_id,
            title,
            project_id,
            document_path,
            document_hash,
            "chat",
            "chat-import",
            "chat-import-v1",
            source_format,
            occurred_at,
            segments,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn create_text_record(
        &self,
        record_id: &str,
        title: &str,
        project_id: Option<&str>,
        document_path: &Path,
        document_hash: &str,
        source_type: &str,
        provider: &str,
        pipeline_version: &str,
        source_format: &str,
        occurred_at: Option<&str>,
        segments: &[TranscriptSegmentInput],
    ) -> AppResult<RecordBrief> {
        let title = title.trim();
        if title.is_empty() {
            return Err(AppError::Invalid("记录标题不能为空".into()));
        }
        if document_hash.trim().is_empty() {
            return Err(AppError::Invalid("document_hash 不能为空".into()));
        }
        if segments.is_empty() {
            return Err(AppError::Invalid("文档内容不能为空".into()));
        }
        if let Some(project_id) = project_id {
            self.get_project(project_id)?;
        }
        for segment in segments {
            if segment.end_ms < segment.start_ms || segment.original_text.trim().is_empty() {
                return Err(AppError::Invalid("文档片段无效".into()));
            }
        }

        let now = Utc::now().to_rfc3339();
        // 记录的展示时间用对话/文档的真实时间；导入时刻只作为 updated_at 兜底。
        let imported_at = occurred_at
            .filter(|stamp| !stamp.trim().is_empty())
            .unwrap_or(now.as_str());
        let transcript_version_id = Uuid::new_v4().to_string();
        let preprocessing_json = serde_json::json!({ "sourceFormat": source_format }).to_string();
        let mut connection = self.connect()?;
        let transaction = connection.transaction()?;
        transaction.execute(
            "INSERT INTO records              (id, title, project_id, source_type, audio_path, audio_hash, audio_duration_ms, imported_at, processing_status, created_at, updated_at)              VALUES (?1, ?2, NULLIF(?3, ''), ?4, ?5, ?6, 0, ?7, 'completed', ?8, ?8)",
            params![
                record_id,
                title,
                project_id,
                source_type,
                document_path.to_string_lossy(),
                document_hash,
                imported_at,
                now,
            ],
        )?;
        transaction.execute(
            "INSERT INTO transcript_versions              (id, record_id, provider, model, status, language, pipeline_version, preprocessing_json, created_at)              VALUES (?1, ?2, ?3, ?4, 'completed', 'und', ?5, ?6, ?7)",
            params![
                transcript_version_id,
                record_id,
                provider,
                source_format,
                pipeline_version,
                preprocessing_json,
                now,
            ],
        )?;
        for (sequence, segment) in segments.iter().enumerate() {
            let original_text = segment.original_text.trim();
            transaction.execute(
                "INSERT INTO transcript_segments                  (id, transcript_version_id, record_id, sequence, speaker_label, start_ms, end_ms, original_text, normalized_text, normalization_version, edited_text, created_at, updated_at)                  VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, NULL, ?11, ?11)",
                params![
                    Uuid::new_v4().to_string(),
                    transcript_version_id,
                    record_id,
                    sequence as i64,
                    segment.speaker_label,
                    segment.start_ms,
                    segment.end_ms,
                    original_text,
                    normalize_chinese(original_text),
                    NORMALIZATION_VERSION,
                    now,
                ],
            )?;
        }
        transaction.commit()?;
        self.get_record(record_id)
    }

    /// 列出记录，可按项目过滤（None 表示全部）。
    pub fn list_records(
        &self,
        project_id: Option<&str>,
        unfiled_only: bool,
    ) -> AppResult<Vec<RecordBrief>> {
        self.list_record_scope(project_id, unfiled_only, false)
    }

    pub fn list_archived_records(
        &self,
        project_id: Option<&str>,
        unfiled_only: bool,
    ) -> AppResult<Vec<RecordBrief>> {
        self.list_record_scope(project_id, unfiled_only, true)
    }

    fn list_record_scope(
        &self,
        project_id: Option<&str>,
        unfiled_only: bool,
        archived: bool,
    ) -> AppResult<Vec<RecordBrief>> {
        let connection = self.connect()?;
        let archived_condition = if archived {
            "records.archived_at IS NOT NULL"
        } else {
            "records.archived_at IS NULL"
        };
        let rows = match (project_id, unfiled_only) {
            (Some(pid), _) => {
                let mut stmt = connection.prepare(&format!(
                    "{RECORD_SELECT} WHERE {archived_condition} AND records.project_id = ?1 ORDER BY records.updated_at DESC"
                ))?;
                let records = stmt
                    .query_map(params![pid], Self::map_record)?
                    .collect::<Result<Vec<_>, rusqlite::Error>>()?;
                records
            }
            (None, true) => {
                let mut stmt = connection.prepare(&format!(
                    "{RECORD_SELECT} WHERE {archived_condition} AND records.project_id IS NULL ORDER BY records.updated_at DESC"
                ))?;
                let records = stmt
                    .query_map([], Self::map_record)?
                    .collect::<Result<Vec<_>, rusqlite::Error>>()?;
                records
            }
            (None, false) => {
                let mut stmt = connection.prepare(&format!(
                    "{RECORD_SELECT} WHERE {archived_condition} ORDER BY records.updated_at DESC"
                ))?;
                let records = stmt
                    .query_map([], Self::map_record)?
                    .collect::<Result<Vec<_>, rusqlite::Error>>()?;
                records
            }
        };
        Ok(rows)
    }

    pub fn get_record(&self, id: &str) -> AppResult<RecordBrief> {
        self.connect()?
            .query_row(
                &format!("{RECORD_SELECT} WHERE records.id = ?1"),
                params![id],
                Self::map_record,
            )
            .optional()?
            .ok_or_else(|| AppError::NotFound(format!("record {id}")))
    }

    pub fn find_record_by_hash(&self, audio_hash: &str) -> AppResult<Option<RecordBrief>> {
        self.connect()?
            .query_row(
                &format!("{RECORD_SELECT} WHERE records.audio_hash = ?1"),
                params![audio_hash],
                Self::map_record,
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn find_record_by_source_hash(
        &self,
        source_type: &str,
        source_hash: &str,
        project_id: Option<&str>,
    ) -> AppResult<Option<RecordBrief>> {
        self.connect()?
            .query_row(
                &format!(
                    "{RECORD_SELECT} WHERE records.source_type = ?1 AND records.audio_hash = ?2 \
                     AND ((?3 IS NULL AND records.project_id IS NULL) OR records.project_id = ?3) \
                     ORDER BY records.imported_at ASC LIMIT 1"
                ),
                params![source_type, source_hash, project_id],
                Self::map_record,
            )
            .optional()
            .map_err(Into::into)
    }

    /// 更新记录的处理状态（processing_status）。
    pub fn update_record_status(&self, id: &str, status: &str) -> AppResult<RecordBrief> {
        Self::validate_processing_status(status)?;
        let now = Utc::now().to_rfc3339();
        let affected = self.connect()?.execute(
            "UPDATE records SET processing_status = ?2, updated_at = ?3 WHERE id = ?1",
            params![id, status, now],
        )?;
        if affected == 0 {
            return Err(AppError::NotFound(format!("record {id}")));
        }
        self.get_record(id)
    }

    pub fn set_record_archived(&self, id: &str, archived: bool) -> AppResult<RecordBrief> {
        let now = Utc::now().to_rfc3339();
        let archived_at: Option<&str> = archived.then_some(now.as_str());
        let affected = self.connect()?.execute(
            "UPDATE records SET archived_at = ?2, updated_at = ?3 WHERE id = ?1",
            params![id, archived_at, now],
        )?;
        if affected == 0 {
            return Err(AppError::NotFound(format!("record {id}")));
        }
        self.get_record(id)
    }

    /// 块4：导入内容确认门——document/chat 记录的「确认入记忆 / 撤回」。
    /// 与纪要条目同语义：随时双向可逆；确认才对 MCP 可见。
    /// 录音类记录没有确认语义（原文层 v1.0 起对 MCP 可读），调用此方法于事无补但也无害。
    pub fn set_import_confirmed(&self, id: &str, confirmed: bool) -> AppResult<RecordBrief> {
        let now = Utc::now().to_rfc3339();
        let confirmed_at: Option<&str> = confirmed.then_some(now.as_str());
        let affected = self.connect()?.execute(
            "UPDATE records SET import_confirmed_at = ?2, updated_at = ?3 WHERE id = ?1",
            params![id, confirmed_at, now],
        )?;
        if affected == 0 {
            return Err(AppError::NotFound(format!("record {id}")));
        }
        self.get_record(id)
    }

    /// 块4：该记录是否为「未确认的导入内容」（document/chat 且未确认）。
    /// MCP 原文层据此拦截；app 自身视图不受影响（用户能看到自己的暂定内容）。
    pub fn is_unconfirmed_import(&self, id: &str) -> AppResult<bool> {
        let count: i64 = self.connect()?.query_row(
            "SELECT COUNT(*) FROM records WHERE id = ?1 \
             AND source_type IN ('document', 'chat') AND import_confirmed_at IS NULL",
            params![id],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// 块4：全部未确认导入记录的 id（MCP 侧过滤 search/list 用）。
    pub fn list_unconfirmed_import_ids(&self) -> AppResult<Vec<String>> {
        let connection = self.connect()?;
        let mut statement = connection.prepare(
            "SELECT id FROM records \
             WHERE source_type IN ('document', 'chat') AND import_confirmed_at IS NULL",
        )?;
        let ids = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, rusqlite::Error>>()?;
        Ok(ids)
    }

    pub fn update_record_project(
        &self,
        id: &str,
        project_id: Option<&str>,
    ) -> AppResult<RecordBrief> {
        if let Some(project_id) = project_id {
            self.get_project(project_id)?;
        }
        let mut connection = self.connect()?;
        let transaction = connection.transaction()?;
        let affected = transaction.execute(
            "UPDATE records SET project_id = ?2, updated_at = ?3 WHERE id = ?1",
            params![id, project_id, Utc::now().to_rfc3339()],
        )?;
        if affected == 0 {
            return Err(AppError::NotFound(format!("record {id}")));
        }
        transaction.execute(
            "UPDATE record_search SET project_id = ?2 WHERE record_id = ?1",
            params![id, project_id],
        )?;
        transaction.execute(
            "UPDATE action_items SET project_id = ?2 WHERE record_id = ?1",
            params![id, project_id],
        )?;
        transaction.execute(
            "UPDATE knowledge_chunks SET project_id = ?2 WHERE record_id = ?1",
            params![id, project_id],
        )?;
        transaction.commit()?;
        self.get_record(id)
    }

    pub fn move_records(&self, record_ids: &[String], project_id: Option<&str>) -> AppResult<u32> {
        let mut connection = self.connect()?;
        let mut transaction = connection.transaction()?;
        record_ops::move_records(&mut transaction, record_ids, project_id)?;
        transaction.commit()?;
        Ok(record_ids.len() as u32)
    }

    pub fn update_record_title(&self, id: &str, title: &str) -> AppResult<RecordBrief> {
        let title = title.trim();
        if title.is_empty() {
            return Err(AppError::Invalid("录音标题不能为空".to_owned()));
        }
        let affected = self.connect()?.execute(
            "UPDATE records SET title = ?2, updated_at = ?3 WHERE id = ?1",
            params![id, title, Utc::now().to_rfc3339()],
        )?;
        if affected == 0 {
            return Err(AppError::NotFound(format!("record {id}")));
        }
        self.get_record(id)
    }

    pub fn set_record_analysis_template(
        &self,
        id: &str,
        template_id: &str,
    ) -> AppResult<RecordBrief> {
        self.get_analysis_template(template_id)?;
        let affected = self.connect()?.execute(
            "UPDATE records SET analysis_template_id = ?2, updated_at = ?3 WHERE id = ?1",
            params![id, template_id, Utc::now().to_rfc3339()],
        )?;
        if affected == 0 {
            return Err(AppError::NotFound(format!("record {id}")));
        }
        self.get_record(id)
    }

    pub fn delete_record(&self, id: &str) -> AppResult<()> {
        let affected = self
            .connect()?
            .execute("DELETE FROM records WHERE id = ?1", params![id])?;
        if affected == 0 {
            return Err(AppError::NotFound(format!("record {id}")));
        }
        Ok(())
    }

    pub fn delete_records(&self, record_ids: &[String]) -> AppResult<Vec<(String, PathBuf)>> {
        let mut connection = self.connect()?;
        let mut transaction = connection.transaction()?;
        let deleted = record_ops::delete_records(&mut transaction, record_ids)?;
        transaction.commit()?;
        Ok(deleted)
    }

    pub fn cleanup_deleted_record_files(
        &self,
        library_root: &Path,
        deleted_records: &[(String, PathBuf)],
    ) -> Vec<PathBuf> {
        record_ops::cleanup_deleted_record_files(library_root, deleted_records)
    }

    /// 删除记录在磁盘上的产物：受管目录里的原始音频，以及 raw/<记录 id>/ 下的
    /// 预处理音频与转写中间文件。库内行由 delete_record 删除（外键级联会带走
    /// 转写、分析、待办、知识分片和搜索项）。
    ///
    /// 返回没能删掉的路径：文件清理失败不阻断删除——库里已经没有这条记录了，
    /// 留下孤儿文件比让整批删除失败更可接受，但要把痕迹报告给用户。
    pub fn remove_record_files(&self, library_root: &Path, id: &str) -> Vec<PathBuf> {
        let mut failed = Vec::new();
        if let Ok(relative) = self.audio_path_for_record(id) {
            let audio = library_root.join(&relative);
            match std::fs::remove_file(&audio) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => failed.push(audio),
            }
        }
        let raw = library_root.join("raw").join(id);
        if raw.exists() && std::fs::remove_dir_all(&raw).is_err() {
            failed.push(raw);
        }
        failed
    }

    fn map_record(row: &Row<'_>) -> rusqlite::Result<RecordBrief> {
        let analysis_status: Option<String> = row.get(9)?;
        Ok(RecordBrief {
            id: row.get(0)?,
            title: row.get(1)?,
            project_id: row.get(2)?,
            project_name: row.get(3)?,
            audio_hash: row.get(4)?,
            audio_duration_ms: row.get(5)?,
            imported_at: row.get(6)?,
            status: row.get(7)?,
            has_transcript: row.get::<_, i64>(8)? != 0,
            has_analysis: analysis_status.as_deref() == Some("completed"),
            analysis_status,
            analysis_has_quality_warning: row.get::<_, i64>(17)? != 0,
            last_analysis_error: row.get(10)?,
            analysis_template_id: row.get(11)?,
            processing_stage: row.get(12)?,
            progress_current: row.get(13)?,
            progress_total: row.get(14)?,
            source_type: row.get(15)?,
            archived_at: row.get(16)?,
            import_confirmed_at: row.get(18)?,
        })
    }

    /// v1.4 包5：保存录音打点标记（相对录音开始的毫秒升序列表）。
    /// marks 为空时清除该记录的标记（删除记录的级联由 records 表删除覆盖）。
    pub fn save_record_marks(&self, record_id: &str, marks: &[i64]) -> AppResult<()> {
        let mut sorted = marks.to_vec();
        sorted.sort_unstable();
        let json = if sorted.is_empty() {
            None
        } else {
            Some(serde_json::to_string(&sorted).map_err(|error| {
                AppError::Invalid(format!("录音标记序列化失败：{error}"))
            })?)
        };
        self.connect()?.execute(
            "UPDATE records SET marks_json = ?2 WHERE id = ?1",
            params![record_id, json],
        )?;
        Ok(())
    }

    /// v1.4 包5：读取录音打点标记；从未打过标记（NULL）按空数组处理。
    pub fn get_record_marks(&self, record_id: &str) -> AppResult<Vec<i64>> {
        let json: Option<String> = self
            .connect()?
            .query_row(
                "SELECT marks_json FROM records WHERE id = ?1",
                params![record_id],
                |row| row.get(0),
            )
            .optional()?
            .flatten();
        let marks: Vec<i64> = json
            .and_then(|value| serde_json::from_str(&value).ok())
            .unwrap_or_default();
        Ok(marks)
    }

    /// v1.4 包8：保存一条记录产物（口播稿/show notes/社媒文案）。
    /// 同一 kind 已存在则覆盖（重新生成不堆积历史）。
    pub fn save_record_artifact(
        &self,
        record_id: &str,
        kind: &str,
        content: &str,
        provider: &str,
        model: &str,
    ) -> AppResult<crate::types::RecordArtifact> {
        let now = chrono::Utc::now().to_rfc3339();
        let artifact = crate::types::RecordArtifact {
            id: Uuid::new_v4().to_string(),
            record_id: record_id.to_owned(),
            kind: kind.to_owned(),
            title: crate::types::artifact_kind_label(kind).to_owned(),
            content: content.to_owned(),
            provider: provider.to_owned(),
            model: model.to_owned(),
            created_at: now.clone(),
        };
        self.connect()?.execute(
            "INSERT INTO record_artifacts (id, record_id, kind, title, content, provider, model, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) \
             ON CONFLICT(record_id, kind) DO UPDATE SET id = excluded.id, title = excluded.title, \
               content = excluded.content, provider = excluded.provider, model = excluded.model, \
               created_at = excluded.created_at",
            params![
                artifact.id,
                artifact.record_id,
                artifact.kind,
                artifact.title,
                artifact.content,
                artifact.provider,
                artifact.model,
                artifact.created_at,
            ],
        )?;
        Ok(artifact)
    }

    /// v1.4 包8：列出某条记录的产物（按生成时间倒序）。
    pub fn list_record_artifacts(&self, record_id: &str) -> AppResult<Vec<crate::types::RecordArtifact>> {
        let connection = self.connect()?;
        let mut statement = connection.prepare(
            "SELECT id, record_id, kind, title, content, provider, model, created_at \
             FROM record_artifacts WHERE record_id = ?1 ORDER BY created_at DESC",
        )?;
        let rows = statement.query_map(params![record_id], |row| {
            Ok(crate::types::RecordArtifact {
                id: row.get(0)?,
                record_id: row.get(1)?,
                kind: row.get(2)?,
                title: row.get(3)?,
                content: row.get(4)?,
                provider: row.get(5)?,
                model: row.get(6)?,
                created_at: row.get(7)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn temp_repository() -> (LibraryRepository, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!("record-marks-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        (
            LibraryRepository::new(root.join("library.db")).unwrap(),
            root,
        )
    }

    fn seeded_record(repository: &LibraryRepository, root: &std::path::Path, title: &str) -> crate::types::RecordBrief {
        let dir = root.join("audio");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("sample.wav");
        std::fs::write(&path, [0_u8; 1024]).unwrap();
        let hash = format!("{:x}", md5_of_title(title));
        repository
            .create_record(title, None, &path, &hash, 60_000)
            .unwrap()
    }

    /// 测试辅助：从标题生成稳定 hash（无 md5 依赖，任何确定性函数都行）。
    fn md5_of_title(title: &str) -> u64 {
        title.bytes().fold(0xcbf29ce484222325_u64, |acc, byte| {
            (acc ^ byte as u64).wrapping_mul(0x100000001b3)
        })
    }

    #[test]
    fn record_marks_round_trip_and_default_empty() {
        let (repository, root) = temp_repository();
        let record = seeded_record(&repository, &root, "带标记的录音");

        // 未打标记：空数组
        assert!(repository.get_record_marks(&record.id).unwrap().is_empty());

        // 乱序写入 → 读取时升序
        repository
            .save_record_marks(&record.id, &[62_000, 15_000, 30_000])
            .unwrap();
        assert_eq!(
            repository.get_record_marks(&record.id).unwrap(),
            vec![15_000, 30_000, 62_000]
        );

        // 覆盖保存
        repository.save_record_marks(&record.id, &[5_000]).unwrap();
        assert_eq!(repository.get_record_marks(&record.id).unwrap(), vec![5_000]);

        // 清空
        repository.save_record_marks(&record.id, &[]).unwrap();
        assert!(repository.get_record_marks(&record.id).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn marks_survive_record_reload_and_vanish_with_record() {
        let (repository, root) = temp_repository();
        let record = seeded_record(&repository, &root, "标记随记录");
        repository
            .save_record_marks(&record.id, &[1_000, 2_000])
            .unwrap();
        assert_eq!(
            repository.get_record_marks(&record.id).unwrap(),
            vec![1_000, 2_000]
        );

        // 记录删除 → 标记一起消失（marks_json 在 records 行内；
        // 读取语义：记录不存在与未打标同样返回空数组）
        repository.delete_record(&record.id).unwrap();
        let marks = repository.get_record_marks(&record.id).unwrap();
        assert!(marks.is_empty(), "记录删除后标记必须清空，实际：{marks:?}");
        let _ = std::fs::remove_dir_all(root);
    }
    #[test]
    fn record_artifacts_round_trip_overwrite_and_list() {
        let (repository, root) = temp_repository();
        let record = seeded_record(&repository, &root, "口播课第一节");

        // 未生成：空
        assert!(repository.list_record_artifacts(&record.id).unwrap().is_empty());

        let artifact = repository
            .save_record_artifact(&record.id, "script", "第一版口播稿", "ollama", "qwen2.5:7b")
            .unwrap();
        assert_eq!(artifact.title, "改写稿");
        let list = repository.list_record_artifacts(&record.id).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].content, "第一版口播稿");

        // 重新生成：覆盖同 kind，不堆积
        repository
            .save_record_artifact(&record.id, "script", "第二版口播稿", "ollama", "qwen2.5:7b")
            .unwrap();
        let list = repository.list_record_artifacts(&record.id).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].content, "第二版口播稿");

        // 不同 kind 并存
        repository
            .save_record_artifact(&record.id, "show_notes", "本期简介", "ollama", "qwen2.5:7b")
            .unwrap();
        assert_eq!(repository.list_record_artifacts(&record.id).unwrap().len(), 2);

        // 记录删除 → 产物级联消失（外键 ON DELETE CASCADE）
        repository.delete_record(&record.id).unwrap();
        assert!(repository.list_record_artifacts(&record.id).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(root);
    }
}
