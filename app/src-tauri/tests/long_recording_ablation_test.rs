//! 长录音幻觉归因消融（可选本地任务，阶段一 P0 专用）。
//!
//! 已排除：ffmpeg 滤波/loudnorm（`ECHO_FFMPEG_BIN` 指空走 afconvert 路径，
//! 37 分钟样本依旧 467 段英文循环——见 benchmarks/results/20260921T114058Z-*.json）。
//!
//! 本测试消融剩下的增强管线部件：**整文件单次解码、无初始 prompt、无 accept 窗口、
//! 无重叠去重**——等价于 2026-07-18 产出 1538 段通顺转写时的调用形态
//! （那版代码未进 git，无法直接 diff，只能复现调用方式）。
//!
//! 运行：
//! ```bash
//! ECHO_ABLATE=1 ECHO_TEST_AUDIO=/path/to/37min.m4a \
//!   ECHO_WHISPER_MODEL=/path/to/ggml-*.bin \
//!   cargo test --features mcp-bin --test long_recording_ablation -- --ignored --nocapture
//! ```
//! 判读：若本测试输出通顺中文而基准跑器（分块+滚动 prompt）输出幻觉 →
//! 根因在增强管线（分块/滚动 prompt）；若依旧幻觉 → 根因在解码参数或模型本身。

use echo_memory_lib::audio::{self, read_normalized_wav};
use echo_memory_lib::whisper::WhisperAdapter;
use std::path::PathBuf;

#[test]
#[ignore = "requires ECHO_ABLATE=1, ECHO_TEST_AUDIO and a Whisper model"]
fn whole_file_single_decode_without_prompt() {
    if std::env::var("ECHO_ABLATE")
        .map(|v| v != "1")
        .unwrap_or(true)
    {
        panic!("消融测试需要 ECHO_ABLATE=1 显式开启");
    }
    let audio_path = PathBuf::from(std::env::var("ECHO_TEST_AUDIO").expect("ECHO_TEST_AUDIO"));
    let model = std::env::var("ECHO_WHISPER_MODEL")
        .map(PathBuf::from)
        .expect("ECHO_WHISPER_MODEL");

    let scratch = std::env::temp_dir().join(format!("echo-ablate-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&scratch).unwrap();
    let wav = scratch.join("preprocessed.wav");
    let metadata = audio::preprocess(&audio_path, &wav).unwrap();
    println!("预处理: {metadata:?}");
    let samples = read_normalized_wav(&wav).unwrap();
    println!(
        "采样点数: {}（约 {:.1} 分钟）",
        samples.len(),
        samples.len() as f32 / 16_000.0 / 60.0
    );

    let adapter = WhisperAdapter::detect_with_model_path(Some(model.to_str().unwrap())).unwrap();
    let started = std::time::Instant::now();
    let segments = adapter.transcribe_samples(&samples, "zh", None).unwrap();
    let elapsed = started.elapsed();

    let text: String = segments.iter().map(|s| s.text.clone()).collect();
    let chars: Vec<char> = text.chars().filter(|c| c.is_alphanumeric()).collect();
    let cjk = chars
        .iter()
        .filter(|c| matches!(**c, '\u{4e00}'..='\u{9fff}'))
        .count();
    println!(
        "整文件单次解码：{} 段，用时 {:.0}s，字符 {}，中文占比 {:.0}%",
        segments.len(),
        elapsed.as_secs_f32(),
        chars.len(),
        cjk as f64 / chars.len().max(1) as f64 * 100.0
    );
    println!("前 400 字: {}", text.chars().take(400).collect::<String>());
    println!(
        "中段 300 字: {}",
        text.chars().skip(1200).take(300).collect::<String>()
    );
    let report = echo_memory_lib::degeneration::assess_transcript_degeneration(
        &segments.iter().map(|s| s.text.clone()).collect::<Vec<_>>(),
        "zh",
    );
    println!("退化检测: {report:?}");

    std::fs::remove_dir_all(&scratch).ok();
}
