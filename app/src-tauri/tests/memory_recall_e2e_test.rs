//! 记忆召回端到端（真实 Ollama）：导入 AI 对话 → 确认记忆条目 → 记忆索引重建
//! → 知识索引 → 问知识库 → 断言记忆引用与逐字引文。
//! 本地免费模型全真管线，不依赖外部 AI。
//!
//! 运行：ECHO_E2E_OLLAMA=1 cargo test --test memory_recall_e2e_test -- --ignored

use echo_memory_lib::chat_import;
use echo_memory_lib::knowledge;
use echo_memory_lib::library::ManagedLibrary;
use echo_memory_lib::memory_index;
use echo_memory_lib::types::{
    EvolutionItem, MemoryGenerationStatus, MemoryScope, MemorySnapshotResult, MemoryViewKind,
};

#[test]
#[ignore = "requires local Ollama with embedding and analysis models"]
fn confirmed_memory_is_recalled_by_knowledge_ask() {
    let root = std::env::temp_dir()
        .join("echo_memory_recall_e2e")
        .join(uuid_v4());
    let library = ManagedLibrary::open(root.clone()).expect("打开临时资料库失败");
    let repository = library.repository();

    // 1. 导入一段与问题无关的 AI 对话（资料路），保证记忆引用不是来自资料原文。
    let chat_path = root.join("source-chat.md");
    std::fs::write(
        &chat_path,
        "# 周末爬山\n\n用户：周末去哪爬山比较好？\n\n助手：推荐去郊外的云顶山，早上出发避开人流。\n",
    )
    .unwrap();
    let outcome =
        chat_import::import_chat_exports(&library, std::slice::from_ref(&chat_path), None, false)
            .expect("导入对话失败");
    assert_eq!(
        outcome.imported.len(),
        1,
        "对话应导入成功: {:?}",
        outcome.skipped
    );
    let record_id = outcome.imported[0].record_id.clone();
    let segments = repository.list_transcript_segments(&record_id).unwrap();
    assert!(!segments.is_empty());
    let segment = &segments[0];

    // 1b. 同一文件重复导入应被去重跳过。
    let repeat =
        chat_import::import_chat_exports(&library, std::slice::from_ref(&chat_path), None, false)
            .expect("重复导入不应报错");
    assert!(repeat.imported.is_empty(), "重复导入不应产生新记录");
    assert_eq!(repeat.skipped.len(), 1);
    assert!(repeat.skipped[0].reason.contains("已导入过"));

    // 2. 生成一条演化记忆并由用户确认（稳定内容 ID 与线上一致）。
    let scope = MemoryScope {
        kind: "all".to_owned(),
        project_id: None,
    };
    let snapshot = repository
        .create_memory_snapshot(
            &MemoryViewKind::Evolution,
            &scope,
            (None, None),
            "e2e-model",
            std::slice::from_ref(&record_id),
            "e2e-request-hash",
        )
        .unwrap();
    let item: EvolutionItem = serde_json::from_value(serde_json::json!({
        "id": "e2e-item-1",
        "topic": "记忆召回",
        "changeType": "added",
        "beforeText": "",
        "afterText": "确认过的记忆条目会进入知识库检索，问答时可以引用并回链。",
        "reason": "",
        "occurredAt": "2026-10-11T00:00:00Z",
        "inferred": true,
        "confidence": 0.9,
        "sources": [{
            "recordId": record_id,
            "segmentId": segment.id,
            "startMs": segment.start_ms,
            "endMs": segment.end_ms,
            "quoteText": segment.original_text
        }]
    }))
    .unwrap();
    repository
        .finish_memory_snapshot(
            &snapshot.id,
            MemoryGenerationStatus::Completed,
            &MemorySnapshotResult {
                evolution_items: vec![item],
                ..Default::default()
            },
            None,
            None,
        )
        .unwrap();
    repository
        .update_memory_feedback(&snapshot.id, "e2e-item-1", "confirmed", "")
        .unwrap();

    // 3. 记忆索引重建：真实 Ollama 嵌入。
    memory_index::rebuild_memory_index(&library).unwrap();
    let settings = repository.knowledge_settings().unwrap();
    let memory_status = repository
        .get_knowledge_index_status("memory", &settings.embedding_model)
        .unwrap();
    assert_eq!(memory_status.status, "completed");
    assert_eq!(memory_status.chunk_count, 1, "确认条目应被投影为记忆块");

    // 4. 资料索引（同一段对话），随后问答。
    knowledge::rebuild_scope(&library, None, false, &mut || {}).unwrap();
    let answer = knowledge::ask(&library, None, false, "确认过的记忆条目能被知识库检索吗？")
        .expect("知识问答失败");
    assert!(!answer.insufficient_evidence, "应有足够证据：{answer:?}");
    assert!(
        !answer.memory_citations.is_empty(),
        "应引用确认过的记忆条目：citations={:?}",
        answer.citations
    );
    let memory_citation = &answer.memory_citations[0];
    assert_eq!(memory_citation.topic, "记忆召回");
    assert!(
        memory_citation
            .source
            .as_ref()
            .is_some_and(|source| source.record_id == record_id),
        "记忆引用应能回链到来源记录"
    );

    // 5. 未确认条目绝不入库：把反馈改为驳回后重建，记忆块应清空。
    repository
        .update_memory_feedback(&snapshot.id, "e2e-item-1", "rejected", "")
        .unwrap();
    memory_index::rebuild_memory_index(&library).unwrap();
    let chunks = repository
        .list_memory_chunks(&settings.embedding_model)
        .unwrap();
    assert!(chunks.is_empty(), "驳回后记忆块应清空");

    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_file(&chat_path);
}

fn uuid_v4() -> String {
    let id = uuid::Uuid::new_v4();
    id.to_string()
}
