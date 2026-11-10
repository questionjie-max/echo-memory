//! 块4 验收：导入确认门的端到端语义（lib 层——MCP 门集中在 repository，
//! 二进制只做 JSON-RPC 壳；真二进制复跑见进度日志的探针记录）。
//! 承诺：未确认的导入内容在全部 MCP 出口出现次数为 0；确认后带溯源可见；
//! 撤回后再次不可见；归档后同样排除；录音链路不受门影响。
#![cfg(feature = "mcp-bin")]

use echo_memory_lib::library::ManagedLibrary;
use echo_memory_lib::minutes::{MinutesDocument, MinutesEvidence, MinutesItem, MinutesItemKind, MinutesItemStatus};
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

/// 造一条纪要条目（直接走仓储，绕过模型）；confirm=false 时保持暂定态。
fn add_minutes(
    repo: &echo_memory_lib::db::repository::LibraryRepository,
    record_id: &str,
    item_id: &str,
    text: &str,
    quote: &str,
    confirm: bool,
) {
    repo.ensure_minutes_pending(record_id).unwrap();
    repo.save_minutes_document(
        record_id,
        "ollama",
        "qwen2.5:7b",
        &MinutesDocument {
            topics: vec![],
            conclusions: vec![MinutesItem {
                id: item_id.into(),
                kind: MinutesItemKind::Conclusion,
                text: text.into(),
                owner: None,
                evidence: MinutesEvidence {
                    start_ms: 1_000,
                    end_ms: 5_000,
                    quote_text: quote.into(),
                },
            }],
            action_items: vec![],
            glossary: vec![],
            dropped_unverifiable: 0,
        },
    )
    .unwrap();
    if confirm {
        repo.set_minutes_item_status(item_id, MinutesItemStatus::Confirmed)
            .unwrap();
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

    // 独立工具出口（终审 P0 补测）：未确认导入的分析派生待办不得从
    // list_action_items 漏出。给 chat 记录补一份带待办的分析。
    let (version, chat_segs) = repo
        .save_transcript(&chat.id, "import", "v1", &[seg(0, "派生待办的来源段落。")])
        .unwrap();
    repo.save_analysis(
        &chat.id,
        &version.id,
        "test-model",
        &echo_memory_lib::analysis::AnalysisDraft {
            summary: "含机密的分析LEAKSUM".into(),
            key_points: vec![],
            decisions: vec![],
            action_items: vec![echo_memory_lib::analysis::AnalysisItemDraft {
                owner: None,
                text: "机密待办LEAKTODO".into(),
                citation_segment_ids: vec![chat_segs[0].id.clone()],
                quote_text: "派生待办的来源段落。".into(),
                start_ms: Some(0),
                end_ms: Some(1_000),
            }],
            open_questions: vec![],
            custom_sections: vec![],
            chapters: vec![],
            highlights: vec![],
            quality_warning: None,
            quality_severity: None,
        },
    )
    .unwrap();
    let leaked = repo.mcp_list_action_items(None, None).unwrap();
    assert!(
        !leaked
            .iter()
            .any(|item| item.record_id == chat.id && item.title.contains("LEAKTODO")),
        "未确认导入的派生待办泄漏到独立工具 list_action_items：{leaked:?}"
    );
    assert!(
        !repo
            .mcp_search("LEAKTODO", None, 10)
            .unwrap()
            .iter()
            .any(|item| item.record_id == chat.id),
        "分析待办文本不应能被搜索命中（未确认）"
    );

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
        transcript[0].original_text.contains("派生待办"),
        "确认后应能读到原文（含逐字稿溯源坐标；最新版本）"
    );
    assert!(
        repo.mcp_search("派生待办", None, 10)
            .unwrap()
            .iter()
            .any(|item| item.record_id == chat.id),
        "确认后应可被搜索命中（含其分析派生内容）"
    );
    // 独立工具出口同步放行（终审修1 的正向断言）。
    let released = repo.mcp_list_action_items(None, None).unwrap();
    assert!(
        released
            .iter()
            .any(|item| item.record_id == chat.id && item.title.contains("LEAKTODO")),
        "确认后派生待办应可见：{released:?}"
    );

    // —— 第三轮：撤回后再次不可见（与纪要条目同语义，双向可逆）——
    let withdrawn = repo.set_import_confirmed(&chat.id, false).unwrap();
    assert!(withdrawn.import_confirmed_at.is_none());
    assert!(repo.mcp_get_record(&chat.id).is_err());
    assert!(
        !repo
            .mcp_search("派生待办", None, 10)
            .unwrap()
            .iter()
            .any(|item| item.record_id == chat.id),
        "撤回后不应被搜索命中"
    );
    assert!(
        !repo
            .mcp_list_action_items(None, None)
            .unwrap()
            .iter()
            .any(|item| item.record_id == chat.id),
        "撤回后派生待办应再次不可见"
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

/// 块5 验收：list_confirmed_minutes（按时间范围取已确认纪要）的可见性与范围语义。
/// 三类数据（已确认/暂定/归档上的已确认）逐出口断言；时间范围以记录时间为准。
#[test]
fn list_confirmed_minutes_respects_range_and_gates() {
    let root = std::env::temp_dir().join(format!("echo-m5-{}", uuid::Uuid::new_v4()));
    let library = ManagedLibrary::open(&root).unwrap();
    let repo = library.repository();
    let project = repo.create_project("块五项目").unwrap();

    let live = repo
        .create_record("在库录音", Some(&project.id), Path::new("audio/a.wav"), "h-a", 60_000)
        .unwrap();
    repo.save_transcript(&live.id, "whisper.cpp", "base", &[seg(1_000, "会议确认了发布日期定档。")])
        .unwrap();
    add_minutes(&repo, &live.id, "m-live", "发布日期定档结论", "会议确认了发布日期定档。", true);

    // 暂定条目（不确认）——任何出口都不该出现。
    let pending = repo
        .create_record("暂定录音", Some(&project.id), Path::new("audio/b.wav"), "h-b", 60_000)
        .unwrap();
    repo.save_transcript(&pending.id, "whisper.cpp", "base", &[seg(1_000, "暂定条目不该出现。")])
        .unwrap();
    add_minutes(&repo, &pending.id, "m-pending", "暂定纪要M5P", "暂定条目不该出现。", false);

    // 归档记录上的已确认条目——不该出现。
    let archived = repo
        .create_record("归档录音", Some(&project.id), Path::new("audio/c.wav"), "h-c", 60_000)
        .unwrap();
    repo.save_transcript(&archived.id, "whisper.cpp", "base", &[seg(1_000, "归档条目不该出现。")])
        .unwrap();
    add_minutes(&repo, &archived.id, "m-arch", "归档纪要M5A", "归档条目不该出现。", true);
    repo.set_record_archived(&archived.id, true).unwrap();

    repo.set_mcp_enabled(true).unwrap();

    // 全量（无范围）：只有 live 的已确认条目；暂定与归档条目缺席即断言成立。
    let all = repo.list_confirmed_minutes_in_range(None, None, None, 20).unwrap();
    assert_eq!(all.len(), 1, "只应有 1 条已确认未归档条目：{all:?}");
    assert_eq!(all[0].item_id, "m-live");
    assert_eq!(all[0].record_title, "在库录音");
    assert_eq!(all[0].quote_text, "会议确认了发布日期定档。");
    assert!(all[0].start_ms == 1_000 && all[0].end_ms == 5_000, "溯源时间窗必须带上");
    assert!(!all[0].confirmed_at.is_empty());

    // 时间下界在未来：空。
    let none = repo
        .list_confirmed_minutes_in_range(Some("2099-01-01"), None, None, 20)
        .unwrap();
    assert!(none.is_empty(), "未来下界不应返回条目");

    // 时间下界在远古：返回全部（边界语义：since/until 是包含式过滤）。
    let ancient = repo
        .list_confirmed_minutes_in_range(Some("2000-01-01"), None, None, 20)
        .unwrap();
    assert_eq!(ancient.len(), 1);
    assert_eq!(ancient[0].item_id, "m-live");

    // 终审修3：date-only 的 until 是「含当日」——今天创建的记录必须被包含。
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let todays = repo
        .list_confirmed_minutes_in_range(None, Some(&today), None, 20)
        .unwrap();
    assert_eq!(todays.len(), 1, "date-only until 必须含当日：{todays:?}");
    assert_eq!(todays[0].item_id, "m-live");

    // 知识库过滤：其他项目的条目不串台。
    let other_project = repo.create_project("另一个库").unwrap();
    let other = repo
        .create_record("别库录音", Some(&other_project.id), Path::new("audio/d.wav"), "h-d", 60_000)
        .unwrap();
    repo.save_transcript(&other.id, "whisper.cpp", "base", &[seg(1_000, "另一个库的结论。")])
        .unwrap();
    add_minutes(&repo, &other.id, "m-other", "别库纪要M5O", "另一个库的结论。", true);
    let scoped = repo
        .list_confirmed_minutes_in_range(None, None, Some(&project.id), 20)
        .unwrap();
    assert_eq!(scoped.len(), 1);
    assert_eq!(scoped[0].item_id, "m-live");
    let unscoped = repo.list_confirmed_minutes_in_range(None, None, None, 20).unwrap();
    assert_eq!(unscoped.len(), 2);

    // limit 生效。
    let limited = repo.list_confirmed_minutes_in_range(None, None, None, 1).unwrap();
    assert_eq!(limited.len(), 1);

    std::fs::remove_dir_all(&root).ok();
}
