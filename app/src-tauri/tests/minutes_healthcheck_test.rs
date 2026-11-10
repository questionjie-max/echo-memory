//! 块6：纪要质量体检（可重复跑的离线体检工具，测试/开发层落点，无产品 UI）。
//!
//! 用法：`ECHO_MINUTES_HEALTHCHECK=1 cargo test --manifest-path app/src-tauri/Cargo.toml --test minutes_healthcheck_test -- --nocapture`
//!
//! 用固定 fixture 逐字稿集（短会 / 术语密集 / 长稿）跑事后纪要生成（当前已选定的
//! 本机模型通道，v1.1 实测为 Ollama qwen2.5:7b），输出可比报告：
//! - 每类各多少条议题/结论/待办、证据命中率、dropped 数、耗时、每 10 句条目密度；
//! - 报告落 `benchmarks/results/minutes-healthcheck/`（仓库 .gitignore 已排除该目录，
//!   报告只含 fixture 衍生内容，不喂任何真实资料）：latest.json（机器可读）+ latest.md（人可读）；
//! - 重跑时与上一份 latest.json 并排对比（md 里有 Δ 列，历史快照留 history/）。
//!
//! 红线：不依赖网络（只用本机 Ollama 通道）、不喂任何真实内容（只用仓库 fixture）；
//! 体检默认跳过（env 门控），失败不影响产品构建与测试全绿。
use echo_memory_lib::library::ManagedLibrary;
use echo_memory_lib::minutes::{generate_minutes_document, MinutesGenerator};
use echo_memory_lib::types::TranscriptSegment;

/// (fixture id, 类别, 时长 ms)。短会 / 术语密集（柳比歇夫、SHA256、截止/截至）/ 长稿。
const FIXTURES: &[(&str, &str, i64)] = &[
    ("01-short", "短会", 24_660),
    ("02-medium", "术语密集", 82_660),
    ("06-long", "长稿", 239_513),
];

fn build_segments(text: &str, duration_ms: i64) -> Vec<TranscriptSegment> {
    let sentences: Vec<&str> = text
        .split(|c: char| c == '。' || c == '！' || c == '？' || c == '\n')
        .filter(|s| !s.trim().is_empty())
        .collect();
    let total: usize = sentences.iter().map(|s| s.chars().count()).sum();
    let mut segments = Vec::new();
    let mut cursor = 0i64;
    for (index, sentence) in sentences.iter().enumerate() {
        let span = sentence.chars().count() as i64 * duration_ms / total.max(1) as i64;
        segments.push(TranscriptSegment {
            id: format!("s{index}"),
            record_id: "healthcheck".into(),
            sequence: index as i64,
            speaker_label: None,
            start_ms: cursor,
            end_ms: cursor + span.max(200),
            original_text: (*sentence).to_owned(),
            normalized_text: None,
            normalization_version: None,
            edited_text: None,
        });
        cursor += span.max(200);
    }
    segments
}

#[test]
fn minutes_healthcheck_report() {
    // env 门控：默认跳过，不进常规测试盘子（体检失败绝不影响全绿）。
    if std::env::var("ECHO_MINUTES_HEALTHCHECK").as_deref() != Ok("1") {
        return;
    }
    let clips_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/clips");
    let out_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../benchmarks/results/minutes-healthcheck");
    std::fs::create_dir_all(out_dir.join("history")).unwrap();

    // 本机模型通道（云端未配置时自动落到 Ollama；体检不配置、不依赖任何云）。
    let library = ManagedLibrary::open(std::env::temp_dir().join(format!("echo-hc-{}", uuid::Uuid::new_v4()))).unwrap();
    let generator = MinutesGenerator::detect(&library)
        .unwrap_or_else(|error| panic!("体检需要可用的本机模型通道（未配置云、未检测到 Ollama）：{error}"));
    let provider = generator.provider_name().to_owned();
    let model = generator.model_name().to_owned();

    let mut rows: Vec<serde_json::Value> = Vec::new();
    for (id, category, duration_ms) in FIXTURES {
        let text = std::fs::read_to_string(clips_dir.join(format!("{id}.txt")))
            .unwrap_or_else(|error| panic!("fixture 真值应存在 {id}: {error}"));
        let segments = build_segments(&text, *duration_ms);
        let started = std::time::Instant::now();
        let document = generate_minutes_document(&segments, *duration_ms, |prompt| {
            let draft = generator.generate_minutes_json(prompt)?;
            serde_json::to_string(&draft).map_err(|e| echo_memory_lib::error::AppError::Invalid(e.to_string()))
        })
        .unwrap_or_else(|error| panic!("体检生成失败 {id}: {error}"));
        let elapsed_ms = started.elapsed().as_millis() as u64;
        let sentences = segments.len();
        let items = document.conclusions.len() + document.action_items.len();
        let generated_total = items + document.dropped_unverifiable;
        let hit_rate = if generated_total > 0 {
            items as f64 / generated_total as f64
        } else {
            1.0
        };
        let per_10 = if sentences > 0 {
            items as f64 * 10.0 / sentences as f64
        } else {
            0.0
        };
        rows.push(serde_json::json!({
            "id": id,
            "category": category,
            "sentences": sentences,
            "topics": document.topics.len(),
            "conclusions": document.conclusions.len(),
            "actionItems": document.action_items.len(),
            "dropped": document.dropped_unverifiable,
            "evidenceHitRate": (hit_rate * 1000.0).round() / 1000.0,
            "per10": (per_10 * 100.0).round() / 100.0,
            "elapsedMs": elapsed_ms,
        }));
        println!(
            "HC {id}[{category}] 句={} 议题={} 结论={} 待办={} dropped={} 命中率={:.1}% 耗时={}ms",
            sentences,
            document.topics.len(),
            document.conclusions.len(),
            document.action_items.len(),
            document.dropped_unverifiable,
            hit_rate * 100.0,
            elapsed_ms
        );
    }

    let previous = std::fs::read_to_string(out_dir.join("latest.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok());

    let report = serde_json::json!({
        "generatedAt": chrono::Utc::now().to_rfc3339(),
        "provider": provider,
        "model": model,
        "fixtures": rows,
    });
    std::fs::write(
        out_dir.join("latest.json"),
        serde_json::to_string_pretty(&report).unwrap(),
    )
    .unwrap();

    // 人可读 md：本轮 + 与上一轮并排对比（Δ 列）。
    let mut md = String::new();
    md.push_str(&format!(
        "# 纪要质量体检报告\n\n- 生成时间：{}\n- 通道：{provider} / {model}\n- 条件：仓库 fixture 真值 → 比例时间轴 → 事后纪要全流程（分块→生成→证据回链 fail-closed）\n\n",
        chrono::Utc::now().to_rfc3339()
    ));
    md.push_str("| fixture | 类别 | 句数 | 议题 | 结论 | 待办 | dropped | 证据命中率 | 每10句条目 | 耗时 |\n|---|---|---|---|---|---|---|---|---|---|\n");
    for row in &rows {
        md.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} | {:.1}% | {} | {}ms |\n",
            row["id"].as_str().unwrap_or(""),
            row["category"].as_str().unwrap_or(""),
            row["sentences"],
            row["topics"],
            row["conclusions"],
            row["actionItems"],
            row["dropped"],
            row["evidenceHitRate"].as_f64().unwrap_or(0.0) * 100.0,
            row["per10"].as_f64().unwrap_or(0.0),
            row["elapsedMs"],
        ));
    }
    if let Some(previous) = &previous {
        md.push_str("\n## 与上一轮对比（Δ = 本轮 − 上轮）\n\n");
        md.push_str("| fixture | 议题Δ | 结论Δ | 待办Δ | droppedΔ | 耗时Δ |\n|---|---|---|---|---|---|\n");
        let previous_rows = previous["fixtures"].as_array().cloned().unwrap_or_default();
        for row in &rows {
            let id = row["id"].as_str().unwrap_or("");
            let previous_row = previous_rows
                .iter()
                .find(|previous_row| previous_row["id"].as_str() == Some(id));
            let delta = |key: &str| -> String {
                match previous_row.map(|previous_row| previous_row[key].as_i64()) {
                    Some(Some(previous_value)) => {
                        let delta = row[key].as_i64().unwrap_or(0) - previous_value;
                        if delta > 0 { format!("+{delta}") } else { format!("{delta}") }
                    }
                    _ => "—".to_owned(),
                }
            };
            md.push_str(&format!(
                "| {} | {} | {} | {} | {} | {} |\n",
                id,
                delta("topics"),
                delta("conclusions"),
                delta("actionItems"),
                delta("dropped"),
                delta("elapsedMs"),
            ));
        }
        if let Some(previous_at) = previous["generatedAt"].as_str() {
            md.push_str(&format!("\n上一轮报告时间：{previous_at}（模型 {}/{}）\n",
                previous["provider"].as_str().unwrap_or("?"),
                previous["model"].as_str().unwrap_or("?")));
        }
    }
    std::fs::write(out_dir.join("latest.md"), md).unwrap();

    // 历史快照按时间留档。
    let stamp = chrono::Utc::now().format("%Y%m%d-%H%M%S");
    std::fs::write(
        out_dir.join("history").join(format!("{stamp}.json")),
        serde_json::to_string_pretty(&report).unwrap(),
    )
    .unwrap();
    println!("HC 报告已写入 {}", out_dir.display());
}
