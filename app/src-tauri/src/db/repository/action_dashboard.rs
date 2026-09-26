use super::LibraryRepository;
use crate::error::AppResult;
use crate::types::{ActionDashboardItem, OpenQuestionItem};
use rusqlite::params;

impl LibraryRepository {
    pub fn list_action_items_detailed(&self) -> AppResult<Vec<ActionDashboardItem>> {
        let connection = self.connect()?;
        let mut statement = connection.prepare(
            "SELECT items.id, items.record_id, records.title, items.project_id, projects.name, items.title, items.owner_text, items.due_text, items.status, items.source_segment_id, records.imported_at              FROM action_items AS items              JOIN records ON records.id = items.record_id              LEFT JOIN projects ON projects.id = items.project_id              WHERE records.archived_at IS NULL              ORDER BY CASE items.status WHEN 'open' THEN 0 ELSE 1 END, records.imported_at DESC",
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
            "SELECT records.id, records.title, records.imported_at, analyses.content_json              FROM records JOIN analyses ON analyses.id = (                 SELECT a2.id FROM analyses AS a2 WHERE a2.record_id = records.id ORDER BY a2.created_at DESC LIMIT 1             )              WHERE records.archived_at IS NULL              ORDER BY records.imported_at DESC LIMIT ?1",
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
}
