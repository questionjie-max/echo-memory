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
    records.source_type \
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
            last_analysis_error: None,
            analysis_template_id: Some("builtin-standard".to_owned()),
            processing_stage: None,
            progress_current: 0,
            progress_total: 0,
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

    /// 原子地写入记录和首个转写任务。文件复制由上层服务负责；若写库失败，
    /// 调用方据此删除刚复制的文件，避免无法解释的孤儿音频。
    pub fn create_record_with_transcription_job(
        &self,
        record_id: &str,
        title: &str,
        project_id: Option<&str>,
        audio_path: &Path,
        audio_hash: &str,
        audio_duration_ms: i64,
    ) -> AppResult<(RecordBrief, ProcessingJob)> {
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
            last_analysis_error: None,
            analysis_template_id: Some("builtin-standard".to_owned()),
            processing_stage: None,
            progress_current: 0,
            progress_total: 0,
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
        let transcript_version_id = Uuid::new_v4().to_string();
        let preprocessing_json = serde_json::json!({ "sourceFormat": source_format }).to_string();
        let mut connection = self.connect()?;
        let transaction = connection.transaction()?;
        transaction.execute(
            "INSERT INTO records              (id, title, project_id, source_type, audio_path, audio_hash, audio_duration_ms, imported_at, processing_status, created_at, updated_at)              VALUES (?1, ?2, NULLIF(?3, ''), 'document', ?4, ?5, 0, ?6, 'completed', ?6, ?6)",
            params![
                record_id,
                title,
                project_id,
                document_path.to_string_lossy(),
                document_hash,
                now,
            ],
        )?;
        transaction.execute(
            "INSERT INTO transcript_versions              (id, record_id, provider, model, status, language, pipeline_version, preprocessing_json, created_at)              VALUES (?1, ?2, 'document-import', ?3, 'completed', 'und', 'document-import-v1', ?4, ?5)",
            params![
                transcript_version_id,
                record_id,
                source_format,
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
        let connection = self.connect()?;
        let rows = match (project_id, unfiled_only) {
            (Some(pid), _) => {
                let mut stmt = connection.prepare(&format!(
                    "{RECORD_SELECT} WHERE records.project_id = ?1 ORDER BY records.updated_at DESC"
                ))?;
                let records = stmt
                    .query_map(params![pid], Self::map_record)?
                    .collect::<Result<Vec<_>, rusqlite::Error>>()?;
                records
            }
            (None, true) => {
                let mut stmt = connection.prepare(&format!(
                    "{RECORD_SELECT} WHERE records.project_id IS NULL ORDER BY records.updated_at DESC"
                ))?;
                let records = stmt
                    .query_map([], Self::map_record)?
                    .collect::<Result<Vec<_>, rusqlite::Error>>()?;
                records
            }
            (None, false) => {
                let mut stmt = connection
                    .prepare(&format!("{RECORD_SELECT} ORDER BY records.updated_at DESC"))?;
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
            last_analysis_error: row.get(10)?,
            analysis_template_id: row.get(11)?,
            processing_stage: row.get(12)?,
            progress_current: row.get(13)?,
            progress_total: row.get(14)?,
            source_type: row.get(15)?,
        })
    }
}
