use super::LibraryRepository;
use crate::error::AppResult;
use crate::types::{McpAccessLog, McpStatus};
use chrono::Utc;
use rusqlite::params;
use uuid::Uuid;

impl LibraryRepository {
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
}
