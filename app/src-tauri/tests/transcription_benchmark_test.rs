//! 转写质量基准跑器（可选本地任务）。
//!
//! 用 benchmarks/ 下真值精确已知的合成音频，对内嵌引擎做可量化的质量评估：
//! 每条片段走完整的 导入 → 预处理 → 分块 → Whisper → 归一化 链路，
//! 自动计算字错误率（CER）与时间轴单调性，结果落盘 benchmarks/results/。
//!
//! 运行方式：
//! ```bash
//! ECHO_BENCH=1 cargo test --features mcp-bin --test transcription_benchmark -- --nocapture
//! ```
//! 可选环境变量：
//! - `ECHO_WHISPER_MODEL`：模型 .bin 路径；缺省时依次尝试
//!   真实库 `~/Library/Application Support/回声记忆/models/` 与 `WHISPER_MODEL_PATH`。
//! - `ECHO_BENCH_REAL`：额外的真实录音路径列表（`;` 分隔），做稳定性对照，
//!   不算 CER（无真值），只记录与时间轴信息。
//!
//! 结果同时写入 `benchmarks/results/<UTC 时间戳>-<模型>.json`，
//! 供 `benchmarks/RESULTS.md` 汇总。

use echo_memory_lib::commands::transcribe_with_library;
use echo_memory_lib::library::ManagedLibrary;
use serde_json::json;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// 字级编辑距离（Levenshtein），用于中文 CER。
fn edit_distance(a: &[char], b: &[char]) -> usize {
    let (rows, cols) = (a.len() + 1, b.len() + 1);
    let mut prev: Vec<usize> = (0..cols).collect();
    let mut curr = vec![0usize; cols];
    for i in 1..rows {
        curr[0] = i;
        for j in 1..cols {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            curr[j] = (prev[j] + 1).min(curr[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[cols - 1]
}

/// CER 的可比口径：去除标点、空白与常见符号，只留文字数字字母。
fn normalize_for_cer(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric() || matches!(*c, '\u{4e00}'..='\u{9fff}'))
        .collect()
}

fn cer(reference: &str, hypothesis: &str) -> Option<f64> {
    let r = normalize_for_cer(reference);
    let h = normalize_for_cer(hypothesis);
    if r.is_empty() {
        return None;
    }
    let dist = edit_distance(
        &r.chars().collect::<Vec<_>>(),
        &h.chars().collect::<Vec<_>>(),
    );
    Some(dist as f64 / r.chars().count() as f64)
}

fn default_model_path() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("ECHO_WHISPER_MODEL") {
        return Some(PathBuf::from(p));
    }
    let home = std::env::var("HOME").ok()?;
    let dir = Path::new(&home).join("Library/Application Support/回声记忆/models");
    let best = std::fs::read_dir(&dir)
        .ok()?
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            name.ends_with(".bin") && !name.starts_with('.')
        })
        .map(|entry| entry.path())
        .max_by_key(|path| path.metadata().ok().map(|m| m.len()).unwrap_or(0))?;
    Some(best)
}

fn sha256_hex(path: &Path) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    if let Ok(bytes) = std::fs::read(path) {
        hasher.update(&bytes);
    }
    format!("{:x}", hasher.finalize())
}

#[test]
#[ignore = "requires ECHO_BENCH=1, a local Whisper model and the benchmarks/ clips"]
fn transcribes_benchmark_clips_and_reports_cer() {
    if std::env::var("ECHO_BENCH")
        .map(|v| v != "1")
        .unwrap_or(true)
    {
        panic!("基准跑器需要 ECHO_BENCH=1 显式开启（会占用本地模型数分钟）");
    }
    let manifest_path = {
        let manifest =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/manifest.json");
        assert!(manifest.is_file(), "找不到 {}", manifest.display());
        manifest
    };
    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&manifest_path).unwrap()).unwrap();
    let benchmarks_dir = manifest_path.parent().unwrap().to_path_buf();

    let model = default_model_path()
        .expect("未找到 Whisper 模型（设置 ECHO_WHISPER_MODEL 或放入真实库 models/）");
    println!("使用模型: {}", model.display());

    let root = std::env::temp_dir().join(format!("echo-memory-bench-{}", uuid::Uuid::new_v4()));
    let library = ManagedLibrary::open(&root).unwrap();
    // 模型路径写进临时库设置：转写链路只认设置里的 whisper_model_path。
    library
        .repository()
        .update_knowledge_settings(&echo_memory_lib::types::KnowledgeSettings {
            whisper_model_path: model.display().to_string(),
            ..library.repository().knowledge_settings().unwrap()
        })
        .unwrap();

    let mut results = Vec::new();
    for clip in manifest["clips"].as_array().expect("manifest.clips") {
        let id = clip["id"].as_str().unwrap().to_string();
        let audio = benchmarks_dir.join(clip["file"].as_str().unwrap());
        let language = clip["language"].as_str().unwrap_or("zh").to_string();
        let reference = clip["text"].as_str().unwrap_or("").to_string();

        let imported = library.import_audio(&audio, None, false).unwrap();
        // 基准片段的语言写进设置，避免语言「自动」带来的方差。
        library
            .repository()
            .update_knowledge_settings(&echo_memory_lib::types::KnowledgeSettings {
                transcription_language: language.clone(),
                ..library.repository().knowledge_settings().unwrap()
            })
            .unwrap();

        let started = Instant::now();
        // 质量闸门（degeneration）判定退化时 transcribe_with_library 会返回错误——
        // 对基准来说这本身就是有效结果：记录失败原因，不要 panic。
        let transcribe_result = transcribe_with_library(&library, &imported.record_id);
        let elapsed = started.elapsed();

        if let Err(error) = transcribe_result {
            println!("{}  转写被拒/失败: {error}", id);
            results.push(json!({
                "id": id,
                "model": model.file_name().unwrap().to_string_lossy(),
                "language": language,
                "duration_ms": clip["duration_ms"],
                "elapsed_ms": elapsed.as_millis() as u64,
                "accepted": false,
                "error": error.to_string(),
                "reference": reference,
            }));
            continue;
        }

        let segments = library
            .repository()
            .list_transcript_segments(&imported.record_id)
            .unwrap();
        let hypothesis: String = segments
            .iter()
            .filter_map(|segment| segment.normalized_text.as_deref())
            .collect::<Vec<_>>()
            .join("");
        let score = cer(&reference, &hypothesis);
        let monotonic = segments
            .windows(2)
            .all(|pair| pair[1].start_ms >= pair[0].start_ms && pair[1].end_ms >= pair[0].end_ms);

        let entry = json!({
            "id": id,
            "model": model.file_name().unwrap().to_string_lossy(),
            "language": language,
            "duration_ms": clip["duration_ms"],
            "elapsed_ms": elapsed.as_millis() as u64,
            "cer": score,
            "segment_count": segments.len(),
            "timeline_monotonic": monotonic,
            "hypothesis": hypothesis,
            "reference": reference,
        });
        println!(
            "{}  CER={}  分段={}  用时={}s  时间轴单调={}",
            id,
            score
                .map(|v| format!("{:.1}%", v * 100.0))
                .unwrap_or("-".into()),
            segments.len(),
            elapsed.as_secs_f32(),
            monotonic
        );
        println!("  识别: {hypothesis}");
        results.push(entry);
    }

    // 可选：真实录音稳定性对照（无真值，只记录转写与时间轴）。
    // 语言必须显式设置：跑完片段循环后设置里残留的是最后一条片段的语言
    // （英文），不设置就会用英语去转中文录音——whisper 对错语言输入会产生
    // 循环幻觉，2026-09-21 的「P0 复现」整个是这个 harness 缺陷造成的假象。
    if let Ok(extra) = std::env::var("ECHO_BENCH_REAL") {
        let real_language =
            std::env::var("ECHO_BENCH_REAL_LANG").unwrap_or_else(|_| "zh".to_owned());
        library
            .repository()
            .update_knowledge_settings(&echo_memory_lib::types::KnowledgeSettings {
                transcription_language: real_language.clone(),
                ..library.repository().knowledge_settings().unwrap()
            })
            .unwrap();
        for path in extra.split(';').filter(|p| !p.trim().is_empty()) {
            let path = PathBuf::from(path.trim());
            let imported = library.import_audio(&path, None, false).unwrap();
            let started = Instant::now();
            // 质量闸门判定退化时返回错误——对真实录音对照来说同样是有效结果。
            let transcribe = transcribe_with_library(&library, &imported.record_id);
            let elapsed = started.elapsed();
            let segments = library
                .repository()
                .list_transcript_segments(&imported.record_id)
                .unwrap_or_default();
            let hypothesis: String = segments
                .iter()
                .filter_map(|segment| segment.normalized_text.as_deref())
                .collect::<Vec<_>>()
                .join("");
            results.push(json!({
                "id": format!("real-{}", path.file_stem().unwrap_or_default().to_string_lossy()),
                "model": model.file_name().unwrap().to_string_lossy(),
                "language": real_language,
                "elapsed_ms": elapsed.as_millis() as u64,
                "accepted": transcribe.is_ok(),
                "error": transcribe.as_ref().err().map(|error| error.to_string()),
                "segment_count": segments.len(),
                "hypothesis": hypothesis,
            }));
        }
    }

    let results_dir = benchmarks_dir.join("results");
    std::fs::create_dir_all(&results_dir).unwrap();
    let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
    let model_sha = sha256_hex(&model);
    let out = results_dir.join(format!("{stamp}-{}.json", &model_sha[..8]));
    std::fs::write(
        &out,
        serde_json::to_string_pretty(&json!({
            "model_path": model.display().to_string(),
            "model_sha256_prefix": &model_sha[..16],
            "results": results,
        }))
        .unwrap(),
    )
    .unwrap();
    println!("结果已写入 {}", out.display());

    std::fs::remove_dir_all(root).ok();
}
