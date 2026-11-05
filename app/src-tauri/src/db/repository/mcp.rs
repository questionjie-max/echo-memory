use super::LibraryRepository;
use crate::error::{AppError, AppResult};
use crate::minutes::MinutesEvidence;
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

    /// v1.1 块4：get_my_context 的「会议纪要已确认条目」段。
    /// 只读 minutes_items 里 status='confirmed' 的条目，并附溯源坐标
    /// （记录标题、证据时间窗、逐字引文）。未确认条目永远不出现在这里——
    /// 这是 MCP 侧唯一的纪要记忆入口，未确认内容对 Agent 完全不可见。
    pub fn confirmed_minutes_memory_entries(
        &self,
        limit: usize,
    ) -> AppResult<Vec<ConfirmedMinutesMemoryEntry>> {
        let rows = self.list_confirmed_minutes_items(limit)?;
        let mut entries = Vec::with_capacity(rows.len());
        for row in rows {
            // 单条证据损坏只跳过该条（不拖垮整个 get_my_context）：
            // fail-closed 的对象是「未确认不可见」，损坏条目不输出即可。
            let Ok(evidence) = serde_json::from_str::<MinutesEvidence>(&row.evidence_json) else {
                continue;
            };
            let record_title = self
                .get_record(&row.record_id)
                .map(|record| record.title)
                .unwrap_or_default();
            entries.push(ConfirmedMinutesMemoryEntry {
                item_id: row.id.clone(),
                kind: row.kind.clone(),
                text: row.text.clone(),
                owner: row.owner_text.clone(),
                record_id: row.record_id.clone(),
                record_title,
                start_ms: evidence.start_ms,
                end_ms: evidence.end_ms,
                quote_text: evidence.quote_text,
                confirmed_at: row.updated_at,
            });
        }
        Ok(entries)
    }
}

/// get_my_context 返回的已确认纪要条目（含溯源坐标）。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfirmedMinutesMemoryEntry {
    pub item_id: String,
    pub kind: String,
    pub text: String,
    pub owner: Option<String>,
    pub record_id: String,
    pub record_title: String,
    pub start_ms: i64,
    pub end_ms: i64,
    pub quote_text: String,
    pub confirmed_at: String,
}
