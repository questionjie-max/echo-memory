//! 块7 验收：单条记录导出包。
//! 断言：文件齐全可打开；纪要与面板同一份 content_json 逐条一致（含时间戳与出处）；
//! srt 与 segments 一致；manifest 字段完整、确认状态分布如实（未确认不悄悄变已确认）；
//! 双路对齐偏移进 manifest；auto-export 与导出包互不耦合（口径注释见 export.rs）。
use echo_memory_lib::library::ManagedLibrary;
use echo_memory_lib::minutes::{MinutesDocument, MinutesEvidence, MinutesItem, MinutesItemKind, MinutesItemStatus};
use echo_memory_lib::types::TranscriptSegmentInput;
use std::path::Path;

fn seg(start: i64, text: &str) -> TranscriptSegmentInput {
    TranscriptSegmentInput {
        start_ms: start,
        end_ms: start + 2_000,
        speaker_label: Some("说话人一".into()),
        original_text: text.into(),
    }
}

#[test]
fn export_package_matches_panel_and_manifest_is_complete() {
    let root = std::env::temp_dir().join(format!("echo-pkg-{}", uuid::Uuid::new_v4()));
    let library = ManagedLibrary::open(&root).unwrap();
    let repo = library.repository();
    let record = repo
        .create_record("导出包会议", None, Path::new("audio/p.wav"), "hash-p", 60_000)
        .unwrap();
    repo.save_transcript(
        &record.id,
        "whisper.cpp",
        "base",
        &[seg(0, "第一句确认发布日期。"), seg(2_000, "第二句讨论热词库。")],
    )
    .unwrap();
    repo.ensure_minutes_pending(&record.id).unwrap();
    repo.save_minutes_document(
        &record.id,
        "ollama",
        "qwen2.5:7b",
        &MinutesDocument {
            topics: vec![],
            conclusions: vec![
                MinutesItem {
                    id: "pkg-conf".into(),
                    kind: MinutesItemKind::Conclusion,
                    text: "发布日期已确认PKG1".into(),
                    owner: None,
                    evidence: MinutesEvidence {
                        start_ms: 0,
                        end_ms: 2_000,
                        quote_text: "第一句确认发布日期。".into(),
                    },
                },
                MinutesItem {
                    id: "pkg-prov".into(),
                    kind: MinutesItemKind::Conclusion,
                    text: "讨论热词库暂定PKG2".into(),
                    owner: None,
                    evidence: MinutesEvidence {
                        start_ms: 2_000,
                        end_ms: 4_000,
                        quote_text: "第二句讨论热词库。".into(),
                    },
                },
            ],
            action_items: vec![],
            glossary: vec![],
            dropped_unverifiable: 0,
        },
    )
    .unwrap();
    repo.set_minutes_item_status("pkg-conf", MinutesItemStatus::Confirmed)
        .unwrap();

    let destination = root.join("产出");
    let folder = echo_memory_lib::export::export_record_package(&library, &record.id, &destination).unwrap();

    // 文件齐全且可读。
    let transcript_md = std::fs::read_to_string(folder.join("逐字稿.md")).unwrap();
    let transcript_srt = std::fs::read_to_string(folder.join("逐字稿.srt")).unwrap();
    let minutes_md = std::fs::read_to_string(folder.join("纪要.md")).unwrap();
    let manifest_text = std::fs::read_to_string(folder.join("manifest.json")).unwrap();

    // 纪要与面板同一份 content_json：逐条文本 + 出处 + 时间戳一致。
    assert!(minutes_md.contains("发布日期已确认PKG1"));
    assert!(minutes_md.contains("讨论热词库暂定PKG2"), "未确认条目照常渲染（既有导出口径），不得消失");
    assert!(minutes_md.contains("出处：「第一句确认发布日期。」"), "出处引文必须一致");
    assert!(minutes_md.contains("0:00"), "结论时间戳必须一致");

    // srt 与 segments 一致（时间轴 + 文本）。
    assert!(transcript_srt.contains("00:00:00,000 --> 00:00:02,000"));
    assert!(transcript_srt.contains("第二句讨论热词库。"));
    assert!(transcript_md.contains("第一句确认发布日期。"));

    // manifest 字段完整。
    let manifest: serde_json::Value = serde_json::from_str(&manifest_text).unwrap();
    assert_eq!(manifest["record"]["title"], "导出包会议");
    assert_eq!(manifest["record"]["sourceType"], "import");
    assert_eq!(manifest["record"]["id"], record.id);
    assert!(manifest["record"]["importedAt"].is_string());
    assert_eq!(manifest["record"]["durationMs"], 60_000);
    assert_eq!(manifest["record"]["audioHash"], "hash-p");
    assert!(manifest["record"]["archivedAt"].is_null());
    assert!(manifest["record"]["importConfirmedAt"].is_null(), "音频类记录无确认语义");
    assert!(manifest["alignmentOffsetMs"].is_null(), "单路录音无对齐偏移");
    assert_eq!(manifest["minutes"]["status"], "completed");
    assert_eq!(manifest["minutes"]["provider"], "ollama");
    assert_eq!(manifest["minutes"]["model"], "qwen2.5:7b");
    assert_eq!(manifest["minutes"]["droppedUnverifiable"], 0);
    assert_eq!(manifest["minutes"]["itemStatuses"]["confirmed"], 1, "确认状态分布必须如实");
    assert_eq!(manifest["minutes"]["itemStatuses"]["provisional"], 1);
    assert_eq!(manifest["files"][0], "逐字稿.md");
    assert_eq!(manifest["files"][1], "逐字稿.srt");
    assert_eq!(manifest["files"][2], "纪要.md");
    assert!(manifest["exportedAt"].is_string());

    // 撤回后重导：分布变为 0/2——导出如实反映当前确认态。
    repo.set_minutes_item_status("pkg-conf", MinutesItemStatus::Provisional)
        .unwrap();
    let folder2 = echo_memory_lib::export::export_record_package(&library, &record.id, &destination).unwrap();
    let manifest2: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(folder2.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest2["minutes"]["itemStatuses"]["confirmed"], 0);
    assert_eq!(manifest2["minutes"]["itemStatuses"]["provisional"], 2);
    assert_ne!(folder, folder2, "重名导出包必须自动避让");

    std::fs::remove_dir_all(&root).ok();
}

/// 无逐字稿的记录不能导出包（错误可读）。
#[test]
fn export_package_requires_transcript() {
    let root = std::env::temp_dir().join(format!("echo-pkg-e{}", uuid::Uuid::new_v4()));
    let library = ManagedLibrary::open(&root).unwrap();
    let repo = library.repository();
    let record = repo
        .create_record("空记录", None, Path::new("audio/e.wav"), "hash-e", 1_000)
        .unwrap();
    let error = echo_memory_lib::export::export_record_package(&library, &record.id, &root.join("out")).unwrap_err();
    assert!(error.to_string().contains("逐字稿"));
    std::fs::remove_dir_all(&root).ok();
}
