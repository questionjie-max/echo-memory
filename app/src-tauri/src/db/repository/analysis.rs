use super::LibraryRepository;
use crate::analysis::AnalysisDraft;
use crate::error::{AppError, AppResult};
use crate::types::{AnalysisTemplate, StoredAnalysis, TranscriptVersion};
use chrono::Utc;
use rusqlite::{params, OptionalExtension, Row};
use uuid::Uuid;

impl LibraryRepository {
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
}
