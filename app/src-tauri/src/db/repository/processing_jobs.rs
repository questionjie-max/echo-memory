use super::LibraryRepository;
use crate::error::{AppError, AppResult};
use crate::types::ProcessingJob;
use chrono::Utc;
use rusqlite::{params, OptionalExtension, Row};
use uuid::Uuid;

impl LibraryRepository {
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
}
