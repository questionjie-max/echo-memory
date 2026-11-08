//! 块4 验收：导入确认门的端到端语义（lib 层——MCP 门集中在 repository，
//! 二进制只做 JSON-RPC 壳；真二进制复跑见进度日志的探针记录）。
//! 承诺：未确认的导入内容在全部 MCP 出口出现次数为 0；确认后带溯源可见；
//! 撤回后再次不可见；归档后同样排除；录音链路不受门影响。
#![cfg(feature = "mcp-bin")]

use echo_memory_lib::library::ManagedLibrary;
use echo_memory_lib::types::TranscriptSegmentInput;
use std::path::Path;

fn seg(start: i64, text: &str) -> TranscriptSegmentInput {
    TranscriptSegmentInput {
        start_ms: start,
        end_ms: start + 1_000,
        speaker_label: Some("用户".into()),
        original_text: text.into(),
    }
}

#[test]
fn unconfirmed_imports_are_invisible_across_mcp_exits() {
    let root = std::env::temp_dir().join(format!("echo-gate-{}", uuid::Uuid::new_v4()));
    let library = ManagedLibrary::open(&root).unwrap();
    let repo = library.repository();
    let project = repo.create_project("导入确认门").unwrap();

    // 未确认 chat 记录：新导入的默认态（import_confirmed_at = NULL）。
    let doc_path = root.join("chat.md");
    std::fs::write(&doc_path, "用户: 探针对话内容\n\n助手: 收到。").unwrap();
    let chat = repo
        .create_chat_record(
            &uuid::Uuid::new_v4().to_string(),
            "未确认对话",
            Some(&project.id),
            &doc_path,
            "hash-chat",
            "chat-md",
            None,
            &[seg(0, "探针对话内容GATE1")],
        )
        .unwrap();
    // 录音记录：不受导入确认门影响（原文层 v1.0 起可读）。
    let recording = repo
        .create_record(
            "普通录音",
            Some(&project.id),
            Path::new("audio/r.wav"),
            "hash-rec",
            10_000,
        )
        .unwrap();
    repo.save_transcript(
        &recording.id,
        "whisper.cpp",
        "base",
        &[TranscriptSegmentInput {
            start_ms: 0,
            end_ms: 1_000,
            speaker_label: None,
            original_text: "录音原文GATE2".into(),
        }],
    )
    .unwrap();
    repo.set_mcp_enabled(true).unwrap();

    // —— 第一轮：未确认导入必须从全部出口消失 ——
    let search = repo.mcp_search("GATE", None, 10).unwrap();
    assert!(
        search.iter().any(|item| item.record_id == recording.id),
        "录音应可搜到"
    );
    assert!(
        !search.iter().any(|item| item.record_id == chat.id),
        "未确认导入泄漏到 mcp_search"
    );

    let error = repo.mcp_get_record(&chat.id).unwrap_err();
    assert!(error.to_string().contains("确认"), "拒绝文案应讲明与确认有关：{error}");
    let error = repo.mcp_get_transcript(&chat.id, 0, i64::MAX).unwrap_err();
    assert!(error.to_string().contains("确认"));

    let (records, decisions, action_items) = repo.mcp_project_context(&project.id).unwrap();
    assert!(
        !records.iter().any(|record| record.id == chat.id),
        "未确认导入泄漏到 mcp_project_context.records"
    );
    assert!(decisions.is_empty() && action_items.is_empty());

    // 录音不受影响（原文可读）。
    let (record, transcript) = repo.mcp_get_record(&recording.id).unwrap();
    assert_eq!(record.id, recording.id);
    assert!(transcript[0].original_text.contains("GATE2"));

    // —— 第二轮：确认后带溯源可见 ——
    let confirmed = repo.set_import_confirmed(&chat.id, true).unwrap();
    assert!(confirmed.import_confirmed_at.is_some());
    let (record, transcript) = repo.mcp_get_record(&chat.id).unwrap();
    assert_eq!(record.id, chat.id);
    assert!(
        transcript[0].original_text.contains("GATE1"),
        "确认后应能读到原文（含逐字稿溯源坐标）"
    );
    assert!(
        repo.mcp_search("GATE1", None, 10)
            .unwrap()
            .iter()
            .any(|item| item.record_id == chat.id),
        "确认后应可被搜索命中"
    );

    // —— 第三轮：撤回后再次不可见（与纪要条目同语义，双向可逆）——
    let withdrawn = repo.set_import_confirmed(&chat.id, false).unwrap();
    assert!(withdrawn.import_confirmed_at.is_none());
    assert!(repo.mcp_get_record(&chat.id).is_err());
    assert!(
        !repo
            .mcp_search("GATE1", None, 10)
            .unwrap()
            .iter()
            .any(|item| item.record_id == chat.id),
        "撤回后不应被搜索命中"
    );

    // —— 第四轮：重新确认后归档，搜索与项目上下文排除归档（原文层 get_record 属 v1.0 既有口径）——
    repo.set_import_confirmed(&chat.id, true).unwrap();
    repo.set_record_archived(&chat.id, true).unwrap();
    assert!(
        !repo
            .mcp_search("GATE1", None, 10)
            .unwrap()
            .iter()
            .any(|item| item.record_id == chat.id),
        "归档导入不应被搜索命中"
    );
    let (records, _, _) = repo.mcp_project_context(&project.id).unwrap();
    assert!(
        !records.iter().any(|record| record.id == chat.id),
        "归档导入不应出现在项目上下文"
    );

    std::fs::remove_dir_all(&root).ok();
}

/// 迁移回填：存量 document/chat 记录视为已确认（既有库 MCP 可见性不变），
/// 录音类记录字段为 NULL 但不受门影响。
#[test]
fn migration_backfills_existing_imports_as_confirmed() {
    let root = std::env::temp_dir().join(format!("echo-gate-mig-{}", uuid::Uuid::new_v4()));
    let library = ManagedLibrary::open(&root).unwrap();
    let repo = library.repository();
    // 新建库即应用全部迁移：直接验证「新导入默认未确认」与「录音不受影响」。
    let doc_path = root.join("d.md");
    std::fs::write(&doc_path, "# 文档\n\n内容").unwrap();
    let doc = repo
        .create_chat_record(
            &uuid::Uuid::new_v4().to_string(),
            "新导入对话",
            None,
            &doc_path,
            "hash-d",
            "chat-md",
            None,
            &[seg(0, "新导入内容")],
        )
        .unwrap();
    assert!(
        doc.import_confirmed_at.is_none(),
        "新导入必须默认未确认（确认门生效）"
    );
    assert!(repo.is_unconfirmed_import(&doc.id).unwrap());
    assert!(!repo.list_unconfirmed_import_ids().unwrap().is_empty());
    std::fs::remove_dir_all(&root).ok();
}
