//! 资料库仓库层（M1a）。
//! 封装 projects / records / processing_jobs 三张表的全量 CRUD。
//! UI 与 MCP 都经由此层访问 SQLite，不直接触碰连接（见「架构与数据.md」边界约定）。
//!
//! 连接策略：每次操作打开一个短连接并开启外键约束。桌面单进程写入场景下足够，
//! 且避免长连接跨线程共享的复杂度。

use crate::analysis::AnalysisDraft;
use crate::error::{AppError, AppResult};
use crate::transcript::{normalize_chinese, NORMALIZATION_VERSION};
use crate::types::{
    AnalysisTemplate, McpAccessLog, McpStatus, ProcessingJob, Project, RecordBrief, SearchResult,
    StoredAnalysis, TranscriptSegmentInput, TranscriptVersion,
};
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use uuid::Uuid;

mod action_dashboard;
mod dock;
mod hotwords;
mod inbox;
mod knowledge;
mod memory_snapshots;
mod settings;
mod speakers;
mod transcripts;

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

    /* ----------------------------- projects ----------------------------- */

    pub fn create_project(&self, name: &str) -> AppResult<Project> {
        let name = name.trim();
        if name.is_empty() {
            return Err(AppError::Invalid("项目名不能为空".into()));
        }
        let now = Utc::now().to_rfc3339();
        let project = Project {
            id: Uuid::new_v4().to_string(),
            name: name.to_owned(),
            status: "active".to_owned(),
            created_at: now.clone(),
            updated_at: now,
        };
        self.connect()?.execute(
            "INSERT INTO projects (id, name, status, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![project.id, project.name, project.status, project.created_at, project.updated_at],
        )?;
        Ok(project)
    }

    pub fn list_projects(&self) -> AppResult<Vec<Project>> {
        let connection = self.connect()?;
        let mut statement = connection.prepare(
            "SELECT id, name, status, created_at, updated_at FROM projects ORDER BY updated_at DESC",
        )?;
        let rows = statement
            .query_map([], Self::map_project)?
            .collect::<Result<Vec<_>, rusqlite::Error>>()?;
        Ok(rows)
    }

    pub fn get_project(&self, id: &str) -> AppResult<Project> {
        self.connect()?
            .query_row(
                "SELECT id, name, status, created_at, updated_at FROM projects WHERE id = ?1",
                params![id],
                Self::map_project,
            )
            .optional()?
            .ok_or_else(|| AppError::NotFound(format!("project {id}")))
    }

    /// 更新项目名与状态（传 None 表示保持原值）。
    pub fn update_project(
        &self,
        id: &str,
        name: Option<&str>,
        status: Option<&str>,
    ) -> AppResult<Project> {
        let current = self.get_project(id)?;
        let new_name = match name {
            Some(n) if n.trim().is_empty() => {
                return Err(AppError::Invalid("项目名不能为空".into()))
            }
            Some(n) => n.trim().to_owned(),
            None => current.name,
        };
        let new_status = match status {
            Some(s) if s != "active" && s != "archived" => {
                return Err(AppError::Invalid(format!("非法项目状态: {s}")))
            }
            Some(s) => s.to_owned(),
            None => current.status,
        };
        let now = Utc::now().to_rfc3339();
        self.connect()?.execute(
            "UPDATE projects SET name = ?2, status = ?3, updated_at = ?4 WHERE id = ?1",
            params![id, new_name, new_status, now],
        )?;
        self.get_project(id)
    }

    /// 删除项目。关联记录的 project_id 由外键 ON DELETE SET NULL 置空。
    pub fn delete_project(&self, id: &str) -> AppResult<()> {
        let affected = self
            .connect()?
            .execute("DELETE FROM projects WHERE id = ?1", params![id])?;
        if affected == 0 {
            return Err(AppError::NotFound(format!("project {id}")));
        }
        Ok(())
    }

    /* ------------------------------ records ----------------------------- */

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
        super::record_ops::move_records(&mut transaction, record_ids, project_id)?;
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
        let deleted = super::record_ops::delete_records(&mut transaction, record_ids)?;
        transaction.commit()?;
        Ok(deleted)
    }

    pub fn cleanup_deleted_record_files(
        &self,
        library_root: &Path,
        deleted_records: &[(String, PathBuf)],
    ) -> Vec<PathBuf> {
        super::record_ops::cleanup_deleted_record_files(library_root, deleted_records)
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

    /* --------------------------- processing_jobs ------------------------ */

    pub fn create_job(&self, record_id: &str, job_type: &str) -> AppResult<ProcessingJob> {
        if job_type != "transcribe" && job_type != "analyze" {
            return Err(AppError::Invalid(format!("非法任务类型: {job_type}")));
        }
        // 确保记录存在（外键约束也会拦截，但提前给出明确错误）。
        self.get_record(record_id)?;
        let now = Utc::now().to_rfc3339();
        let job = ProcessingJob {
            id: Uuid::new_v4().to_string(),
            record_id: record_id.to_owned(),
            job_type: job_type.to_owned(),
            status: "queued".to_owned(),
            attempt_count: 0,
            last_error: None,
            stage: None,
            progress_current: 0,
            progress_total: 0,
            created_at: now.clone(),
            updated_at: now,
        };
        self.connect()?.execute(
            "INSERT INTO processing_jobs \
             (id, record_id, job_type, status, attempt_count, last_error, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
            params![
                job.id,
                job.record_id,
                job.job_type,
                job.status,
                job.attempt_count,
                job.last_error,
                job.created_at
            ],
        )?;
        Ok(job)
    }

    pub fn list_jobs_for_record(&self, record_id: &str) -> AppResult<Vec<ProcessingJob>> {
        let connection = self.connect()?;
        let mut statement = connection.prepare(
            "SELECT id, record_id, job_type, status, attempt_count, last_error, stage, progress_current, progress_total, created_at, updated_at \
             FROM processing_jobs WHERE record_id = ?1 ORDER BY created_at ASC",
        )?;
        let rows = statement
            .query_map(params![record_id], Self::map_job)?
            .collect::<Result<Vec<_>, rusqlite::Error>>()?;
        Ok(rows)
    }

    /// 更新任务状态；进入处理态自增 attempt_count，failed 时记录 last_error。
    pub fn update_job_status(
        &self,
        id: &str,
        status: &str,
        last_error: Option<&str>,
    ) -> AppResult<ProcessingJob> {
        Self::validate_processing_status(status)?;
        let now = Utc::now().to_rfc3339();
        let bump = matches!(status, "preparing" | "transcribing" | "analyzing");
        let affected = self.connect()?.execute(
            "UPDATE processing_jobs \
             SET status = ?2, \
                 last_error = ?3, \
                 attempt_count = attempt_count + ?4, \
                 updated_at = ?5 \
             WHERE id = ?1",
            params![id, status, last_error, bump as i64, now],
        )?;
        if affected == 0 {
            return Err(AppError::NotFound(format!("job {id}")));
        }
        self.get_job(id)
    }

    pub fn update_job_progress(
        &self,
        id: &str,
        stage: &str,
        current: i64,
        total: i64,
    ) -> AppResult<ProcessingJob> {
        if stage.trim().is_empty() || current < 0 || total < 0 || (total > 0 && current > total) {
            return Err(AppError::Invalid("处理进度无效".to_owned()));
        }
        let affected = self.connect()?.execute(
            "UPDATE processing_jobs SET stage = ?2, progress_current = ?3, progress_total = ?4, updated_at = ?5 WHERE id = ?1",
            params![id, stage.trim(), current, total, Utc::now().to_rfc3339()],
        )?;
        if affected == 0 {
            return Err(AppError::NotFound(format!("job {id}")));
        }
        self.get_job(id)
    }

    pub fn get_job(&self, id: &str) -> AppResult<ProcessingJob> {
        self.connect()?
            .query_row(
                "SELECT id, record_id, job_type, status, attempt_count, last_error, stage, progress_current, progress_total, created_at, updated_at \
                 FROM processing_jobs WHERE id = ?1",
                params![id],
                Self::map_job,
            )
            .optional()?
            .ok_or_else(|| AppError::NotFound(format!("job {id}")))
    }

    /// 取下一个 queued 任务（FIFO），供后台调度器拉取。
    pub fn next_queued_job(&self) -> AppResult<Option<ProcessingJob>> {
        self.connect()?
            .query_row(
                "SELECT id, record_id, job_type, status, attempt_count, last_error, stage, progress_current, progress_total, created_at, updated_at \
                 FROM processing_jobs WHERE status = 'queued' ORDER BY created_at ASC LIMIT 1",
                [],
                Self::map_job,
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn search(
        &self,
        query: &str,
        project_id: Option<&str>,
        unfiled_only: bool,
        limit: usize,
    ) -> AppResult<Vec<SearchResult>> {
        let query = query.trim();
        if query.is_empty() {
            return Ok(Vec::new());
        }
        let connection = self.connect()?;
        let citation_segment = "(SELECT citations.transcript_segment_id FROM citations WHERE citations.analysis_id = search.source_id AND citations.verified = 1 ORDER BY rowid LIMIT 1)";
        let common_select = format!(
            "SELECT search.record_id, search.project_id, projects.name, search.source_id, \
             COALESCE(segments.id, {citation_segment}), \
             CASE WHEN segments.id IS NOT NULL THEN 'transcript' WHEN analyses.id IS NOT NULL THEN 'analysis' ELSE 'title' END, \
             search.title, {{snippet}}, records.imported_at, \
             COALESCE(segments.speaker_label, citation_segments.speaker_label), \
             COALESCE(segments.start_ms, citation_segments.start_ms), \
             COALESCE(segments.end_ms, citation_segments.end_ms) \
             FROM record_search AS search \
             JOIN records ON records.id = search.record_id \
             LEFT JOIN projects ON projects.id = search.project_id \
             LEFT JOIN transcript_segments AS segments ON segments.id = search.source_id \
             LEFT JOIN analyses ON analyses.id = search.source_id \
             LEFT JOIN transcript_segments AS citation_segments ON citation_segments.id = {citation_segment}"
        );
        let sql = format!(
            "{} WHERE record_search MATCH ?1 AND (?2 IS NULL OR search.project_id = ?2) \
             AND (?3 = 0 OR search.project_id IS NULL) ORDER BY rank LIMIT ?4",
            common_select.replace(
                "{snippet}",
                "snippet(record_search, 4, '<mark>', '</mark>', '...', 20)"
            )
        );
        let short_sql = format!(
            "{} WHERE (search.title LIKE '%' || ?1 || '%' OR search.body LIKE '%' || ?1 || '%') \
             AND (?2 IS NULL OR search.project_id = ?2) AND (?3 = 0 OR search.project_id IS NULL) \
             ORDER BY records.updated_at DESC LIMIT ?4",
            common_select.replace("{snippet}", "search.body")
        );
        let fts_query = Self::build_fts_query(query);
        let use_fts = query.chars().count() >= 3 && fts_query.is_some();
        let mut statement = connection.prepare(if use_fts { &sql } else { &short_sql })?;
        let query_param = fts_query.as_deref().filter(|_| use_fts).unwrap_or(query);
        let results = statement
            .query_map(
                params![
                    query_param,
                    project_id,
                    i64::from(unfiled_only),
                    limit as i64
                ],
                |row| {
                    Ok(SearchResult {
                        record_id: row.get(0)?,
                        project_id: row.get(1)?,
                        project_name: row.get(2)?,
                        source_id: row.get(3)?,
                        target_segment_id: row.get(4)?,
                        source_type: row.get(5)?,
                        title: row.get(6)?,
                        snippet: row.get(7)?,
                        imported_at: row.get(8)?,
                        speaker_label: row.get(9)?,
                        start_ms: row.get(10)?,
                        end_ms: row.get(11)?,
                    })
                },
            )?
            .collect::<Result<Vec<_>, rusqlite::Error>>()?;
        Ok(results)
    }

    fn build_fts_query(query: &str) -> Option<String> {
        let compact = query.chars().collect::<Vec<_>>();
        if (3..=8).contains(&compact.len())
            && compact.iter().all(|character| character.is_alphanumeric())
            && compact.iter().any(|character| !character.is_ascii())
        {
            return Some(format!(r#""{query}""#));
        }

        let mut tokens = Vec::<String>::new();
        let mut run = String::new();

        let push_run = |run: &mut String, tokens: &mut Vec<String>| {
            if run.is_empty() || tokens.len() >= 24 {
                run.clear();
                return;
            }
            let characters = run.chars().collect::<Vec<_>>();
            if characters.len() >= 3 {
                if characters
                    .iter()
                    .all(|character| character.is_ascii_alphanumeric())
                {
                    if !tokens.contains(run) {
                        tokens.push(std::mem::take(run));
                        return;
                    }
                } else {
                    for window in characters.windows(3) {
                        let token = window.iter().collect::<String>();
                        if !tokens.contains(&token) {
                            tokens.push(token);
                            if tokens.len() >= 24 {
                                break;
                            }
                        }
                    }
                }
            }
            run.clear();
        };

        for character in query.chars() {
            if character.is_alphanumeric() {
                run.push(character);
            } else {
                push_run(&mut run, &mut tokens);
            }
            if tokens.len() >= 24 {
                break;
            }
        }
        push_run(&mut run, &mut tokens);
        if tokens.is_empty() {
            return None;
        }
        Some(
            tokens
                .into_iter()
                .map(|token| format!("\"{token}\""))
                .collect::<Vec<_>>()
                .join(" OR "),
        )
    }

    pub fn log_mcp_access(
        &self,
        tool_name: &str,
        record_id: Option<&str>,
        project_id: Option<&str>,
    ) -> AppResult<()> {
        self.connect()?.execute(
            "INSERT INTO mcp_access_logs (id, tool_name, record_id, project_id, called_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![Uuid::new_v4().to_string(), tool_name, record_id, project_id, Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    pub fn mcp_enabled(&self) -> AppResult<bool> {
        let value: String = self.connect()?.query_row(
            "SELECT value FROM app_settings WHERE key = 'mcp_enabled'",
            [],
            |row| row.get(0),
        )?;
        Ok(value == "true")
    }

    pub fn set_mcp_enabled(&self, enabled: bool) -> AppResult<McpStatus> {
        self.connect()?.execute(
            "INSERT INTO app_settings (key, value, updated_at) VALUES ('mcp_enabled', ?1, ?2) \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
            params![if enabled { "true" } else { "false" }, Utc::now().to_rfc3339()],
        )?;
        self.mcp_status()
    }

    pub fn mcp_status(&self) -> AppResult<McpStatus> {
        let connection = self.connect()?;
        let mut statement = connection.prepare(
            "SELECT tool_name, record_id, project_id, called_at FROM mcp_access_logs ORDER BY called_at DESC LIMIT 8",
        )?;
        let recent_calls = statement
            .query_map([], |row| {
                Ok(McpAccessLog {
                    tool_name: row.get(0)?,
                    record_id: row.get(1)?,
                    project_id: row.get(2)?,
                    called_at: row.get(3)?,
                })
            })?
            .collect::<Result<Vec<_>, rusqlite::Error>>()?;
        Ok(McpStatus {
            enabled: self.mcp_enabled()?,
            authorized_scope: "全部知识库（只读）".to_owned(),
            executable_available: false,
            executable_path: None,
            recent_calls,
        })
    }

    /* ------------------------------- analysis -------------------------- */

    pub fn latest_transcript_version(&self, record_id: &str) -> AppResult<TranscriptVersion> {
        self.connect()?.query_row(
            "SELECT id, record_id, provider, model, status, language, pipeline_version, preprocessing_json, created_at FROM transcript_versions WHERE record_id = ?1 AND status = 'completed' ORDER BY created_at DESC LIMIT 1",
            params![record_id], Self::map_transcript_version,
        ).optional()?.ok_or_else(|| AppError::NotFound(format!("record {record_id} 的逐字稿")))
    }

    pub fn save_analysis(
        &self,
        record_id: &str,
        version_id: &str,
        model: &str,
        draft: &AnalysisDraft,
    ) -> AppResult<StoredAnalysis> {
        let template = self.get_analysis_template("builtin-standard")?;
        self.save_analysis_with_template(record_id, version_id, model, draft, &template)
    }

    pub fn save_analysis_with_template(
        &self,
        record_id: &str,
        version_id: &str,
        model: &str,
        draft: &AnalysisDraft,
        template: &AnalysisTemplate,
    ) -> AppResult<StoredAnalysis> {
        let content_json = serde_json::to_string(draft)
            .map_err(|error| AppError::Invalid(format!("分析序列化失败: {error}")))?;
        let template_snapshot_json = serde_json::to_string(template)
            .map_err(|error| AppError::Invalid(format!("模板快照序列化失败: {error}")))?;
        let analysis = StoredAnalysis {
            id: Uuid::new_v4().to_string(),
            record_id: record_id.to_owned(),
            source_transcript_version_id: version_id.to_owned(),
            status: if draft.quality_warning.is_some() {
                "incomplete".to_owned()
            } else {
                "completed".to_owned()
            },
            content_json,
            provider: "ollama".to_owned(),
            model: model.to_owned(),
            template_version: "knowledge-v1".to_owned(),
            template_id: Some(template.id.clone()),
            template_snapshot_json,
            created_at: Utc::now().to_rfc3339(),
        };
        let (project_id, title): (Option<String>, String) = self.connect()?.query_row(
            "SELECT project_id, title FROM records WHERE id = ?1",
            params![record_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let mut connection = self.connect()?;
        let transaction = connection.transaction()?;
        Self::invalidate_record_knowledge(&transaction, record_id, false)?;
        transaction.execute(
            "DELETE FROM record_search WHERE source_id IN (SELECT id FROM analyses WHERE record_id = ?1)",
            params![record_id],
        )?;
        transaction.execute(
            "DELETE FROM action_items WHERE record_id = ?1",
            params![record_id],
        )?;
        transaction.execute("INSERT INTO analyses (id, record_id, source_transcript_version_id, status, content_json, provider, model, template_version, template_id, template_snapshot_json, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)", params![analysis.id, analysis.record_id, analysis.source_transcript_version_id, analysis.status, analysis.content_json, analysis.provider, analysis.model, analysis.template_version, analysis.template_id, analysis.template_snapshot_json, analysis.created_at])?;
        let custom_text = draft
            .custom_sections
            .iter()
            .map(|section| {
                format!(
                    "{} {} {}",
                    section.title,
                    section.text,
                    section
                        .items
                        .iter()
                        .map(|item| item.text.as_str())
                        .collect::<Vec<_>>()
                        .join(" ")
                )
            })
            .collect::<Vec<_>>()
            .join(" ");
        let search_text = format!(
            "{} {} {} {} {} {}",
            draft.summary,
            draft
                .key_points
                .iter()
                .map(|item| item.text.as_str())
                .collect::<Vec<_>>()
                .join(" "),
            draft
                .decisions
                .iter()
                .map(|item| item.text.as_str())
                .collect::<Vec<_>>()
                .join(" "),
            draft
                .action_items
                .iter()
                .map(|item| item.text.as_str())
                .collect::<Vec<_>>()
                .join(" "),
            draft
                .open_questions
                .iter()
                .map(|item| item.text.as_str())
                .collect::<Vec<_>>()
                .join(" "),
            custom_text
        );
        transaction.execute("INSERT INTO record_search (record_id, project_id, source_id, title, body) VALUES (?1, ?2, ?3, ?4, ?5)", params![record_id, project_id, analysis.id, title, search_text])?;
        for (kind, items) in [
            ("key_points", &draft.key_points),
            ("decisions", &draft.decisions),
            ("action_items", &draft.action_items),
            ("open_questions", &draft.open_questions),
        ] {
            for (index, item) in items.iter().enumerate() {
                for segment_id in &item.citation_segment_ids {
                    transaction.execute("INSERT INTO citations (id, analysis_id, item_path, transcript_segment_id, quote_text, verified) VALUES (?1, ?2, ?3, ?4, ?5, 1)", params![Uuid::new_v4().to_string(), analysis.id, format!("{kind}[{index}]"), segment_id, item.quote_text])?;
                }
                if kind == "action_items" {
                    transaction.execute("INSERT INTO action_items (id, record_id, project_id, title, owner_text, source_segment_id, analysis_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)", params![Uuid::new_v4().to_string(), record_id, project_id, item.text, item.owner.clone().unwrap_or_default(), item.citation_segment_ids.first(), analysis.id])?;
                }
            }
        }
        for (section_index, section) in draft.custom_sections.iter().enumerate() {
            for (item_index, item) in section.items.iter().enumerate() {
                for segment_id in &item.citation_segment_ids {
                    transaction.execute(
                        "INSERT INTO citations (id, analysis_id, item_path, transcript_segment_id, quote_text, verified) VALUES (?1, ?2, ?3, ?4, ?5, 1)",
                        params![Uuid::new_v4().to_string(), analysis.id, format!("custom_sections[{section_index}].items[{item_index}]"), segment_id, item.quote_text],
                    )?;
                }
            }
        }
        transaction.commit()?;
        Ok(analysis)
    }

    pub fn latest_analysis(&self, record_id: &str) -> AppResult<Option<StoredAnalysis>> {
        self.connect()?.query_row("SELECT id, record_id, source_transcript_version_id, status, content_json, provider, model, template_version, template_id, template_snapshot_json, created_at FROM analyses WHERE record_id = ?1 ORDER BY created_at DESC LIMIT 1", params![record_id], Self::map_analysis).optional().map_err(Into::into)
    }

    pub fn get_analysis(&self, id: &str) -> AppResult<StoredAnalysis> {
        self.connect()?
            .query_row(
                "SELECT id, record_id, source_transcript_version_id, status, content_json, provider, model, template_version, template_id, template_snapshot_json, created_at FROM analyses WHERE id = ?1",
                params![id],
                Self::map_analysis,
            )
            .optional()?
            .ok_or_else(|| AppError::NotFound(format!("analysis {id}")))
    }

    pub fn list_action_items(
        &self,
        project_id: Option<&str>,
        status: Option<&str>,
    ) -> AppResult<Vec<crate::types::ActionItem>> {
        let connection = self.connect()?;
        let mut statement = connection.prepare("SELECT id, record_id, COALESCE(project_id, ''), title, status, source_segment_id, analysis_id FROM action_items WHERE (?1 IS NULL OR project_id = ?1) AND (?2 IS NULL OR status = ?2) ORDER BY rowid DESC")?;
        let items = statement
            .query_map(params![project_id, status], |row| {
                Ok(crate::types::ActionItem {
                    id: row.get(0)?,
                    record_id: row.get(1)?,
                    project_id: row.get(2)?,
                    title: row.get(3)?,
                    status: row.get(4)?,
                    source_segment_id: row.get(5)?,
                    analysis_id: row.get(6)?,
                })
            })?
            .collect::<Result<Vec<_>, rusqlite::Error>>()?;
        Ok(items)
    }

    pub fn list_decisions(
        &self,
        project_id: Option<&str>,
        limit: usize,
    ) -> AppResult<Vec<serde_json::Value>> {
        let connection = self.connect()?;
        let rows = {
            let mut statement = connection.prepare(
                "SELECT analyses.content_json, analyses.record_id, records.title \
                 FROM analyses JOIN records ON records.id = analyses.record_id \
                 WHERE (?1 IS NULL OR records.project_id = ?1) AND analyses.status = 'completed' \
                 AND analyses.id = (SELECT latest.id FROM analyses AS latest WHERE latest.record_id = records.id ORDER BY latest.created_at DESC LIMIT 1) \
                 ORDER BY analyses.created_at DESC LIMIT ?2",
            )?;
            let rows = statement
                .query_map(params![project_id, limit as i64], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                })?
                .collect::<Result<Vec<_>, rusqlite::Error>>()?;
            rows
        };
        let mut decisions = Vec::new();
        for (content_json, record_id, title) in rows {
            let Ok(draft) = serde_json::from_str::<AnalysisDraft>(&content_json) else {
                continue;
            };
            for item in draft.decisions {
                let mut start_ms = item.start_ms;
                let mut end_ms = item.end_ms;
                if start_ms.is_none() {
                    if let Some(segment_id) = item.citation_segment_ids.first() {
                        if let Some((start, end)) = connection
                            .query_row(
                                "SELECT start_ms, end_ms FROM transcript_segments WHERE id = ?1",
                                params![segment_id],
                                |row| Ok((row.get(0)?, row.get(1)?)),
                            )
                            .optional()?
                        {
                            start_ms = Some(start);
                            end_ms = Some(end);
                        }
                    }
                }
                decisions.push(serde_json::json!({
                    "recordId": record_id,
                    "recordTitle": title,
                    "text": item.text,
                    "citationSegmentIds": item.citation_segment_ids,
                    "quoteText": item.quote_text,
                    "startMs": start_ms,
                    "endMs": end_ms,
                }));
                if decisions.len() >= limit {
                    return Ok(decisions);
                }
            }
        }
        Ok(decisions)
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

    pub fn recover_interrupted_processing_jobs(&self) -> AppResult<usize> {
        let mut connection = self.connect()?;
        let transaction = connection.transaction()?;
        let now = Utc::now().to_rfc3339();
        let error = "上次处理因应用退出而中断，请重试";
        let changed = transaction.execute(
            "UPDATE processing_jobs SET status = 'failed', last_error = ?1, updated_at = ?2 WHERE status IN ('preparing', 'transcribing', 'analyzing')",
            params![error, now],
        )?;
        transaction.execute(
            "UPDATE records SET processing_status = CASE WHEN EXISTS(SELECT 1 FROM transcript_versions WHERE transcript_versions.record_id = records.id AND transcript_versions.status = 'completed') THEN 'completed' ELSE 'failed' END, updated_at = ?1 WHERE processing_status IN ('preparing', 'transcribing', 'analyzing')",
            params![now],
        )?;
        transaction.commit()?;
        Ok(changed)
    }

    fn map_project(row: &Row<'_>) -> rusqlite::Result<Project> {
        Ok(Project {
            id: row.get(0)?,
            name: row.get(1)?,
            status: row.get(2)?,
            created_at: row.get(3)?,
            updated_at: row.get(4)?,
        })
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

    fn map_job(row: &Row<'_>) -> rusqlite::Result<ProcessingJob> {
        Ok(ProcessingJob {
            id: row.get(0)?,
            record_id: row.get(1)?,
            job_type: row.get(2)?,
            status: row.get(3)?,
            attempt_count: row.get(4)?,
            last_error: row.get(5)?,
            stage: row.get(6)?,
            progress_current: row.get(7)?,
            progress_total: row.get(8)?,
            created_at: row.get(9)?,
            updated_at: row.get(10)?,
        })
    }

    fn map_transcript_version(row: &Row<'_>) -> rusqlite::Result<TranscriptVersion> {
        Ok(TranscriptVersion {
            id: row.get(0)?,
            record_id: row.get(1)?,
            provider: row.get(2)?,
            model: row.get(3)?,
            status: row.get(4)?,
            language: row.get(5)?,
            pipeline_version: row.get(6)?,
            preprocessing_json: row.get(7)?,
            created_at: row.get(8)?,
        })
    }

    fn map_analysis(row: &Row<'_>) -> rusqlite::Result<StoredAnalysis> {
        Ok(StoredAnalysis {
            id: row.get(0)?,
            record_id: row.get(1)?,
            source_transcript_version_id: row.get(2)?,
            status: row.get(3)?,
            content_json: row.get(4)?,
            provider: row.get(5)?,
            model: row.get(6)?,
            template_version: row.get(7)?,
            template_id: row.get(8)?,
            template_snapshot_json: row.get(9)?,
            created_at: row.get(10)?,
        })
    }
    /* ----------------------------- v0.3.0 extensions ----------------------------- */

    /// 读取通用 KV 设置；不存在时返回 None。
    pub fn setting_value(&self, key: &str) -> AppResult<Option<String>> {
        let connection = self.connect()?;
        let mut statement = connection.prepare("SELECT value FROM app_settings WHERE key = ?1")?;
        let mut rows = statement.query(params![key])?;
        Ok(match rows.next()? {
            Some(row) => Some(row.get(0)?),
            None => None,
        })
    }

    pub fn set_setting_value(&self, key: &str, value: &str) -> AppResult<()> {
        self.connect()?.execute(
            "INSERT INTO app_settings (key, value, updated_at) VALUES (?1, ?2, ?3)              ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
            params![key, value, Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    pub fn onboarding_completed_at(&self) -> AppResult<Option<String>> {
        Ok(self
            .setting_value("onboarding_completed_at")?
            .filter(|value| !value.trim().is_empty()))
    }

    pub fn complete_onboarding(&self) -> AppResult<()> {
        self.set_setting_value("onboarding_completed_at", &Utc::now().to_rfc3339())
    }

    pub fn reset_onboarding(&self) -> AppResult<()> {
        self.set_setting_value("onboarding_completed_at", "")
    }
}
