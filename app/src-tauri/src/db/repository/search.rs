use super::LibraryRepository;
use crate::error::AppResult;
use crate::types::SearchResult;
use rusqlite::params;

impl LibraryRepository {
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
             "{} WHERE records.archived_at IS NULL AND record_search MATCH ?1 AND (?2 IS NULL OR search.project_id = ?2) \
             AND (?3 = 0 OR search.project_id IS NULL) ORDER BY rank LIMIT ?4",
            common_select.replace(
                "{snippet}",
                "snippet(record_search, 4, '<mark>', '</mark>', '...', 20)"
            )
        );
        let short_sql = format!(
             "{} WHERE records.archived_at IS NULL AND (search.title LIKE '%' || ?1 || '%' OR search.body LIKE '%' || ?1 || '%') \
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
}
