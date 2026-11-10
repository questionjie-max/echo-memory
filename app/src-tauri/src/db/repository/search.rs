use super::LibraryRepository;
use crate::error::AppResult;
use crate::types::SearchResult;
use rusqlite::params_from_iter;
use rusqlite::types::Value as SqlValue;

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
        // 术语变体扩展（块7）：查询词精确命中某个识别变体时，把规范写法与
        // 同词其它变体 OR 进搜索——错写也能命中。无扩展时与原先行为完全一致。
        // 有扩展时走 LIKE 路径（别名长度不受 trigram 3 字下限约束，
        // 如 2 字人名「张三」）；变体查询是低频操作，全表 LIKE 可接受。
        let expansions = self.term_variant_expansions(query)?;
        let extended = !expansions.is_empty();
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
        let fts_query = Self::build_fts_query(query);
        let use_fts = !extended && query.chars().count() >= 3 && fts_query.is_some();

        // LIKE 短查询路径：扩展时为每个别名各加一组 LIKE，占位符动态生成
        // （SQLite 占位符从 ?1 起：?1 是原查询，?2..=n+1 是扩展别名）。
        let short_sql = if extended {
            let mut like_groups = Vec::new();
            for index in 1..=expansions.len() + 1 {
                like_groups.push(format!(
                    "(search.title LIKE '%' || ?{index} || '%' OR search.body LIKE '%' || ?{index} || '%')"
                ));
            }
            format!(
                "{} WHERE records.archived_at IS NULL AND ({}) \
                 AND (?{} IS NULL OR search.project_id = ?{}) AND (?{} = 0 OR search.project_id IS NULL) \
                 ORDER BY records.updated_at DESC LIMIT ?{}",
                common_select.replace("{snippet}", "search.body"),
                like_groups.join(" OR "),
                expansions.len() + 2,
                expansions.len() + 2,
                expansions.len() + 3,
                expansions.len() + 4,
            )
        } else {
            format!(
                "{} WHERE records.archived_at IS NULL AND (search.title LIKE '%' || ?1 || '%' OR search.body LIKE '%' || ?1 || '%') \
                 AND (?2 IS NULL OR search.project_id = ?2) AND (?3 = 0 OR search.project_id IS NULL) \
                 ORDER BY records.updated_at DESC LIMIT ?4",
                common_select.replace("{snippet}", "search.body")
            )
        };

        let use_extended_like = extended;
        let sql_to_use = if use_fts { &sql } else { &short_sql };
        let mut statement = connection.prepare(sql_to_use)?;

        let bind_values: Vec<SqlValue> = if use_fts {
            vec![
                SqlValue::from(fts_query.unwrap_or_default()),
                SqlValue::from(project_id.map(str::to_owned)),
                SqlValue::from(i64::from(unfiled_only)),
                SqlValue::from(limit as i64),
            ]
        } else if use_extended_like {
            let mut values = vec![SqlValue::from(query.to_owned())];
            for expansion in &expansions {
                values.push(SqlValue::from(expansion.clone()));
            }
            values.push(SqlValue::from(project_id.map(str::to_owned)));
            values.push(SqlValue::from(i64::from(unfiled_only)));
            values.push(SqlValue::from(limit as i64));
            values
        } else {
            vec![
                SqlValue::from(query.to_owned()),
                SqlValue::from(project_id.map(str::to_owned)),
                SqlValue::from(i64::from(unfiled_only)),
                SqlValue::from(limit as i64),
            ]
        };

        let results = statement
            .query_map(params_from_iter(bind_values), |row| {
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
            })?
            .collect::<Result<Vec<_>, rusqlite::Error>>()?;
        Ok(results)
    }

    pub(super) fn build_fts_query(query: &str) -> Option<String> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::TranscriptSegmentInput;

    fn test_library() -> crate::library::ManagedLibrary {
        let root = std::env::temp_dir().join(format!("echo-search-{}", uuid::Uuid::new_v4()));
        crate::library::ManagedLibrary::open(root).unwrap()
    }

    fn seed_record(library: &crate::library::ManagedLibrary, title: &str, segments: &[&str]) -> String {
        let repository = library.repository();
        let record = repository
            .create_record(
                title,
                None,
                &library.root().join("audio/test.wav"),
                "hash",
                segments.len() as i64 * 1_000,
            )
            .unwrap();
        repository
            .save_transcript_with_metadata(
                &record.id,
                "whisper.cpp",
                "model",
                "zh",
                "enhanced-v2",
                "{}",
                &segments
                    .iter()
                    .enumerate()
                    .map(|(index, text)| TranscriptSegmentInput {
                        start_ms: index as i64 * 1_000,
                        end_ms: (index + 1) as i64 * 1_000,
                        speaker_label: None,
                        original_text: (*text).to_owned(),
                    })
                    .collect::<Vec<_>>(),
            )
            .unwrap();
        record.id
    }

    #[test]
    fn misspelled_variant_still_finds_canonical_transcript() {
        // 块7 验收：识别错的变体来搜，也能命中规范写法的原文。
        let library = test_library();
        let repository = library.repository();
        let record_id = seed_record(
            &library,
            "产品复盘会",
            &["我们把第三季度的产品复盘交给张三负责", "老李负责服务端迁移"],
        );
        repository.add_term_variant("张三", "张叁").unwrap();

        // 错写「张叁」：trigram FTS 路径（3 字）→ 经扩展命中「张三」原文。
        let by_variant = repository.search("张叁负责", None, false, 10).unwrap();
        assert!(
            by_variant.iter().any(|hit| hit.record_id == record_id),
            "错写变体必须命中规范写法的记录"
        );

        // 规范写法照常命中。
        let canonical = repository.search("张三负责", None, false, 10).unwrap();
        assert!(canonical.iter().any(|hit| hit.record_id == record_id));

        // LIKE 短查询路径（<3 字）：变体 2 字也命中。
        repository.remove_term_variant("张叁").unwrap();
        repository.add_term_variant("老李", "老莉").unwrap();
        let short = repository.search("老莉", None, false, 10).unwrap();
        assert!(
            short.iter().any(|hit| hit.record_id == record_id),
            "LIKE 路径的变体扩展必须命中"
        );
        let _ = std::fs::remove_dir_all(library.root());
    }

    #[test]
    fn search_without_variant_expansion_keeps_original_behavior() {
        let library = test_library();
        let repository = library.repository();
        let record_id = seed_record(&library, "周会", &["讨论了发布计划"]);
        let hits = repository.search("发布计划", None, false, 10).unwrap();
        assert!(hits.iter().any(|hit| hit.record_id == record_id));
        // 搜不存在的词：无结果、不报错。
        assert!(repository.search("不存在的词", None, false, 10).unwrap().is_empty());
        // LIKE 非扩展路径（<3 字、无变体）：与扩展前行为一致。
        let short = repository.search("周", None, false, 10).unwrap();
        assert!(short.iter().any(|hit| hit.record_id == record_id));
        let _ = std::fs::remove_dir_all(library.root());
    }

    #[test]
    fn extended_variant_search_respects_project_filter() {
        // 锁住扩展路径的动态占位符编号：project 过滤与 unfiled 在有扩展时
        // 仍然生效（绑定错位时 SQLite 不报错，只能靠断言抓）。
        let library = test_library();
        let repository = library.repository();
        let project = repository.create_project("项目A").unwrap();
        let in_project = seed_record(&library, "项目A会议", &["张三负责产品复盘"]);
        let _out_project = seed_record(&library, "其它会议", &["张三负责产品复盘"]);
        repository
            .update_record_project(&in_project, Some(&project.id))
            .unwrap();
        repository.add_term_variant("张三", "张叁").unwrap();

        let hits = repository.search("张叁", Some(&project.id), false, 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].record_id, in_project);

        // unfiled_only：只出无项目那条（in_project 被过滤）。
        let unfiled_hits = repository.search("张叁", None, true, 10).unwrap();
        assert_eq!(unfiled_hits.len(), 1);
        assert_ne!(unfiled_hits[0].record_id, in_project);
        let _ = std::fs::remove_dir_all(library.root());
    }
}
