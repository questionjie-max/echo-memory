#![cfg(feature = "mcp-bin")]

use echo_memory_lib::analysis::{AnalysisDraft, AnalysisItemDraft};
use echo_memory_lib::library::ManagedLibrary;
use echo_memory_lib::types::TranscriptSegmentInput;
use serde_json::Value;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

#[test]
fn mcp_returns_decisions_with_traceable_source() {
    let root = std::env::temp_dir().join(format!("echo-memory-mcp-{}", uuid::Uuid::new_v4()));
    let library = ManagedLibrary::open(&root).unwrap();
    let project = library.repository().create_project("产品决策").unwrap();
    let record = library
        .repository()
        .create_record(
            "桌面端讨论",
            Some(&project.id),
            Path::new("audio/test.wav"),
            "mcp-e2e-hash",
            10_000,
        )
        .unwrap();
    let (version, segments) = library
        .repository()
        .save_transcript(
            &record.id,
            "whisper.cpp",
            "base",
            &[TranscriptSegmentInput {
                start_ms: 1_000,
                end_ms: 3_000,
                speaker_label: Some("未知".into()),
                original_text: "为了隐私和离线使用，我们决定采用桌面应用。".into(),
            }],
        )
        .unwrap();
    library
        .repository()
        .save_analysis(
            &record.id,
            &version.id,
            "qwen",
            &AnalysisDraft {
                summary: "团队讨论了产品形态，并基于隐私与离线能力确定采用本地桌面应用。".into(),
                key_points: vec![],
                decisions: vec![AnalysisItemDraft {
                    owner: None,
                    text: "采用桌面应用".into(),
                    citation_segment_ids: vec![segments[0].id.clone()],
                    quote_text: "为了隐私和离线使用，我们决定采用桌面应用。".into(),
                    start_ms: Some(1_000),
                    end_ms: Some(3_000),
                }],
                action_items: vec![],
                open_questions: vec![],
                custom_sections: vec![],
                quality_warning: None,
                quality_severity: None,
            },
        )
        .unwrap();
    library.repository().set_mcp_enabled(true).unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_echo-memory-mcp"))
        .env("ECHO_LIBRARY_ROOT", &root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "get_project_context",
            "arguments": { "project_id": project.id }
        }
    });
    writeln!(child.stdin.as_mut().unwrap(), "{request}").unwrap();
    drop(child.stdin.take());
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let response: Value = serde_json::from_slice(&output.stdout).unwrap();
    let content = response["result"]["content"][0]["text"].as_str().unwrap();
    let context: Value = serde_json::from_str(content).unwrap();
    assert_eq!(context["decisions"][0]["text"], "采用桌面应用");
    assert_eq!(context["decisions"][0]["startMs"], 1_000);
    assert_eq!(
        context["decisions"][0]["quoteText"],
        "为了隐私和离线使用，我们决定采用桌面应用。"
    );

    std::fs::remove_dir_all(root).unwrap();
}
