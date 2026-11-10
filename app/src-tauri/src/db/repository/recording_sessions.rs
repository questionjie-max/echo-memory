use super::LibraryRepository;
use crate::error::{AppError, AppResult};
use chrono::Utc;
use rusqlite::{params, OptionalExtension};

#[derive(Debug, Clone)]
pub struct RecordingSessionRow {
    pub id: String,
    pub status: String,
    pub temp_path: String,
    pub title: String,
    pub project_id: Option<String>,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub duration_ms: i64,
    pub paused_ms: i64,
    pub input_device: Option<String>,
    pub sample_rate: i64,
    pub channels: i64,
    pub written_bytes: i64,
    pub record_id: Option<String>,
    pub last_error: Option<String>,
    pub updated_at: String,
    /// 块8 双路：主会话指向副会话（系统声）。
    pub paired_session_id: Option<String>,
    /// 块8 双路：两路启动实测偏移（毫秒）。
    pub alignment_offset_ms: Option<i64>,
}

impl LibraryRepository {
    pub fn create_recording_session(
        &self,
        id: &str,
        temp_path: &str,
        title: &str,
        project_id: Option<&str>,
        started_at: &str,
    ) -> AppResult<RecordingSessionRow> {
        if id.trim().is_empty() || temp_path.trim().is_empty() || title.trim().is_empty() {
            return Err(AppError::Invalid("录音会话参数不能为空".to_owned()));
        }
        let session = RecordingSessionRow {
            id: id.to_owned(),
            status: "starting".to_owned(),
            temp_path: temp_path.to_owned(),
            title: title.trim().to_owned(),
            project_id: project_id.map(str::to_owned),
            started_at: started_at.to_owned(),
            ended_at: None,
            duration_ms: 0,
            paused_ms: 0,
            input_device: None,
            sample_rate: 48_000,
            channels: 1,
            written_bytes: 0,
            record_id: None,
            last_error: None,
            updated_at: started_at.to_owned(),
            paired_session_id: None,
            alignment_offset_ms: None,
        };
        self.connect()?.execute(
            "INSERT INTO recording_sessions
             (id, status, temp_path, title, project_id, started_at, duration_ms, paused_ms, sample_rate, channels, written_bytes, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, 0, ?7, ?8, 0, ?6)",
            params![
                session.id,
                session.status,
                session.temp_path,
                session.title,
                session.project_id,
                session.started_at,
                session.sample_rate,
                session.channels,
            ],
        )?;
        Ok(session)
    }

    pub fn update_recording_session_status(
        &self,
        id: &str,
        status: &str,
        duration_ms: i64,
        paused_ms: i64,
        written_bytes: i64,
        last_error: Option<&str>,
    ) -> AppResult<()> {
        const ALLOWED: [&str; 8] = [
            "starting",
            "recording",
            "paused",
            "finalizing",
            "completed",
            "failed",
            "recoverable",
            "canceled",
        ];
        if !ALLOWED.contains(&status) || duration_ms < 0 || paused_ms < 0 || written_bytes < 0 {
            return Err(AppError::Invalid("录音会话状态或统计值无效".to_owned()));
        }
        let now = Utc::now().to_rfc3339();
        let ended_at = matches!(status, "finalizing" | "completed" | "failed" | "canceled")
            .then_some(now.clone());
        let affected = self.connect()?.execute(
            "UPDATE recording_sessions
             SET status = ?2, duration_ms = ?3, paused_ms = ?4, written_bytes = ?5,
                 last_error = ?6, ended_at = COALESCE(?7, ended_at), updated_at = ?8
             WHERE id = ?1",
            params![
                id,
                status,
                duration_ms,
                paused_ms,
                written_bytes,
                last_error,
                ended_at,
                now
            ],
        )?;
        if affected == 0 {
            return Err(AppError::NotFound(format!("recording session {id}")));
        }
        Ok(())
    }

    pub fn complete_recording_session(&self, id: &str, record_id: &str) -> AppResult<()> {
        let now = Utc::now().to_rfc3339();
        let affected = self.connect()?.execute(
            "UPDATE recording_sessions
             SET status = 'completed', record_id = ?2, ended_at = COALESCE(ended_at, ?3),
                 last_error = NULL, updated_at = ?3 WHERE id = ?1",
            params![id, record_id, now],
        )?;
        if affected == 0 {
            return Err(AppError::NotFound(format!("recording session {id}")));
        }
        Ok(())
    }

    pub fn get_recording_session(&self, id: &str) -> AppResult<RecordingSessionRow> {
        self.connect()?
            .query_row(
                "SELECT id, status, temp_path, title, project_id, started_at, ended_at, duration_ms, paused_ms,
                        input_device, sample_rate, channels, written_bytes, record_id, last_error, updated_at,
                        paired_session_id, alignment_offset_ms
                 FROM recording_sessions WHERE id = ?1",
                params![id],
                map_recording_session,
            )
            .optional()?
            .ok_or_else(|| AppError::NotFound(format!("recording session {id}")))
    }

    /* ---------------- 双路录音（块8） ---------------- */

    /// 主会话（麦克风）登记副会话（系统声）。
    pub fn set_paired_session(
        &self,
        mic_session_id: &str,
        system_session_id: &str,
    ) -> AppResult<()> {
        self.connect()?.execute(
            "UPDATE recording_sessions SET paired_session_id = ?2, updated_at = ?3 WHERE id = ?1",
            params![mic_session_id, system_session_id, Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    /// 记录两路启动的实测偏移（对齐误差可见、可查）。
    pub fn set_alignment_offset(&self, session_id: &str, offset_ms: i64) -> AppResult<()> {
        self.connect()?.execute(
            "UPDATE recording_sessions SET alignment_offset_ms = ?2, updated_at = ?3 WHERE id = ?1",
            params![session_id, offset_ms, Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    /// 块7 导出包 manifest 用：查某条记录的双路对齐偏移（无则 None）。
    pub fn alignment_offset_for_record(&self, record_id: &str) -> AppResult<Option<i64>> {
        let connection = self.connect()?;
        let mut statement = connection.prepare(
            "SELECT alignment_offset_ms FROM recording_sessions \
             WHERE record_id = ?1 AND alignment_offset_ms IS NOT NULL \
             ORDER BY updated_at DESC LIMIT 1",
        )?;
        let mut rows = statement.query(params![record_id])?;
        if let Some(row) = rows.next()? {
            Ok(row.get(0)?)
        } else {
            Ok(None)
        }
    }

    /// 副会话收尾：canceled + 指向合并后的记录（0021 的 status CHECK 不含
    /// 新枚举，不动表结构；canceled + record_id 即「已并入」的对账语义）。
    pub fn finish_paired_session(
        &self,
        system_session_id: &str,
        record_id: &str,
        duration_ms: i64,
        written_bytes: i64,
    ) -> AppResult<()> {
        let affected = self.connect()?.execute(
            "UPDATE recording_sessions SET status = 'canceled', record_id = ?2, duration_ms = ?3,
             written_bytes = ?4, updated_at = ?5 WHERE id = ?1",
            params![
                system_session_id,
                record_id,
                duration_ms,
                written_bytes,
                Utc::now().to_rfc3339()
            ],
        )?;
        if affected == 0 {
            return Err(AppError::NotFound(format!(
                "recording session {system_session_id}"
            )));
        }
        Ok(())
    }

    /// 应用重新启动后，进程内的原生录音器已经不存在；活动会话只能交给用户恢复或放弃。
    pub fn recover_interrupted_recording_sessions(&self) -> AppResult<usize> {
        let now = Utc::now().to_rfc3339();
        let changed = self.connect()?.execute(
            "UPDATE recording_sessions
             SET status = 'recoverable', last_error = '上次应用退出时录音未封存，请恢复或放弃', updated_at = ?1
             WHERE status IN ('starting', 'recording', 'paused', 'finalizing')",
            params![now],
        )?;
        Ok(changed)
    }

    pub fn list_recoverable_recording_sessions(&self) -> AppResult<Vec<RecordingSessionRow>> {
        let connection = self.connect()?;
        let mut statement = connection.prepare(
            "SELECT id, status, temp_path, title, project_id, started_at, ended_at, duration_ms, paused_ms,
                    input_device, sample_rate, channels, written_bytes, record_id, last_error, updated_at,
                    paired_session_id, alignment_offset_ms
             FROM recording_sessions WHERE status IN ('starting', 'recording', 'paused', 'finalizing', 'recoverable', 'failed')
             ORDER BY updated_at ASC",
        )?;
        let rows = statement
            .query_map([], map_recording_session)?
            .collect::<Result<Vec<_>, rusqlite::Error>>()?;
        Ok(rows)
    }
}

fn map_recording_session(row: &rusqlite::Row<'_>) -> rusqlite::Result<RecordingSessionRow> {
    Ok(RecordingSessionRow {
        id: row.get(0)?,
        status: row.get(1)?,
        temp_path: row.get(2)?,
        title: row.get(3)?,
        project_id: row.get(4)?,
        started_at: row.get(5)?,
        ended_at: row.get(6)?,
        duration_ms: row.get(7)?,
        paused_ms: row.get(8)?,
        input_device: row.get(9)?,
        sample_rate: row.get(10)?,
        channels: row.get(11)?,
        written_bytes: row.get(12)?,
        record_id: row.get(13)?,
        last_error: row.get(14)?,
        updated_at: row.get(15)?,
        paired_session_id: row.get(16)?,
        alignment_offset_ms: row.get(17)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn repository() -> LibraryRepository {
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("echo-recording-session-{id}"));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        LibraryRepository::new(root.join("memory.db")).unwrap()
    }

    #[test]
    fn interrupted_session_becomes_recoverable_and_keeps_metadata() {
        let repository = repository();
        repository
            .create_recording_session(
                "session-1",
                "recordings/inflight/session-1/capture.wav",
                "产品讨论",
                None,
                "2026-10-16T09:00:00Z",
            )
            .unwrap();
        repository
            .update_recording_session_status("session-1", "recording", 3_000, 500, 12_345, None)
            .unwrap();

        assert_eq!(
            repository.recover_interrupted_recording_sessions().unwrap(),
            1
        );
        let session = repository.get_recording_session("session-1").unwrap();
        assert_eq!(session.status, "recoverable");
        assert_eq!(session.title, "产品讨论");
        assert_eq!(session.duration_ms, 3_000);
        assert_eq!(session.paused_ms, 500);
        assert_eq!(session.written_bytes, 12_345);
        assert!(session.last_error.is_some());
    }

    #[test]
    fn completed_session_points_to_record() {
        let repository = repository();
        repository
            .create_record(
                "父记录",
                None,
                std::path::Path::new("audio/parent.wav"),
                "parent-hash",
                1_000,
            )
            .unwrap();
        repository
            .create_recording_session(
                "session-2",
                "recordings/inflight/session-2/capture.wav",
                "语音备忘",
                None,
                "2026-10-16T09:00:00Z",
            )
            .unwrap();
        let parent_id = repository
            .list_records(None, false)
            .unwrap()
            .into_iter()
            .find(|record| record.title == "父记录")
            .unwrap()
            .id;
        repository
            .complete_recording_session("session-2", &parent_id)
            .unwrap();
        let session = repository.get_recording_session("session-2").unwrap();
        assert_eq!(session.status, "completed");
        assert_eq!(session.record_id.as_deref(), Some(parent_id.as_str()));
    }

    #[test]
    fn invalid_status_and_negative_stats_are_rejected() {
        let repository = repository();
        let result = repository.update_recording_session_status("missing", "bogus", -1, 0, 0, None);
        assert!(result.is_err());
        assert!(PathBuf::from("recordings/inflight").is_relative());
    }
}
