//! 分析输出质量基准（可选本地任务）。
//!
//! 在临时库里跑完整链路：导入基准片段 → 转写 → 用内置模板分析，
//! 把分析 JSON 落盘到 benchmarks/results/，供人工按真值逐条评估
//! （决策是否确有其事、待办是否有未来行动证据、引用是否有效、有无幻觉）。
//!
//! 运行方式：
//! ```bash
//! ECHO_BENCH_ANALYSIS=1 cargo test --features mcp-bin --test analysis_benchmark_test -- --ignored --nocapture
//! ```
//! 前置：本机 Ollama 在线且装有设置里的分析模型（默认 qwen2.5:7b）；
//! 可选的 ECHO_ANALYSIS_MODEL 覆盖模型名。

use echo_memory_lib::analysis::AnalysisDraft;
use echo_memory_lib::commands::{analyze_with_library, transcribe_with_library};
use echo_memory_lib::library::ManagedLibrary;
use serde_json::json;
use std::path::{Path, PathBuf};

fn default_model_path() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("ECHO_WHISPER_MODEL") {
        return Some(PathBuf::from(p));
    }
    let home = std::env::var("HOME").ok()?;
    let dir = Path::new(&home).join("Library/Application Support/回声记忆/models");
    std::fs::read_dir(&dir)
        .ok()?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            let name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            name.ends_with(".bin") && !name.starts_with('.')
        })
        .max_by_key(|path| path.metadata().ok().map(|m| m.len()).unwrap_or(0))
}

#[test]
#[ignore = "requires ECHO_BENCH_ANALYSIS=1, a local Whisper model and local Ollama"]
fn analyzes_benchmark_clip_and_reports_quality() {
    if std::env::var("ECHO_BENCH_ANALYSIS")
        .map(|v| v != "1")
        .unwrap_or(true)
    {
        panic!("分析基准需要 ECHO_BENCH_ANALYSIS=1 显式开启（会调用本机 Ollama 数分钟）");
    }
    let benchmarks_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks");
    let manifest: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(benchmarks_dir.join("manifest.json")).unwrap(),
    )
    .unwrap();
    // 02-medium 是按「会议内容」写的：含已确认安排、待办、开放问题，最适合评模板。
    let clip = manifest["clips"]
        .as_array()
        .unwrap()
        .iter()
        .find(|clip| clip["id"] == "02-medium")
        .expect("manifest 缺 02-medium");

    let model = default_model_path().expect("未找到 Whisper 模型");
    let analysis_model =
        std::env::var("ECHO_ANALYSIS_MODEL").unwrap_or_else(|_| "qwen2.5:7b".to_string());

    let root = std::env::temp_dir().join(format!("echo-analysis-bench-{}", uuid::Uuid::new_v4()));
    let library = ManagedLibrary::open(&root).unwrap();
    let settings = library.repository().knowledge_settings().unwrap();
    library
        .repository()
        .update_knowledge_settings(&echo_memory_lib::types::KnowledgeSettings {
            whisper_model_path: model.display().to_string(),
            transcription_language: "zh".to_string(),
            analysis_model: analysis_model.clone(),
            ..settings
        })
        .unwrap();

    let audio = benchmarks_dir.join(clip["file"].as_str().unwrap());
    let imported = library.import_audio(&audio, None, false).unwrap();
    transcribe_with_library(&library, &imported.record_id).unwrap();
    analyze_with_library(&library, &imported.record_id).unwrap();

    let stored = library
        .repository()
        .latest_analysis(&imported.record_id)
        .unwrap()
        .expect("分析结果缺失");
    let draft: AnalysisDraft = serde_json::from_str(&stored.content_json).unwrap();

    println!("状态: {}", stored.status);
    if let Some(warning) = &draft.quality_warning {
        println!("质量警告: {warning}");
    }
    println!("摘要: {}", draft.summary);
    println!("要点({}):", draft.key_points.len());
    for item in &draft.key_points {
        println!("  - {} | 引用 {:?}", item.text, item.citation_segment_ids);
    }
    println!("决策({}):", draft.decisions.len());
    for item in &draft.decisions {
        println!("  - {} | 引用 {:?}", item.text, item.citation_segment_ids);
    }
    println!("待办({}):", draft.action_items.len());
    for item in &draft.action_items {
        println!("  - {} | 引用 {:?}", item.text, item.citation_segment_ids);
    }
    println!("未解决({}):", draft.open_questions.len());
    for item in &draft.open_questions {
        println!("  - {} | 引用 {:?}", item.text, item.citation_segment_ids);
    }

    let results_dir = benchmarks_dir.join("results");
    std::fs::create_dir_all(&results_dir).unwrap();
    let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
    let out = results_dir.join(format!("{stamp}-analysis-{analysis_model}.json"));
    std::fs::write(
        &out,
        serde_json::to_string_pretty(&json!({
            "clip": "02-medium",
            "analysis_model": analysis_model,
            "status": stored.status,
            "quality_warning": draft.quality_warning,
            "summary": draft.summary,
            "key_points": draft.key_points.iter().map(|i| json!({"text": i.text, "citations": i.citation_segment_ids, "quote": i.quote_text})).collect::<Vec<_>>(),
            "decisions": draft.decisions.iter().map(|i| json!({"text": i.text, "citations": i.citation_segment_ids, "quote": i.quote_text})).collect::<Vec<_>>(),
            "action_items": draft.action_items.iter().map(|i| json!({"text": i.text, "citations": i.citation_segment_ids, "quote": i.quote_text})).collect::<Vec<_>>(),
            "open_questions": draft.open_questions.iter().map(|i| json!({"text": i.text, "citations": i.citation_segment_ids, "quote": i.quote_text})).collect::<Vec<_>>(),
        }))
        .unwrap(),
    )
    .unwrap();
    println!("结果已写入 {}", out.display());

    assert!(!draft.summary.trim().is_empty(), "摘要为空");
    assert!(
        (3..=8).contains(&draft.key_points.len()),
        "要点数不在 3..=8：{}",
        draft.key_points.len()
    );
    assert!(
        draft
            .key_points
            .iter()
            .all(|item| !item.citation_segment_ids.is_empty()),
        "存在没有引用的要点"
    );

    std::fs::remove_dir_all(root).ok();
}
