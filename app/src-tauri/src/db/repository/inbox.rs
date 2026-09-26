use super::LibraryRepository;
use crate::error::AppResult;
use crate::types::{InboxCounts, InboxSeenFile, InboxWatchFolder};
use chrono::Utc;
use rusqlite::params;
use uuid::Uuid;

impl LibraryRepository {
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
            .query_map([], Self::row_to_seen_file)?
            .collect::<Result<Vec<_>, rusqlite::Error>>()?;
        Ok(rows)
    }

    pub fn recent_seen_files(&self, limit: u32) -> AppResult<Vec<InboxSeenFile>> {
        let connection = self.connect()?;
        let mut statement = connection.prepare(
            "SELECT id, source_kind, source_path, file_path, file_name, file_size, mtime_ms, sha256, status, record_id, error_message, seen_at, updated_at              FROM inbox_seen_files ORDER BY seen_at DESC LIMIT ?1",
        )?;
        let rows = statement
            .query_map(params![limit], Self::row_to_seen_file)?
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
}
