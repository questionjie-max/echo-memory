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

    /// 块5：按时间范围取已确认纪要条目（新 MCP 工具 list_confirmed_minutes 的数据层）。
    /// 只返回 confirmed 且记录未归档的条目（与 get_my_context 同一门），每条带溯源坐标
    /// （记录标题、证据时间窗、逐字引文、确认时间）。时间范围按记录的 imported_at 过滤。
    pub fn list_confirmed_minutes_in_range(
        &self,
        since: Option<&str>,
        until: Option<&str>,
        project_id: Option<&str>,
        limit: usize,
    ) -> AppResult<Vec<ConfirmedMinutesMemoryEntry>> {
        let connection = self.connect()?;
        let mut statement = connection
            .prepare(
                "SELECT minutes_items.id, minutes_items.record_id, minutes_items.kind, minutes_items.text, \
                 minutes_items.owner_text, minutes_items.evidence_json, minutes_items.updated_at, \
                 records.title, records.imported_at, projects.name \
                 FROM minutes_items \
                 JOIN records ON records.id = minutes_items.record_id \
                 LEFT JOIN projects ON projects.id = records.project_id \
                 WHERE minutes_items.status = 'confirmed' AND records.archived_at IS NULL \
                 AND (?1 IS NULL OR records.imported_at >= ?1) \
                 AND (?2 IS NULL OR records.imported_at <= ?2) \
                 AND (?3 IS NULL OR records.project_id = ?3) \
                 ORDER BY records.imported_at DESC LIMIT ?4",
            )
            .map_err(|error| AppError::Invalid(format!("读取已确认纪要失败：{error}")))?;
        let rows = statement
            .query_map(
                params![since, until, project_id, limit as i64],
                |row| {
                    let evidence: MinutesEvidence =
                        serde_json::from_str(&row.get::<_, String>(5)?)
                            .map_err(|_| rusqlite::Error::InvalidColumnType(
                                5,
                                "evidence_json".into(),
                                rusqlite::types::Type::Text,
                            ))?;
                    Ok(ConfirmedMinutesMemoryEntry {
                        item_id: row.get(0)?,
                        kind: row.get(2)?,
                        text: row.get(3)?,
                        owner: row.get(4)?,
                        record_id: row.get(1)?,
                        record_title: row.get(7)?,
                        start_ms: evidence.start_ms,
                        end_ms: evidence.end_ms,
                        quote_text: evidence.quote_text,
                        confirmed_at: row.get(6)?,
                        record_imported_at: row.get(8)?,
                        project_name: row.get(9)?,
                    })
                },
            )
            .map_err(|error| AppError::Invalid(format!("读取已确认纪要失败：{error}")))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| AppError::Invalid(format!("读取已确认纪要失败：{error}")))
    }

    /// 块4 导入确认门：未确认的 document/chat 记录对 MCP 完全不可见。
    /// 门语义集中在 lib（可测试），二进制只做 JSON-RPC 壳。
    fn ensure_import_confirmed(&self, record_id: &str) -> AppResult<()> {
        if self.is_unconfirmed_import(record_id)? {
            return Err(crate::error::AppError::Invalid(
                "该记录是尚未确认的导入内容，对 AI 助手不可见；请在回声记忆里确认入记忆后再试。".to_owned(),
            ));
        }
        Ok(())
    }

    fn unconfirmed_import_ids(&self) -> AppResult<std::collections::HashSet<String>> {
        Ok(self
            .list_unconfirmed_import_ids()?
            .into_iter()
            .collect())
    }

    /// MCP search_records：应用内搜索的同一召回，叠加导入确认门过滤。
    pub fn mcp_search(&self, query: &str, project_id: Option<&str>, limit: usize) -> AppResult<Vec<crate::types::SearchResult>> {
        let unconfirmed = self.unconfirmed_import_ids()?;
        Ok(self
            .search(query, project_id, false, limit)?
            .into_iter()
            .filter(|item| !unconfirmed.contains(&item.record_id))
            .collect())
    }

    /// MCP get_record：元数据 + 最新逐字稿前 100 段；未确认导入拒绝。
    pub fn mcp_get_record(
        &self,
        record_id: &str,
    ) -> AppResult<(crate::types::RecordBrief, Vec<crate::types::TranscriptSegment>)> {
        self.ensure_import_confirmed(record_id)?;
        let record = self.get_record(record_id)?;
        let transcript = self.list_transcript_segments(record_id)?;
        Ok((record, transcript.into_iter().take(100).collect()))
    }

    /// MCP get_transcript：时间窗过滤的逐字稿段；未确认导入拒绝。
    pub fn mcp_get_transcript(
        &self,
        record_id: &str,
        start_ms: i64,
        end_ms: i64,
    ) -> AppResult<Vec<crate::types::TranscriptSegment>> {
        self.ensure_import_confirmed(record_id)?;
        Ok(self
            .list_transcript_segments(record_id)?
            .into_iter()
            .filter(|segment| segment.end_ms >= start_ms && segment.start_ms <= end_ms)
            .take(100)
            .collect())
    }

    /// MCP get_project_context：项目内记录列表 + 决策 + 待办，全部过滤未确认导入。
    pub fn mcp_project_context(
        &self,
        project_id: &str,
    ) -> AppResult<(
        Vec<crate::types::RecordBrief>,
        Vec<serde_json::Value>,
        Vec<crate::types::ActionItem>,
    )> {
        let unconfirmed = self.unconfirmed_import_ids()?;
        let records = self
            .list_records(Some(project_id), false)?
            .into_iter()
            .filter(|record| !unconfirmed.contains(&record.id))
            .take(20)
            .collect();
        let decisions = self
            .list_decisions(Some(project_id), 20)?
            .into_iter()
            .filter(|item| {
                item.get("recordId")
                    .and_then(serde_json::Value::as_str)
                    .map(|id| !unconfirmed.contains(id))
                    .unwrap_or(true)
            })
            .collect();
        let action_items = self
            .list_action_items(Some(project_id), None)?
            .into_iter()
            .filter(|item| !unconfirmed.contains(&item.record_id))
            .collect();
        Ok((records, decisions, action_items))
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
            authorized_scope: "全部记录的标题、逐字稿与文档原文（只读）；记忆条目仅限已确认内容".to_owned(),
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
                record_imported_at: None,
                project_name: None,
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
    /// 块5：记录的发生/导入时间与所属知识库（时间范围检索的过滤基准与展示字段）。
    pub record_imported_at: Option<String>,
    pub project_name: Option<String>,
}

#[cfg(test)]
mod scope_copy_tests {
    use super::*;

    /// 块3/F1 契约测试：MCP 授权口径文案必须如实讲两层——
    /// 原文层（记录标题/逐字稿/文档原文，只读、本就可读）与
    /// 记忆条目层（仅 confirmed）。缺任何一半都会误导用户。
    #[test]
    fn authorized_scope_copy_states_both_layers() {
        let root = std::env::temp_dir().join(format!("echo-mcp-scope-{}", uuid::Uuid::new_v4()));
        let library = crate::library::ManagedLibrary::open(root).unwrap();
        let status = library.repository().mcp_status().unwrap();
        assert!(
            status.authorized_scope.contains("（只读）"),
            "口径文案应声明只读：{}",
            status.authorized_scope
        );
        assert!(
            status.authorized_scope.contains("逐字稿") || status.authorized_scope.contains("原文"),
            "口径文案应讲明原文层对 MCP 可读：{}",
            status.authorized_scope
        );
        assert!(
            status.authorized_scope.contains("已确认"),
            "口径文案应讲明记忆条目仅限已确认：{}",
            status.authorized_scope
        );
    }
}
