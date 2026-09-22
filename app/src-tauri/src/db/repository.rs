//! 资料库仓库层（M1a）。
//! 封装 projects / records / processing_jobs 三张表的全量 CRUD。
//! UI 与 MCP 都经由此层访问 SQLite，不直接触碰连接（见「架构与数据.md」边界约定）。
//!
//! 连接策略：每次操作打开一个短连接并开启外键约束。桌面单进程写入场景下足够，
//! 且避免长连接跨线程共享的复杂度。

use crate::analysis::AnalysisDraft;
use crate::error::{AppError, AppResult};
use crate::transcript::{build_blocks, normalize_chinese, NORMALIZATION_VERSION};
use crate::types::{
    ActionDashboardItem, AnalysisTemplate, DockChat, DockMessage, ExternalAiSettings, Hotword,
    InboxCounts, InboxSeenFile, InboxWatchFolder, KnowledgeIndexStatus, KnowledgeSettings,
    McpAccessLog, McpStatus, MemoryFeedback, MemoryGenerationStatus, MemoryScope, MemorySnapshot,
    MemorySnapshotResult, MemoryViewKind, OpenQuestionItem, ProcessingJob, Project, RecordBrief,
    SearchResult, SpeakerSummary, StoredAnalysis, TemplateSection, TranscriptBlock,
    TranscriptSegment, TranscriptSegmentInput, TranscriptVersion,
};
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::Deserialize;
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

    /* ----------------------------- transcripts ------------------------- */

    pub fn audio_path_for_record(&self, record_id: &str) -> AppResult<PathBuf> {
        self.connect()?
            .query_row(
                "SELECT audio_path FROM records WHERE id = ?1",
                params![record_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .map(PathBuf::from)
            .ok_or_else(|| AppError::NotFound(format!("record {record_id}")))
    }

    pub fn save_transcript(
        &self,
        record_id: &str,
        provider: &str,
        model: &str,
        segments: &[TranscriptSegmentInput],
    ) -> AppResult<(TranscriptVersion, Vec<TranscriptSegment>)> {
        self.save_transcript_with_metadata(
            record_id,
            provider,
            model,
            "zh",
            "legacy-v1",
            "{}",
            segments,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn save_transcript_with_metadata(
        &self,
        record_id: &str,
        provider: &str,
        model: &str,
        language: &str,
        pipeline_version: &str,
        preprocessing_json: &str,
        segments: &[TranscriptSegmentInput],
    ) -> AppResult<(TranscriptVersion, Vec<TranscriptSegment>)> {
        self.get_record(record_id)?;
        if segments.is_empty() {
            return Err(AppError::Invalid("逐字稿不能没有片段".into()));
        }
        let now = Utc::now().to_rfc3339();
        let version = TranscriptVersion {
            id: Uuid::new_v4().to_string(),
            record_id: record_id.to_owned(),
            provider: provider.to_owned(),
            model: model.to_owned(),
            status: "completed".to_owned(),
            language: language.to_owned(),
            pipeline_version: pipeline_version.to_owned(),
            preprocessing_json: preprocessing_json.to_owned(),
            created_at: now.clone(),
        };
        let mut connection = self.connect()?;
        let transaction = connection.transaction()?;
        transaction.execute(
            "DELETE FROM record_search WHERE source_id IN (SELECT id FROM transcript_segments WHERE record_id = ?1)",
            params![record_id],
        )?;
        Self::invalidate_record_knowledge(&transaction, record_id, true)?;
        transaction.execute(
            "INSERT INTO transcript_versions (id, record_id, provider, model, status, language, pipeline_version, preprocessing_json, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![version.id, version.record_id, version.provider, version.model, version.status, version.language, version.pipeline_version, version.preprocessing_json, version.created_at],
        )?;
        let mut saved = Vec::with_capacity(segments.len());
        for (sequence, segment) in segments.iter().enumerate() {
            if segment.end_ms < segment.start_ms || segment.original_text.trim().is_empty() {
                return Err(AppError::Invalid("逐字稿片段无效".into()));
            }
            let saved_segment = TranscriptSegment {
                id: Uuid::new_v4().to_string(),
                record_id: record_id.to_owned(),
                sequence: sequence as i64,
                speaker_label: segment.speaker_label.clone(),
                start_ms: segment.start_ms,
                end_ms: segment.end_ms,
                original_text: segment.original_text.trim().to_owned(),
                normalized_text: Some(normalize_chinese(segment.original_text.trim())),
                normalization_version: Some(NORMALIZATION_VERSION.to_owned()),
                edited_text: None,
            };
            transaction.execute(
                "INSERT INTO transcript_segments (id, transcript_version_id, record_id, sequence, speaker_label, start_ms, end_ms, original_text, normalized_text, normalization_version, edited_text, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, NULL, ?11, ?11)",
                params![saved_segment.id, version.id, saved_segment.record_id, saved_segment.sequence, saved_segment.speaker_label, saved_segment.start_ms, saved_segment.end_ms, saved_segment.original_text, saved_segment.normalized_text, saved_segment.normalization_version, now],
            )?;
            saved.push(saved_segment);
        }
        transaction.commit()?;
        Ok((version, saved))
    }

    pub fn list_transcript_segments(&self, record_id: &str) -> AppResult<Vec<TranscriptSegment>> {
        let connection = self.connect()?;
        let mut statement = connection.prepare(
            "SELECT segments.id, segments.record_id, segments.sequence, segments.speaker_label, segments.start_ms, segments.end_ms, segments.original_text, segments.normalized_text, segments.normalization_version, segments.edited_text \
             FROM transcript_segments AS segments \
             WHERE segments.transcript_version_id = ( \
               SELECT versions.id FROM transcript_versions AS versions \
               WHERE versions.record_id = ?1 AND versions.status = 'completed' \
               ORDER BY versions.created_at DESC LIMIT 1 \
             ) ORDER BY segments.sequence ASC",
        )?;
        let segments = statement
            .query_map(params![record_id], Self::map_segment)?
            .collect::<Result<Vec<_>, rusqlite::Error>>()?;
        Ok(segments)
    }

    pub fn list_transcript_blocks(&self, record_id: &str) -> AppResult<Vec<TranscriptBlock>> {
        Ok(build_blocks(&self.list_transcript_segments(record_id)?))
    }

    pub fn update_segment_text(
        &self,
        id: &str,
        edited_text: Option<&str>,
    ) -> AppResult<TranscriptSegment> {
        let mut connection = self.connect()?;
        let transaction = connection.transaction()?;
        let record_id: Option<String> = transaction
            .query_row(
                "SELECT record_id FROM transcript_segments WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .optional()?;
        let record_id = record_id.ok_or_else(|| AppError::NotFound(format!("segment {id}")))?;
        let affected = transaction.execute(
            "UPDATE transcript_segments SET edited_text = ?2, updated_at = ?3 WHERE id = ?1",
            params![
                id,
                edited_text.map(str::trim).filter(|text| !text.is_empty()),
                Utc::now().to_rfc3339()
            ],
        )?;
        debug_assert_eq!(affected, 1);
        Self::invalidate_record_knowledge(&transaction, &record_id, true)?;
        transaction.commit()?;
        self.connect()?.query_row(
            "SELECT id, record_id, sequence, speaker_label, start_ms, end_ms, original_text, normalized_text, normalization_version, edited_text FROM transcript_segments WHERE id = ?1",
            params![id], Self::map_segment,
        ).map_err(Into::into)
    }

    fn backfill_normalized_transcripts(&self) -> AppResult<()> {
        let mut connection = self.connect()?;
        let rows = {
            let mut statement = connection.prepare(
                "SELECT id, original_text FROM transcript_segments WHERE normalized_text IS NULL OR normalization_version IS NULL",
            )?;
            let rows = statement
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<Result<Vec<_>, rusqlite::Error>>()?;
            rows
        };
        if rows.is_empty() {
            return Ok(());
        }
        let transaction = connection.transaction()?;
        for (id, original) in rows {
            transaction.execute(
                "UPDATE transcript_segments SET normalized_text = ?2, normalization_version = ?3 WHERE id = ?1",
                params![id, normalize_chinese(&original), NORMALIZATION_VERSION],
            )?;
        }
        transaction.commit()?;
        Ok(())
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

    /* ----------------------- settings and templates -------------------- */

    pub fn knowledge_settings(&self) -> AppResult<KnowledgeSettings> {
        Ok(KnowledgeSettings {
            transcription_language: self.setting("transcription_language", "zh")?,
            whisper_model_path: self.setting("whisper_model_path", "")?,
            analysis_model: self.setting("analysis_model", "qwen2.5:7b")?,
            embedding_model: self.setting("embedding_model", "qwen3-embedding:0.6b")?,
        })
    }

    pub fn update_knowledge_settings(
        &self,
        settings: &KnowledgeSettings,
    ) -> AppResult<KnowledgeSettings> {
        let language = settings.transcription_language.trim();
        if language.is_empty() || language.len() > 16 {
            return Err(AppError::Invalid("转写语言无效".to_owned()));
        }
        for (label, value) in [
            ("分析模型", settings.analysis_model.trim()),
            ("嵌入模型", settings.embedding_model.trim()),
        ] {
            if value.is_empty() || value.len() > 120 {
                return Err(AppError::Invalid(format!("{label}无效")));
            }
        }
        let model_path = settings.whisper_model_path.trim();
        if !model_path.is_empty() && !Path::new(model_path).is_file() {
            return Err(AppError::Invalid(
                "选择的 Whisper 模型文件不存在".to_owned(),
            ));
        }
        let mut connection = self.connect()?;
        let transaction = connection.transaction()?;
        let now = Utc::now().to_rfc3339();
        for (key, value) in [
            ("transcription_language", language),
            ("whisper_model_path", model_path),
            ("analysis_model", settings.analysis_model.trim()),
            ("embedding_model", settings.embedding_model.trim()),
        ] {
            transaction.execute(
                "INSERT INTO app_settings (key, value, updated_at) VALUES (?1, ?2, ?3) ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
                params![key, value, now],
            )?;
        }
        transaction.execute(
            "UPDATE knowledge_index_state SET status = 'stale', embedding_model = ?1, updated_at = ?2 WHERE embedding_model != ?1",
            params![settings.embedding_model.trim(), now],
        )?;
        transaction.commit()?;
        self.knowledge_settings()
    }

    pub fn external_ai_settings(&self, has_api_key: bool) -> AppResult<ExternalAiSettings> {
        Ok(ExternalAiSettings {
            enabled: self.setting("external_ai_enabled", "false")? == "true",
            base_url: self.setting("external_ai_base_url", "https://api.openai.com/v1")?,
            model: self.setting("external_ai_model", "gpt-4.1-mini")?,
            has_api_key,
            privacy_consent_at: match self.setting("external_ai_privacy_consent_at", "")? {
                value if value.is_empty() => None,
                value => Some(value),
            },
            transcription_provider: self.setting("cloud_transcription_provider", "none")?,
        })
    }

    pub fn update_external_ai_settings(
        &self,
        settings: &ExternalAiSettings,
        has_api_key: bool,
    ) -> AppResult<ExternalAiSettings> {
        let base_url = settings.base_url.trim().trim_end_matches('/');
        let model = settings.model.trim();
        if base_url.is_empty() || base_url.len() > 500 {
            return Err(AppError::Invalid("外部 AI Base URL 无效".to_owned()));
        }
        // 安全校验：解析真实 host 后判定（字符串前缀可被 userinfo/子域绕过，
        // 详见 memory.rs::validate_external_base_url）。
        crate::memory::validate_external_base_url(base_url)?;
        if model.is_empty() || model.len() > 160 {
            return Err(AppError::Invalid("外部 AI 模型名称无效".to_owned()));
        }
        if settings.enabled && settings.privacy_consent_at.is_none() {
            return Err(AppError::Invalid(
                "启用外部 AI 前必须确认文本发送说明".to_owned(),
            ));
        }
        let mut connection = self.connect()?;
        let transaction = connection.transaction()?;
        let now = Utc::now().to_rfc3339();
        let consent = settings.privacy_consent_at.as_deref().unwrap_or("");
        for (key, value) in [
            (
                "external_ai_enabled",
                if settings.enabled { "true" } else { "false" },
            ),
            ("external_ai_base_url", base_url),
            ("external_ai_model", model),
            ("external_ai_privacy_consent_at", consent),
            ("cloud_transcription_provider", "none"),
        ] {
            transaction.execute(
                "INSERT INTO app_settings (key, value, updated_at) VALUES (?1, ?2, ?3) ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
                params![key, value, now],
            )?;
        }
        transaction.commit()?;
        self.external_ai_settings(has_api_key)
    }

    fn setting(&self, key: &str, default: &str) -> AppResult<String> {
        Ok(self
            .connect()?
            .query_row(
                "SELECT value FROM app_settings WHERE key = ?1",
                params![key],
                |row| row.get(0),
            )
            .optional()?
            .unwrap_or_else(|| default.to_owned()))
    }

    pub fn list_analysis_templates(&self) -> AppResult<Vec<AnalysisTemplate>> {
        let connection = self.connect()?;
        let mut statement = connection.prepare(
            "SELECT id, name, description, focus_instructions, custom_sections_json, is_builtin, created_at, updated_at FROM analysis_templates ORDER BY is_builtin DESC, name ASC",
        )?;
        let templates = statement
            .query_map([], Self::map_analysis_template)?
            .collect::<Result<Vec<_>, rusqlite::Error>>()?;
        Ok(templates)
    }

    pub fn get_analysis_template(&self, id: &str) -> AppResult<AnalysisTemplate> {
        self.connect()?
            .query_row(
                "SELECT id, name, description, focus_instructions, custom_sections_json, is_builtin, created_at, updated_at FROM analysis_templates WHERE id = ?1",
                params![id],
                Self::map_analysis_template,
            )
            .optional()?
            .ok_or_else(|| AppError::NotFound(format!("analysis template {id}")))
    }

    pub fn create_analysis_template(
        &self,
        name: &str,
        description: &str,
        focus_instructions: &str,
        sections: &[TemplateSection],
    ) -> AppResult<AnalysisTemplate> {
        Self::validate_template(name, focus_instructions, sections)?;
        let now = Utc::now().to_rfc3339();
        let id = Uuid::new_v4().to_string();
        let sections_json = serde_json::to_string(sections)
            .map_err(|error| AppError::Invalid(format!("模板栏目无效: {error}")))?;
        self.connect()?.execute(
            "INSERT INTO analysis_templates (id, name, description, focus_instructions, custom_sections_json, is_builtin, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6, ?6)",
            params![id, name.trim(), description.trim(), focus_instructions.trim(), sections_json, now],
        )?;
        self.get_analysis_template(&id)
    }

    pub fn update_analysis_template(
        &self,
        id: &str,
        name: &str,
        description: &str,
        focus_instructions: &str,
        sections: &[TemplateSection],
    ) -> AppResult<AnalysisTemplate> {
        let current = self.get_analysis_template(id)?;
        if current.is_builtin {
            return Err(AppError::Invalid(
                "内置模板不可直接修改，请先复制".to_owned(),
            ));
        }
        Self::validate_template(name, focus_instructions, sections)?;
        let sections_json = serde_json::to_string(sections)
            .map_err(|error| AppError::Invalid(format!("模板栏目无效: {error}")))?;
        self.connect()?.execute(
            "UPDATE analysis_templates SET name = ?2, description = ?3, focus_instructions = ?4, custom_sections_json = ?5, updated_at = ?6 WHERE id = ?1",
            params![id, name.trim(), description.trim(), focus_instructions.trim(), sections_json, Utc::now().to_rfc3339()],
        )?;
        self.get_analysis_template(id)
    }

    pub fn delete_analysis_template(&self, id: &str) -> AppResult<()> {
        let current = self.get_analysis_template(id)?;
        if current.is_builtin {
            return Err(AppError::Invalid("内置模板不可删除".to_owned()));
        }
        let mut connection = self.connect()?;
        let transaction = connection.transaction()?;
        transaction.execute(
            "UPDATE records SET analysis_template_id = 'builtin-standard' WHERE analysis_template_id = ?1",
            params![id],
        )?;
        transaction.execute("DELETE FROM analysis_templates WHERE id = ?1", params![id])?;
        transaction.commit()?;
        Ok(())
    }

    fn validate_template(
        name: &str,
        focus_instructions: &str,
        sections: &[TemplateSection],
    ) -> AppResult<()> {
        if name.trim().is_empty() || name.chars().count() > 40 {
            return Err(AppError::Invalid("模板名称应为 1 到 40 个字符".to_owned()));
        }
        if focus_instructions.trim().is_empty() || focus_instructions.chars().count() > 800 {
            return Err(AppError::Invalid("模板重点应为 1 到 800 个字符".to_owned()));
        }
        if sections.len() > 10 {
            return Err(AppError::Invalid("自定义栏目不能超过 10 个".to_owned()));
        }
        let mut keys = std::collections::HashSet::new();
        for section in sections {
            if section.key.trim().is_empty()
                || section.title.trim().is_empty()
                || !matches!(section.format.as_str(), "paragraph" | "list")
                || section.instruction.trim().is_empty()
                || !keys.insert(section.key.trim().to_owned())
            {
                return Err(AppError::Invalid("自定义栏目名称、格式或键无效".to_owned()));
            }
        }
        Ok(())
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

    /* -------------------------- knowledge index ------------------------ */

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
             WHERE chunks.embedding_model = ?1 AND (?2 IS NULL OR chunks.project_id = ?2) AND (?3 = 0 OR chunks.project_id IS NULL)",
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
                    "SELECT COUNT(*), COALESCE(SUM(EXISTS(SELECT 1 FROM knowledge_chunks AS chunks WHERE chunks.record_id = records.id AND chunks.embedding_model = ?3)), 0), COALESCE((SELECT COUNT(*) FROM knowledge_chunks AS chunks WHERE chunks.embedding_model = ?3 AND (?1 IS NULL OR chunks.project_id = ?1) AND (?2 = 0 OR chunks.project_id IS NULL)), 0) FROM records WHERE EXISTS(SELECT 1 FROM transcript_versions WHERE transcript_versions.record_id = records.id AND transcript_versions.status = 'completed') AND (?1 IS NULL OR records.project_id = ?1) AND (?2 = 0 OR records.project_id IS NULL)",
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
            "SELECT COUNT(*), SUM(EXISTS(SELECT 1 FROM transcript_versions WHERE transcript_versions.record_id = records.id AND status = 'completed')), SUM(EXISTS(SELECT 1 FROM analyses WHERE analyses.record_id = records.id AND status = 'completed' AND analyses.id = (SELECT id FROM analyses AS latest WHERE latest.record_id = records.id ORDER BY created_at DESC LIMIT 1))) FROM records WHERE (?1 IS NULL OR project_id = ?1) AND (?2 = 0 OR project_id IS NULL)",
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
            "SELECT items.record_id, records.title, items.title, COALESCE(segments.edited_text, segments.normalized_text, segments.original_text, ''), items.source_segment_id, segments.start_ms, segments.end_ms FROM action_items AS items JOIN records ON records.id = items.record_id LEFT JOIN transcript_segments AS segments ON segments.id = items.source_segment_id WHERE (?1 IS NULL OR items.project_id = ?1) AND (?2 = 0 OR items.project_id IS NULL) ORDER BY items.rowid DESC LIMIT 8",
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
            .chunks_exact(4)
            .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
            .collect())
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

    /* --------------------------- memory snapshots --------------------------- */

    pub fn create_memory_snapshot(
        &self,
        view_kind: &MemoryViewKind,
        scope: &MemoryScope,
        range_start: Option<&str>,
        range_end: Option<&str>,
        model: &str,
        source_record_ids: &[String],
        request_hash: &str,
    ) -> AppResult<MemorySnapshot> {
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

    fn map_project(row: &Row<'_>) -> rusqlite::Result<Project> {
        Ok(Project {
            id: row.get(0)?,
            name: row.get(1)?,
            status: row.get(2)?,
            created_at: row.get(3)?,
            updated_at: row.get(4)?,
        })
    }

    fn map_analysis_template(row: &Row<'_>) -> rusqlite::Result<AnalysisTemplate> {
        let sections_json: String = row.get(4)?;
        let custom_sections = serde_json::from_str::<Vec<TemplateSection>>(&sections_json)
            .map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    4,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })?;
        Ok(AnalysisTemplate {
            id: row.get(0)?,
            name: row.get(1)?,
            description: row.get(2)?,
            focus_instructions: row.get(3)?,
            custom_sections,
            is_builtin: row.get::<_, i64>(5)? != 0,
            created_at: row.get(6)?,
            updated_at: row.get(7)?,
        })
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

    fn map_segment(row: &Row<'_>) -> rusqlite::Result<TranscriptSegment> {
        Ok(TranscriptSegment {
            id: row.get(0)?,
            record_id: row.get(1)?,
            sequence: row.get(2)?,
            speaker_label: row.get(3)?,
            start_ms: row.get(4)?,
            end_ms: row.get(5)?,
            original_text: row.get(6)?,
            normalized_text: row.get(7)?,
            normalization_version: row.get(8)?,
            edited_text: row.get(9)?,
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

    pub fn inbox_usb_detection(&self) -> AppResult<bool> {
        Ok(self
            .setting_value("inbox_usb_detection")?
            .as_deref()
            .is_some_and(|value| value == "true"))
    }

    pub fn set_inbox_usb_detection(&self, enabled: bool) -> AppResult<()> {
        self.set_setting_value(
            "inbox_usb_detection",
            if enabled { "true" } else { "false" },
        )
    }

    pub fn list_watch_folders(&self) -> AppResult<Vec<InboxWatchFolder>> {
        let connection = self.connect()?;
        let mut statement = connection.prepare(
            "SELECT id, path, label, enabled, created_at FROM inbox_watch_folders ORDER BY created_at",
        )?;
        let rows = statement
            .query_map([], |row| {
                Ok(InboxWatchFolder {
                    id: row.get(0)?,
                    path: row.get(1)?,
                    label: row.get(2)?,
                    enabled: row.get::<_, i64>(3)? != 0,
                    created_at: row.get(4)?,
                })
            })?
            .collect::<Result<Vec<_>, rusqlite::Error>>()?;
        Ok(rows)
    }

    pub fn add_watch_folder(&self, path: &str, label: &str) -> AppResult<InboxWatchFolder> {
        let folder = InboxWatchFolder {
            id: Uuid::new_v4().to_string(),
            path: path.to_owned(),
            label: label.to_owned(),
            enabled: true,
            created_at: Utc::now().to_rfc3339(),
        };
        self.connect()?.execute(
            "INSERT INTO inbox_watch_folders (id, path, label, enabled, created_at) VALUES (?1, ?2, ?3, 1, ?4)",
            params![folder.id, folder.path, folder.label, folder.created_at],
        )?;
        Ok(folder)
    }

    pub fn remove_watch_folder(&self, id: &str) -> AppResult<()> {
        self.connect()?
            .execute("DELETE FROM inbox_watch_folders WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn seen_file_by_path(&self, file_path: &str) -> AppResult<Option<InboxSeenFile>> {
        let connection = self.connect()?;
        let mut statement = connection.prepare(
            "SELECT id, source_kind, source_path, file_path, file_name, file_size, mtime_ms, sha256, status, record_id, error_message, seen_at, updated_at              FROM inbox_seen_files WHERE file_path = ?1",
        )?;
        let mut rows = statement.query(params![file_path])?;
        Ok(match rows.next()? {
            Some(row) => Some(Self::row_to_seen_file(row)?),
            None => None,
        })
    }

    pub fn insert_seen_file(&self, file: &InboxSeenFile) -> AppResult<()> {
        self.connect()?.execute(
            "INSERT INTO inbox_seen_files (id, source_kind, source_path, file_path, file_name, file_size, mtime_ms, sha256, status, record_id, error_message, seen_at, updated_at)              VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                file.id,
                file.source_kind,
                file.source_path,
                file.file_path,
                file.file_name,
                file.file_size,
                file.mtime_ms,
                file.sha256,
                file.status,
                file.record_id,
                file.error_message,
                file.seen_at,
                file.updated_at,
            ],
        )?;
        Ok(())
    }

    pub fn update_seen_file_status(
        &self,
        id: &str,
        status: &str,
        record_id: Option<&str>,
        error_message: Option<&str>,
    ) -> AppResult<()> {
        self.connect()?.execute(
            "UPDATE inbox_seen_files SET status = ?2, record_id = ?3, error_message = ?4, updated_at = ?5 WHERE id = ?1",
            params![id, status, record_id, error_message, Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    /// 启动恢复：上次退出时仍处于 pending/importing 的收件箱文件。
    pub fn list_active_seen_files(&self) -> AppResult<Vec<InboxSeenFile>> {
        let connection = self.connect()?;
        let mut statement = connection.prepare(
            "SELECT id, source_kind, source_path, file_path, file_name, file_size, mtime_ms, sha256, status, record_id, error_message, seen_at, updated_at \
             FROM inbox_seen_files WHERE status IN ('pending', 'importing') ORDER BY seen_at",
        )?;
        let rows = statement
            .query_map([], |row| Self::row_to_seen_file(row))?
            .collect::<Result<Vec<_>, rusqlite::Error>>()?;
        Ok(rows)
    }

    pub fn recent_seen_files(&self, limit: u32) -> AppResult<Vec<InboxSeenFile>> {
        let connection = self.connect()?;
        let mut statement = connection.prepare(
            "SELECT id, source_kind, source_path, file_path, file_name, file_size, mtime_ms, sha256, status, record_id, error_message, seen_at, updated_at              FROM inbox_seen_files ORDER BY seen_at DESC LIMIT ?1",
        )?;
        let rows = statement
            .query_map(params![limit], |row| Self::row_to_seen_file(row))?
            .collect::<Result<Vec<_>, rusqlite::Error>>()?;
        Ok(rows)
    }

    pub fn inbox_status_counts(&self) -> AppResult<InboxCounts> {
        let connection = self.connect()?;
        let pending: i64 = connection.query_row(
            "SELECT COUNT(*) FROM inbox_seen_files WHERE status IN ('pending', 'importing')",
            [],
            |row| row.get(0),
        )?;
        let imported: i64 = connection.query_row(
            "SELECT COUNT(*) FROM inbox_seen_files WHERE status = 'imported'",
            [],
            |row| row.get(0),
        )?;
        let failed: i64 = connection.query_row(
            "SELECT COUNT(*) FROM inbox_seen_files WHERE status = 'failed'",
            [],
            |row| row.get(0),
        )?;
        Ok(InboxCounts {
            pending: pending.max(0) as u32,
            imported: imported.max(0) as u32,
            failed: failed.max(0) as u32,
        })
    }

    fn row_to_seen_file(row: &rusqlite::Row<'_>) -> rusqlite::Result<InboxSeenFile> {
        Ok(InboxSeenFile {
            id: row.get(0)?,
            source_kind: row.get(1)?,
            source_path: row.get(2)?,
            file_path: row.get(3)?,
            file_name: row.get(4)?,
            file_size: row.get(5)?,
            mtime_ms: row.get(6)?,
            sha256: row.get(7)?,
            status: row.get(8)?,
            record_id: row.get(9)?,
            error_message: row.get(10)?,
            seen_at: row.get(11)?,
            updated_at: row.get(12)?,
        })
    }

    /* --------------------------------- hotwords --------------------------------- */

    pub fn list_hotwords(&self) -> AppResult<Vec<Hotword>> {
        let connection = self.connect()?;
        let mut statement = connection
            .prepare("SELECT id, term, note, created_at FROM hotwords ORDER BY created_at")?;
        let rows = statement
            .query_map([], |row| {
                Ok(Hotword {
                    id: row.get(0)?,
                    term: row.get(1)?,
                    note: row.get(2)?,
                    created_at: row.get(3)?,
                })
            })?
            .collect::<Result<Vec<_>, rusqlite::Error>>()?;
        Ok(rows)
    }

    /// 拼接为 Whisper initial_prompt 使用的热词前缀，限制在 160 个字符内。
    pub fn hotwords_prompt(&self) -> AppResult<String> {
        let terms: Vec<String> = self
            .list_hotwords()?
            .into_iter()
            .map(|hotword| hotword.term)
            .collect();
        if terms.is_empty() {
            return Ok(String::new());
        }
        let mut prompt = String::from("术语表：");
        for term in terms {
            if prompt.chars().count() + term.chars().count() + 1 > 160 {
                break;
            }
            prompt.push_str(&term);
            prompt.push('、');
        }
        Ok(prompt.trim_end_matches('、').to_owned())
    }

    pub fn add_hotword(&self, term: &str, note: &str) -> AppResult<Hotword> {
        let term = term.trim();
        if term.is_empty() || term.chars().count() > 40 {
            return Err(crate::error::AppError::Invalid("热词无效".to_owned()));
        }
        let hotword = Hotword {
            id: Uuid::new_v4().to_string(),
            term: term.to_owned(),
            note: note.trim().to_owned(),
            created_at: Utc::now().to_rfc3339(),
        };
        self.connect()?.execute(
            "INSERT INTO hotwords (id, term, note, created_at) VALUES (?1, ?2, ?3, ?4)              ON CONFLICT(term) DO UPDATE SET note = excluded.note",
            params![hotword.id, hotword.term, hotword.note, hotword.created_at],
        )?;
        Ok(hotword)
    }

    pub fn remove_hotword(&self, id: &str) -> AppResult<()> {
        self.connect()?
            .execute("DELETE FROM hotwords WHERE id = ?1", params![id])?;
        Ok(())
    }

    /* ------------------------------ action dashboard ------------------------------ */

    pub fn list_action_items_detailed(&self) -> AppResult<Vec<ActionDashboardItem>> {
        let connection = self.connect()?;
        let mut statement = connection.prepare(
            "SELECT items.id, items.record_id, records.title, items.project_id, projects.name, items.title, items.owner_text, items.due_text, items.status, items.source_segment_id, records.imported_at              FROM action_items AS items              JOIN records ON records.id = items.record_id              LEFT JOIN projects ON projects.id = items.project_id              ORDER BY CASE items.status WHEN 'open' THEN 0 ELSE 1 END, records.imported_at DESC",
        )?;
        let rows = statement
            .query_map([], |row| {
                Ok(ActionDashboardItem {
                    id: row.get(0)?,
                    record_id: row.get(1)?,
                    record_title: row.get(2)?,
                    project_id: row.get(3)?,
                    project_name: row.get(4)?,
                    title: row.get(5)?,
                    owner_text: row.get(6)?,
                    due_text: row.get(7)?,
                    status: row.get(8)?,
                    source_segment_id: row.get(9)?,
                    imported_at: row.get(10)?,
                })
            })?
            .collect::<Result<Vec<_>, rusqlite::Error>>()?;
        Ok(rows)
    }

    pub fn set_action_item_status(&self, id: &str, status: &str) -> AppResult<()> {
        if !matches!(status, "open" | "done") {
            return Err(crate::error::AppError::Invalid("行动项状态无效".to_owned()));
        }
        let changed = self.connect()?.execute(
            "UPDATE action_items SET status = ?2 WHERE id = ?1",
            params![id, status],
        )?;
        if changed == 0 {
            return Err(crate::error::AppError::NotFound("行动项不存在".to_owned()));
        }
        Ok(())
    }

    /// 从每条记录的最新分析 JSON 中提取未解决问题，供仪表盘聚合。
    pub fn list_open_questions(&self, limit: u64) -> AppResult<Vec<OpenQuestionItem>> {
        let connection = self.connect()?;
        let mut statement = connection.prepare(
            "SELECT records.id, records.title, records.imported_at, analyses.content_json              FROM records JOIN analyses ON analyses.id = (                 SELECT a2.id FROM analyses AS a2 WHERE a2.record_id = records.id ORDER BY a2.created_at DESC LIMIT 1             )              ORDER BY records.imported_at DESC LIMIT ?1",
        )?;
        let rows = statement
            .query_map(params![limit as i64], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })?
            .collect::<Result<Vec<_>, rusqlite::Error>>()?;
        let mut items = Vec::new();
        for (record_id, record_title, imported_at, content_json) in rows {
            let Ok(draft) = serde_json::from_str::<crate::analysis::AnalysisDraft>(&content_json)
            else {
                continue;
            };
            for question in draft.open_questions {
                items.push(OpenQuestionItem {
                    text: question.text,
                    citation_segment_ids: question.citation_segment_ids.clone(),
                    record_id: record_id.clone(),
                    record_title: record_title.clone(),
                    imported_at: imported_at.clone(),
                });
                if items.len() >= limit as usize {
                    break;
                }
            }
            if items.len() >= limit as usize {
                break;
            }
        }
        Ok(items)
    }

    /* --------------------------- transcript AI correction --------------------------- */

    /// 将 LLM 校对后的文本写入 normalized 层（edited 层保留给用户手改，原始层不动）。
    pub fn set_segment_normalized_texts(
        &self,
        record_id: &str,
        corrections: &[(String, String)],
    ) -> AppResult<u32> {
        let mut connection = self.connect()?;
        let transaction = connection.transaction()?;
        let mut changed = 0_u32;
        for (segment_id, text) in corrections {
            let updated = transaction.execute(
                "UPDATE transcript_segments SET normalized_text = ?2, normalization_version = 'llm-corrected-v1', updated_at = ?4 WHERE id = ?1 AND record_id = ?3",
                params![segment_id, text, record_id, Utc::now().to_rfc3339()],
            )?;
            changed += updated.max(0) as u32;
        }
        // 逐字稿文本变了：分析结论引用的原文可能对不上，知识索引也要重建。
        if changed > 0 {
            Self::invalidate_record_knowledge(&transaction, record_id, true)?;
        }
        transaction.commit()?;
        Ok(changed)
    }
    /* ------------------------------ v0.4.0：AI 伙伴 / 产出文件夹 ------------------------------ */

    pub fn latest_dock_chat(&self) -> AppResult<Option<DockChat>> {
        let connection = self.connect()?;
        let mut statement = connection.prepare(
            "SELECT id, title, engine, created_at, updated_at FROM dock_chats ORDER BY updated_at DESC LIMIT 1",
        )?;
        let mut rows = statement.query([])?;
        Ok(match rows.next()? {
            Some(row) => Some(Self::row_to_dock_chat(row)?),
            None => None,
        })
    }

    pub fn create_dock_chat(&self, engine: &str, title: &str) -> AppResult<DockChat> {
        let chat = DockChat {
            id: Uuid::new_v4().to_string(),
            title: title.to_owned(),
            engine: engine.to_owned(),
            created_at: Utc::now().to_rfc3339(),
            updated_at: Utc::now().to_rfc3339(),
        };
        self.connect()?.execute(
            "INSERT INTO dock_chats (id, title, engine, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![chat.id, chat.title, chat.engine, chat.created_at, chat.updated_at],
        )?;
        Ok(chat)
    }

    pub fn touch_dock_chat(&self, chat_id: &str, title: Option<&str>) -> AppResult<()> {
        self.connect()?.execute(
            "UPDATE dock_chats SET updated_at = ?2, title = COALESCE(?3, title) WHERE id = ?1",
            params![chat_id, Utc::now().to_rfc3339(), title],
        )?;
        Ok(())
    }

    pub fn append_dock_message(
        &self,
        chat_id: &str,
        role: &str,
        content: &str,
        mode: &str,
    ) -> AppResult<DockMessage> {
        let message = DockMessage {
            id: Uuid::new_v4().to_string(),
            chat_id: chat_id.to_owned(),
            role: role.to_owned(),
            content: content.to_owned(),
            mode: mode.to_owned(),
            created_at: Utc::now().to_rfc3339(),
        };
        self.connect()?.execute(
            "INSERT INTO dock_messages (id, chat_id, role, content, mode, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![message.id, message.chat_id, message.role, message.content, message.mode, message.created_at],
        )?;
        Ok(message)
    }

    /// 最近若干条会话消息（旧→新），作为对话上下文与界面回放。
    pub fn list_dock_messages(&self, chat_id: &str, limit: u32) -> AppResult<Vec<DockMessage>> {
        let connection = self.connect()?;
        let mut statement = connection.prepare(
            "SELECT id, chat_id, role, content, mode, created_at FROM (\
                 SELECT dock_messages.rowid AS rid, dock_messages.* FROM dock_messages WHERE chat_id = ?1 ORDER BY created_at DESC, rid DESC LIMIT ?2\
             ) ORDER BY created_at ASC, rid ASC",
        )?;
        let rows = statement
            .query_map(params![chat_id, limit], |row| {
                Ok(DockMessage {
                    id: row.get(0)?,
                    chat_id: row.get(1)?,
                    role: row.get(2)?,
                    content: row.get(3)?,
                    mode: row.get(4)?,
                    created_at: row.get(5)?,
                })
            })?
            .collect::<Result<Vec<_>, rusqlite::Error>>()?;
        Ok(rows)
    }

    pub fn clear_dock_chats(&self) -> AppResult<()> {
        self.connect()?.execute("DELETE FROM dock_chats", [])?;
        Ok(())
    }

    fn row_to_dock_chat(row: &rusqlite::Row<'_>) -> rusqlite::Result<DockChat> {
        Ok(DockChat {
            id: row.get(0)?,
            title: row.get(1)?,
            engine: row.get(2)?,
            created_at: row.get(3)?,
            updated_at: row.get(4)?,
        })
    }
    /* ------------------------------ v0.5.0：说话人 ------------------------------ */

    pub fn list_record_speakers(&self, record_id: &str) -> AppResult<Vec<SpeakerSummary>> {
        let connection = self.connect()?;
        let mut statement = connection.prepare(
            "SELECT segments.speaker_label, COUNT(*) FROM transcript_segments AS segments \
             WHERE segments.transcript_version_id = ( \
               SELECT versions.id FROM transcript_versions AS versions \
               WHERE versions.record_id = ?1 ORDER BY versions.created_at DESC LIMIT 1 ) \
             GROUP BY segments.speaker_label ORDER BY COUNT(*) DESC",
        )?;
        let rows = statement
            .query_map(params![record_id], |row| {
                Ok(SpeakerSummary {
                    label: row.get(0)?,
                    segment_count: row.get::<_, i64>(1)? as u32,
                })
            })?
            .collect::<Result<Vec<_>, rusqlite::Error>>()?;
        Ok(rows)
    }

    pub fn rename_record_speaker(
        &self,
        record_id: &str,
        from_label: &str,
        to_label: &str,
    ) -> AppResult<u32> {
        let to_label = to_label.trim();
        if to_label.is_empty() || to_label.chars().count() > 24 {
            return Err(crate::error::AppError::Invalid("说话人名称无效".to_owned()));
        }
        let changed = self.connect()?.execute(
            "UPDATE transcript_segments SET speaker_label = ?3 \
             WHERE record_id = ?1 AND speaker_label = ?2",
            params![record_id, from_label, to_label],
        )?;
        Ok(changed.max(0) as u32)
    }
}
