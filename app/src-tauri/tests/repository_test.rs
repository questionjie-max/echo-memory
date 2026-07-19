//! 仓库层集成测试（M1a）：覆盖 projects / records / processing_jobs 全量 CRUD。

use echo_memory_lib::analysis::{AnalysisDraft, AnalysisItemDraft};
use echo_memory_lib::db::repository::LibraryRepository;
use echo_memory_lib::types::{
    KnowledgeChunkInput, KnowledgeIndexStatus, TemplateSection, TranscriptSegmentInput,
};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn database_path() -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let directory = std::env::temp_dir().join("echo_memory_repository_tests");
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join(format!("repo_{n}.db"));
    let _ = std::fs::remove_file(&path);
    path
}

fn cited_draft(segment_id: &str, quote: &str) -> AnalysisDraft {
    AnalysisDraft {
        summary: "这是一段足够完整的测试摘要，用于验证知识记忆生命周期。".into(),
        key_points: vec![AnalysisItemDraft {
            text: "记录包含可复用结论".into(),
            citation_segment_ids: vec![segment_id.into()],
            quote_text: quote.into(),
            start_ms: Some(0),
            end_ms: Some(1_000),
        }],
        decisions: vec![],
        action_items: vec![AnalysisItemDraft {
            text: "跟进结论".into(),
            citation_segment_ids: vec![segment_id.into()],
            quote_text: quote.into(),
            start_ms: Some(0),
            end_ms: Some(1_000),
        }],
        open_questions: vec![],
        custom_sections: vec![],
        quality_warning: None,
    }
}

#[test]
fn project_and_record_round_trip_by_hash() {
    let repo = LibraryRepository::new(database_path()).unwrap();
    let project = repo.create_project("Alpha").unwrap();
    let record = repo
        .create_record(
            "访谈录音",
            Some(&project.id),
            Path::new("audio/2026/07/interview.m4a"),
            "abc123",
            42_000,
        )
        .unwrap();

    assert_eq!(repo.list_projects().unwrap().len(), 1);
    let found = repo.find_record_by_hash("abc123").unwrap().unwrap();
    assert_eq!(found.id, record.id);
    assert_eq!(found.project_id.as_deref(), Some(project.id.as_str()));
}

#[test]
fn transcript_keeps_original_text_when_editing() {
    let repository = LibraryRepository::new(database_path()).unwrap();
    let record = repository
        .create_record(
            "访谈录音",
            None,
            std::path::Path::new("audio/2026/07/interview.m4a"),
            "transcript-hash",
            42_000,
        )
        .unwrap();
    let (_, segments) = repository
        .save_transcript(
            &record.id,
            "whisper.cpp",
            "base",
            &[TranscriptSegmentInput {
                start_ms: 0,
                end_ms: 1_000,
                speaker_label: None,
                original_text: "原始逐字稿".into(),
            }],
        )
        .unwrap();
    let updated = repository
        .update_segment_text(&segments[0].id, Some("用户修订文本"))
        .unwrap();
    assert_eq!(updated.original_text, "原始逐字稿");
    assert_eq!(updated.edited_text.as_deref(), Some("用户修订文本"));
}

#[test]
fn transcript_changes_stale_analysis_and_remove_derived_items() {
    let repository = LibraryRepository::new(database_path()).unwrap();
    let record = repository
        .create_record(
            "知识生命周期",
            None,
            Path::new("audio/stale.wav"),
            "stale-hash",
            1_000,
        )
        .unwrap();
    let (version, segments) = repository
        .save_transcript(
            &record.id,
            "whisper.cpp",
            "small",
            &[TranscriptSegmentInput {
                start_ms: 0,
                end_ms: 1_000,
                speaker_label: None,
                original_text: "确认采用本地知识库".into(),
            }],
        )
        .unwrap();
    repository
        .save_analysis(
            &record.id,
            &version.id,
            "qwen",
            &cited_draft(&segments[0].id, "确认采用本地知识库"),
        )
        .unwrap();
    assert_eq!(repository.list_action_items(None, None).unwrap().len(), 1);

    repository
        .update_segment_text(&segments[0].id, Some("确认采用纯本地知识库"))
        .unwrap();
    assert_eq!(
        repository
            .latest_analysis(&record.id)
            .unwrap()
            .unwrap()
            .status,
        "stale"
    );
    assert!(repository.list_action_items(None, None).unwrap().is_empty());
    assert_eq!(
        repository
            .get_record(&record.id)
            .unwrap()
            .analysis_status
            .as_deref(),
        Some("stale")
    );

    repository
        .save_transcript(
            &record.id,
            "whisper.cpp",
            "small-v2",
            &[TranscriptSegmentInput {
                start_ms: 0,
                end_ms: 1_000,
                speaker_label: None,
                original_text: "新版本逐字稿".into(),
            }],
        )
        .unwrap();
    assert_eq!(
        repository.list_transcript_segments(&record.id).unwrap()[0].original_text,
        "新版本逐字稿"
    );
}

#[test]
fn custom_template_snapshot_survives_template_deletion() {
    let repository = LibraryRepository::new(database_path()).unwrap();
    assert!(repository
        .update_analysis_template("builtin-standard", "修改", "", "修改", &[])
        .is_err());
    let template = repository
        .create_analysis_template(
            "洞察模板",
            "验证历史快照",
            "提取可复用洞察",
            &[TemplateSection {
                key: "insights".into(),
                title: "额外洞察".into(),
                format: "list".into(),
                instruction: "提取证据".into(),
            }],
        )
        .unwrap();
    let record = repository
        .create_record(
            "模板测试",
            None,
            Path::new("audio/template.wav"),
            "template-hash",
            1_000,
        )
        .unwrap();
    repository
        .set_record_analysis_template(&record.id, &template.id)
        .unwrap();
    let (version, segments) = repository
        .save_transcript(
            &record.id,
            "whisper.cpp",
            "small",
            &[TranscriptSegmentInput {
                start_ms: 0,
                end_ms: 1_000,
                speaker_label: None,
                original_text: "保留模板快照".into(),
            }],
        )
        .unwrap();
    let stored = repository
        .save_analysis_with_template(
            &record.id,
            &version.id,
            "qwen",
            &cited_draft(&segments[0].id, "保留模板快照"),
            &template,
        )
        .unwrap();
    repository.delete_analysis_template(&template.id).unwrap();
    assert!(repository.get_analysis_template(&template.id).is_err());
    assert!(stored.template_snapshot_json.contains("洞察模板"));
    assert!(repository
        .get_analysis(&stored.id)
        .unwrap()
        .template_snapshot_json
        .contains("额外洞察"));
    assert_eq!(
        repository
            .get_record(&record.id)
            .unwrap()
            .analysis_template_id
            .as_deref(),
        Some("builtin-standard")
    );
}

#[test]
fn knowledge_vectors_round_trip_and_follow_scope() {
    let repository = LibraryRepository::new(database_path()).unwrap();
    let project = repository.create_project("向量知识库").unwrap();
    let record = repository
        .create_record(
            "向量测试",
            Some(&project.id),
            Path::new("audio/vector.wav"),
            "vector-hash",
            1_000,
        )
        .unwrap();
    let (version, segments) = repository
        .save_transcript(
            &record.id,
            "whisper.cpp",
            "small",
            &[TranscriptSegmentInput {
                start_ms: 0,
                end_ms: 1_000,
                speaker_label: Some("发言人".into()),
                original_text: "向量内容".into(),
            }],
        )
        .unwrap();
    repository
        .replace_knowledge_chunks(
            &record.id,
            &[KnowledgeChunkInput {
                id: "chunk-1".into(),
                record_id: record.id.clone(),
                project_id: Some(project.id.clone()),
                transcript_version_id: version.id,
                segment_ids: vec![segments[0].id.clone()],
                body: "发言人：向量内容".into(),
                start_ms: 0,
                end_ms: 1_000,
                speaker_label: Some("发言人".into()),
                content_hash: "hash".into(),
                embedding_model: "embedding-test".into(),
                embedding: vec![0.25, -0.5, 1.0],
            }],
        )
        .unwrap();
    let chunks = repository
        .list_knowledge_chunks(Some(&project.id), false, "embedding-test")
        .unwrap();
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].embedding, vec![0.25, -0.5, 1.0]);
    assert!(repository
        .list_knowledge_chunks(None, true, "embedding-test")
        .unwrap()
        .is_empty());

    repository
        .save_transcript(
            &record.id,
            "whisper.cpp",
            "small-v2",
            &[TranscriptSegmentInput {
                start_ms: 0,
                end_ms: 1_000,
                speaker_label: None,
                original_text: "新版本内容".into(),
            }],
        )
        .unwrap();
    assert!(repository
        .list_knowledge_chunks(Some(&project.id), false, "embedding-test")
        .unwrap()
        .is_empty());
}

#[test]
fn knowledge_index_counts_and_search_cover_more_than_twenty_records() {
    let repository = LibraryRepository::new(database_path()).unwrap();
    let project = repository.create_project("规模测试").unwrap();
    for index in 0..21 {
        let record = repository
            .create_record(
                &format!("知识记忆记录 {index}"),
                Some(&project.id),
                Path::new("audio/scale.wav"),
                &format!("scale-hash-{index}"),
                1_000,
            )
            .unwrap();
        let (version, segments) = repository
            .save_transcript(
                &record.id,
                "whisper.cpp",
                "small",
                &[TranscriptSegmentInput {
                    start_ms: 0,
                    end_ms: 1_000,
                    speaker_label: None,
                    original_text: format!("知识记忆验证内容 {index}"),
                }],
            )
            .unwrap();
        repository
            .replace_knowledge_chunks(
                &record.id,
                &[KnowledgeChunkInput {
                    id: format!("scale-chunk-{index}"),
                    record_id: record.id.clone(),
                    project_id: Some(project.id.clone()),
                    transcript_version_id: version.id,
                    segment_ids: vec![segments[0].id.clone()],
                    body: format!("知识记忆验证内容 {index}"),
                    start_ms: 0,
                    end_ms: 1_000,
                    speaker_label: None,
                    content_hash: format!("content-{index}"),
                    embedding_model: "embedding-test".into(),
                    embedding: vec![index as f32, 1.0],
                }],
            )
            .unwrap();
    }
    repository
        .save_knowledge_index_status(&KnowledgeIndexStatus {
            scope_key: format!("project:{}", project.id),
            status: "stale".into(),
            total_records: 0,
            processed_records: 0,
            chunk_count: 0,
            embedding_model: "embedding-test".into(),
            last_error: None,
            updated_at: "2026-01-01".into(),
        })
        .unwrap();
    repository
        .refresh_knowledge_index_counts("embedding-test")
        .unwrap();
    let status = repository
        .get_knowledge_index_status(&format!("project:{}", project.id), "embedding-test")
        .unwrap();
    assert_eq!(status.status, "completed");
    assert_eq!(status.processed_records, 21);
    assert_eq!(status.chunk_count, 21);
    let started = std::time::Instant::now();
    let result_record_ids = repository
        .search("知识记忆", Some(&project.id), false, 50)
        .unwrap()
        .into_iter()
        .map(|result| result.record_id)
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(result_record_ids.len(), 21);
    assert!(started.elapsed() < std::time::Duration::from_secs(1));
}

#[test]
fn transcript_reads_and_indexes_only_latest_completed_version() {
    let repository = LibraryRepository::new(database_path()).unwrap();
    let record = repository
        .create_record(
            "版本测试",
            None,
            Path::new("audio/version.wav"),
            "version-hash",
            1_000,
        )
        .unwrap();
    repository
        .save_transcript(
            &record.id,
            "whisper.cpp",
            "old",
            &[TranscriptSegmentInput {
                start_ms: 0,
                end_ms: 500,
                speaker_label: None,
                original_text: "旧版本内容".into(),
            }],
        )
        .unwrap();
    repository
        .save_transcript(
            &record.id,
            "whisper.cpp",
            "new",
            &[TranscriptSegmentInput {
                start_ms: 0,
                end_ms: 500,
                speaker_label: None,
                original_text: "最新版本内容".into(),
            }],
        )
        .unwrap();

    let segments = repository.list_transcript_segments(&record.id).unwrap();
    assert_eq!(segments.len(), 1);
    assert_eq!(segments[0].original_text, "最新版本内容");
    assert!(repository
        .search("旧版本", None, false, 10)
        .unwrap()
        .is_empty());
    assert_eq!(
        repository
            .search("最新版本", None, false, 10)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn fts_updates_after_transcript_edit() {
    let repository = LibraryRepository::new(database_path()).unwrap();
    let record = repository
        .create_record(
            "产品讨论",
            None,
            Path::new("audio/demo.wav"),
            "fts-hash",
            1000,
        )
        .unwrap();
    let (_, segments) = repository
        .save_transcript(
            &record.id,
            "whisper.cpp",
            "base",
            &[TranscriptSegmentInput {
                start_ms: 10,
                end_ms: 20,
                speaker_label: None,
                original_text: "旧的决策内容".into(),
            }],
        )
        .unwrap();
    assert_eq!(
        repository.search("旧的决", None, false, 10).unwrap().len(),
        1
    );
    repository
        .update_segment_text(&segments[0].id, Some("新的桌面决策"))
        .unwrap();
    assert!(repository
        .search("旧的决", None, false, 10)
        .unwrap()
        .is_empty());
    let results = repository.search("桌面决", None, false, 10).unwrap();
    assert_eq!(results[0].record_id, record.id);
    assert_eq!(results[0].start_ms, Some(10));
}

#[test]
fn renaming_record_updates_every_search_row() {
    let repository = LibraryRepository::new(database_path()).unwrap();
    let record = repository
        .create_record(
            "旧录音标题",
            None,
            Path::new("audio/rename.wav"),
            "rename-hash",
            1_000,
        )
        .unwrap();
    repository
        .save_transcript(
            &record.id,
            "whisper.cpp",
            "base",
            &[TranscriptSegmentInput {
                start_ms: 10,
                end_ms: 20,
                speaker_label: None,
                original_text: "标题同步测试内容".into(),
            }],
        )
        .unwrap();

    let updated = repository
        .update_record_title(&record.id, "新录音标题")
        .unwrap();
    assert_eq!(updated.title, "新录音标题");
    assert!(repository
        .search("旧录音标题", None, false, 10)
        .unwrap()
        .is_empty());
    let results = repository.search("新录音标题", None, false, 10).unwrap();
    assert!(results.len() >= 2, "标题和逐字稿索引行都应更新");
    assert!(results.iter().all(|result| result.title == "新录音标题"));
}

#[test]
fn analysis_persists_citations_and_action_items() {
    let repository = LibraryRepository::new(database_path()).unwrap();
    let record = repository
        .create_record("讨论", None, Path::new("audio/a.wav"), "analysis-hash", 1)
        .unwrap();
    let (version, segments) = repository
        .save_transcript(
            &record.id,
            "whisper.cpp",
            "base",
            &[TranscriptSegmentInput {
                start_ms: 0,
                end_ms: 1,
                speaker_label: None,
                original_text: "决定使用桌面应用".into(),
            }],
        )
        .unwrap();
    let draft = AnalysisDraft {
        summary: "摘要".into(),
        key_points: vec![],
        decisions: vec![AnalysisItemDraft {
            text: "采用桌面应用".into(),
            citation_segment_ids: vec![segments[0].id.clone()],
            quote_text: "决定使用桌面应用".into(),
            start_ms: Some(0),
            end_ms: Some(1),
        }],
        action_items: vec![AnalysisItemDraft {
            text: "完成发布".into(),
            citation_segment_ids: vec![segments[0].id.clone()],
            quote_text: "决定使用桌面应用".into(),
            start_ms: Some(0),
            end_ms: Some(1),
        }],
        open_questions: vec![],
        custom_sections: vec![],
        quality_warning: None,
    };
    repository
        .save_analysis(&record.id, &version.id, "qwen", &draft)
        .unwrap();
    assert!(repository.latest_analysis(&record.id).unwrap().is_some());
    assert_eq!(
        repository.list_action_items(None, None).unwrap()[0].title,
        "完成发布"
    );
    assert_eq!(
        repository.search("完成发", None, false, 10).unwrap()[0].record_id,
        record.id
    );
    let decisions = repository.list_decisions(None, 10).unwrap();
    assert_eq!(decisions[0]["text"], "采用桌面应用");
    assert_eq!(decisions[0]["startMs"], 0);
}

#[test]
fn incomplete_latest_analysis_stays_in_pending_workflow() {
    let repository = LibraryRepository::new(database_path()).unwrap();
    let record = repository
        .create_record(
            "待修正分析",
            None,
            Path::new("audio/incomplete.wav"),
            "incomplete-analysis-hash",
            1,
        )
        .unwrap();
    let (version, _) = repository
        .save_transcript(
            &record.id,
            "whisper.cpp",
            "base",
            &[TranscriptSegmentInput {
                start_ms: 0,
                end_ms: 1,
                speaker_label: None,
                original_text: "这是一段测试逐字稿".into(),
            }],
        )
        .unwrap();
    let stored = repository
        .save_analysis(
            &record.id,
            &version.id,
            "qwen",
            &AnalysisDraft {
                summary: "分析结果仍需修正".into(),
                key_points: vec![],
                decisions: vec![],
                action_items: vec![],
                open_questions: vec![],
                custom_sections: vec![],
                quality_warning: Some("关键观点不足".into()),
            },
        )
        .unwrap();
    let refreshed = repository.get_record(&record.id).unwrap();
    assert_eq!(stored.status, "incomplete");
    assert!(!refreshed.has_analysis);
    assert_eq!(refreshed.analysis_status.as_deref(), Some("incomplete"));
}

#[test]
fn project_crud_full_cycle() {
    let repo = LibraryRepository::new(database_path()).unwrap();
    let p = repo.create_project("原名").unwrap();

    let updated = repo
        .update_project(&p.id, Some("新名"), Some("archived"))
        .unwrap();
    assert_eq!(updated.name, "新名");
    assert_eq!(updated.status, "archived");

    assert_eq!(repo.get_project(&p.id).unwrap().name, "新名");

    repo.delete_project(&p.id).unwrap();
    assert!(repo.get_project(&p.id).is_err());
}

#[test]
fn empty_project_name_rejected() {
    let repo = LibraryRepository::new(database_path()).unwrap();
    assert!(repo.create_project("   ").is_err());
}

#[test]
fn record_status_transitions_and_filter() {
    let repo = LibraryRepository::new(database_path()).unwrap();
    let a = repo.create_project("A").unwrap();
    let b = repo.create_project("B").unwrap();

    let r1 = repo
        .create_record("R1", Some(&a.id), Path::new("x1"), "h1", 1000)
        .unwrap();
    repo.create_record("R2", Some(&b.id), Path::new("x2"), "h2", 2000)
        .unwrap();

    // 全部 + 按项目过滤
    assert_eq!(repo.list_records(None, false).unwrap().len(), 2);
    assert_eq!(repo.list_records(Some(&a.id), false).unwrap().len(), 1);

    // 状态推进
    let updated = repo.update_record_status(&r1.id, "transcribing").unwrap();
    assert_eq!(updated.status, "transcribing");
    // 非法状态被拒
    assert!(repo.update_record_status(&r1.id, "bogus").is_err());
}

#[test]
fn processing_job_lifecycle() {
    let repo = LibraryRepository::new(database_path()).unwrap();
    let p = repo.create_project("P").unwrap();
    let r = repo
        .create_record("Rec", Some(&p.id), Path::new("a"), "hh", 0)
        .unwrap();

    let job = repo.create_job(&r.id, "transcribe").unwrap();
    assert_eq!(job.status, "queued");
    assert_eq!(job.attempt_count, 0);

    // next_queued 应取到该任务
    assert_eq!(repo.next_queued_job().unwrap().unwrap().id, job.id);

    // 进入处理态 -> attempt_count 自增
    let running = repo
        .update_job_status(&job.id, "transcribing", None)
        .unwrap();
    assert_eq!(running.attempt_count, 1);

    // 失败记录 last_error
    let failed = repo
        .update_job_status(&job.id, "failed", Some("模型超时"))
        .unwrap();
    assert_eq!(failed.status, "failed");
    assert_eq!(failed.last_error.as_deref(), Some("模型超时"));

    // 失败后队列应为空
    assert!(repo.next_queued_job().unwrap().is_none());

    assert_eq!(repo.list_jobs_for_record(&r.id).unwrap().len(), 1);

    // 非法任务类型被拒
    assert!(repo.create_job(&r.id, "bogus").is_err());
    // 记录不存在时创建任务报错
    assert!(repo.create_job("no-such-record", "analyze").is_err());
}

#[test]
fn delete_record_cascades_jobs() {
    let repo = LibraryRepository::new(database_path()).unwrap();
    let p = repo.create_project("P").unwrap();
    let r = repo
        .create_record("Rec", Some(&p.id), Path::new("a"), "cascade-h", 0)
        .unwrap();
    repo.create_job(&r.id, "transcribe").unwrap();
    repo.create_job(&r.id, "analyze").unwrap();
    assert_eq!(repo.list_jobs_for_record(&r.id).unwrap().len(), 2);

    repo.delete_record(&r.id).unwrap();
    // ON DELETE CASCADE：记录删除后其任务一并清除
    assert_eq!(repo.list_jobs_for_record(&r.id).unwrap().len(), 0);
}

#[test]
fn moving_record_updates_search_and_action_item_scope_atomically() {
    let repository = LibraryRepository::new(database_path()).unwrap();
    let source = repository.create_project("来源知识库").unwrap();
    let destination = repository.create_project("目标知识库").unwrap();
    let record = repository
        .create_record(
            "归档测试",
            Some(&source.id),
            Path::new("audio/move.wav"),
            "move-hash",
            1,
        )
        .unwrap();
    let (version, segments) = repository
        .save_transcript(
            &record.id,
            "whisper.cpp",
            "base",
            &[TranscriptSegmentInput {
                start_ms: 2_000,
                end_ms: 3_000,
                speaker_label: Some("未知".into()),
                original_text: "决定迁移到目标知识库".into(),
            }],
        )
        .unwrap();
    repository
        .save_analysis(
            &record.id,
            &version.id,
            "qwen",
            &AnalysisDraft {
                summary: "这是一段用于验证知识库归属同步行为的完整测试摘要。".into(),
                key_points: vec![],
                decisions: vec![],
                action_items: vec![AnalysisItemDraft {
                    text: "完成知识库迁移".into(),
                    citation_segment_ids: vec![segments[0].id.clone()],
                    quote_text: "决定迁移到目标知识库".into(),
                    start_ms: Some(2_000),
                    end_ms: Some(3_000),
                }],
                open_questions: vec![],
                custom_sections: vec![],
                quality_warning: None,
            },
        )
        .unwrap();

    let updated = repository
        .update_record_project(&record.id, Some(&destination.id))
        .unwrap();
    assert_eq!(updated.project_id.as_deref(), Some(destination.id.as_str()));
    assert!(repository
        .search("目标知识", Some(&source.id), false, 10)
        .unwrap()
        .is_empty());
    let result = repository
        .search("目标知识", Some(&destination.id), false, 10)
        .unwrap()
        .remove(0);
    assert_eq!(result.project_name.as_deref(), Some("目标知识库"));
    assert_eq!(result.source_type, "transcript");
    assert_eq!(result.speaker_label.as_deref(), Some("未知"));
    assert_eq!(
        result.target_segment_id.as_deref(),
        Some(segments[0].id.as_str())
    );
    assert_eq!(
        repository
            .list_action_items(Some(&destination.id), None)
            .unwrap()
            .len(),
        1
    );

    repository.update_record_project(&record.id, None).unwrap();
    assert_eq!(repository.list_records(None, true).unwrap().len(), 1);
    assert_eq!(
        repository.search("目标知识", None, true, 10).unwrap().len(),
        1
    );
}

#[test]
fn mcp_is_disabled_by_default_and_tracks_recent_calls() {
    let repository = LibraryRepository::new(database_path()).unwrap();
    assert!(!repository.mcp_status().unwrap().enabled);
    assert!(repository.set_mcp_enabled(true).unwrap().enabled);
    repository
        .log_mcp_access("search_records", None, None)
        .unwrap();
    let status = repository.mcp_status().unwrap();
    assert_eq!(status.recent_calls[0].tool_name, "search_records");
    assert_eq!(status.authorized_scope, "全部知识库（只读）");
}
