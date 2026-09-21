//! whisperX 说话人分离端到端验证（可选本地任务，首次真实跑通此链路的测试）。
//!
//! 用基准片段 03-dialogue（双人对话、轮次真值已知）走完整链路：
//! 导入 → whisperX 转写（--diarize）→ 说话人标签本地化入库，
//! 校验「至少两个说话人、时间轴单调、标签为说话人 N」，并把轮次打出来与真值对照。
//!
//! 运行方式：
//! ```bash
//! ECHO_E2E_WHISPERX=1 ECHO_HF_TOKEN=hf_xxx \
//!   cargo test --features mcp-bin --test whisperx_e2e_test -- --ignored --nocapture
//! ```
//! 前置：
//! - 本机 PATH 或常见安装位有 whisperx（`uv tool install --python 3.11 whisperx`）；
//! - NLTK 的 punkt_tab 数据随应用分发（resources/nltk_data，转写时由
//!   transcribe_whisperx 显式设 NLTK_DATA），无需手动预置；
//! - HuggingFace token 且已接受 pyannote/speaker-diarization-3.1 与
//!   pyannote/segmentation-3.0 的使用协议（也可经 ECHO_HF_TOKEN 注入）。
//!
//! 首次运行会下载 whisperX 转写模型与 pyannote 分离模型（合计约 2GB）。

use echo_memory_lib::commands::transcribe_with_library;
use echo_memory_lib::library::ManagedLibrary;
use serde_json::json;
use std::path::PathBuf;

#[test]
#[ignore = "requires ECHO_E2E_WHISPERX=1, installed whisperx, NLTK data and a HuggingFace token"]
fn whisperx_separates_two_speakers_on_the_dialogue_clip() {
    if std::env::var("ECHO_E2E_WHISPERX")
        .map(|v| v != "1")
        .unwrap_or(true)
    {
        panic!("whisperX e2e 需要 ECHO_E2E_WHISPERX=1 显式开启（会下载约 2GB 模型并占用数分钟）");
    }
    assert!(
        echo_memory_lib::whisper::whisperx_path().is_some(),
        "未检测到 whisperx 命令（PATH 与常见安装位都没有）"
    );
    assert!(
        echo_memory_lib::memory::get_hf_token()
            .ok()
            .flatten()
            .is_some(),
        "未配置 HuggingFace token（ECHO_HF_TOKEN 或应用钥匙串）"
    );

    let benchmarks_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks");
    let manifest: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(benchmarks_dir.join("manifest.json")).unwrap(),
    )
    .unwrap();
    let clip = manifest["clips"]
        .as_array()
        .unwrap()
        .iter()
        .find(|clip| clip["id"] == "03-dialogue")
        .expect("manifest 缺 03-dialogue");
    let ground_truth_turns = clip["turns"]
        .as_array()
        .expect("03-dialogue 缺轮次真值")
        .iter()
        .map(|turn| {
            (
                turn["speaker"].as_str().unwrap_or("").to_string(),
                turn["text"].as_str().unwrap_or("").to_string(),
            )
        })
        .collect::<Vec<_>>();

    let root = std::env::temp_dir().join(format!("echo-whisperx-e2e-{}", uuid::Uuid::new_v4()));
    let library = ManagedLibrary::open(&root).unwrap();
    let settings = library.repository().knowledge_settings().unwrap();
    library
        .repository()
        .update_knowledge_settings(&echo_memory_lib::types::KnowledgeSettings {
            transcription_language: "zh".to_string(),
            ..settings
        })
        .unwrap();
    // 引擎是独立设置项（不是 KnowledgeSettings 的字段）。
    library
        .repository()
        .set_setting_value("transcription_engine", "whisperx")
        .unwrap();

    let audio = benchmarks_dir.join(clip["file"].as_str().unwrap());
    let imported = library.import_audio(&audio, None, false).unwrap();
    transcribe_with_library(&library, &imported.record_id).unwrap();

    let segments = library
        .repository()
        .list_transcript_segments(&imported.record_id)
        .unwrap();
    assert!(!segments.is_empty(), "whisperX 没有产出分段");
    assert!(
        segments
            .windows(2)
            .all(|pair| pair[1].start_ms >= pair[0].start_ms),
        "时间轴不单调"
    );

    let labels = segments
        .iter()
        .filter_map(|segment| segment.speaker_label.clone())
        .collect::<Vec<_>>();
    let distinct: std::collections::BTreeSet<_> = labels.iter().collect();
    println!("说话人标签集合: {distinct:?}");
    for (index, segment) in segments.iter().enumerate() {
        println!(
            "  [{}] {} | {}",
            index,
            segment.speaker_label.as_deref().unwrap_or("-"),
            segment
                .normalized_text
                .clone()
                .unwrap_or_else(|| segment.original_text.clone())
        );
    }
    println!("真值轮次:");
    for (speaker, text) in &ground_truth_turns {
        println!("  {speaker}: {text}");
    }

    assert!(
        labels.iter().all(|label| label.contains("说话人")),
        "说话人标签未本地化: {labels:?}"
    );
    assert!(
        distinct.len() >= 2,
        "只识别出 {} 个说话人，分离失败",
        distinct.len()
    );

    let results_dir = benchmarks_dir.join("results");
    std::fs::create_dir_all(&results_dir).unwrap();
    let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
    let out = results_dir.join(format!("{stamp}-whisperx-diarize.json"));
    std::fs::write(
        &out,
        serde_json::to_string_pretty(&json!({
            "clip": "03-dialogue",
            "segments": segments.iter().map(|s| json!({
                "start_ms": s.start_ms,
                "end_ms": s.end_ms,
                "speaker": s.speaker_label,
                "text": s.normalized_text,
            })).collect::<Vec<_>>(),
            "ground_truth_turns": ground_truth_turns.iter().map(|(speaker, text)| json!({"speaker": speaker, "text": text})).collect::<Vec<_>>(),
        }))
        .unwrap(),
    )
    .unwrap();
    println!("结果已写入 {}", out.display());

    std::fs::remove_dir_all(root).ok();
}
