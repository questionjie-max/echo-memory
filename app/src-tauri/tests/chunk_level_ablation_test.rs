//! 分块粒度消融：定位增强管线里到底哪个部件有毒（分钟级，不用跑完整文件）。
//!
//! 已知（2026-09-21）：同一文件、同一模型、同一预处理——整文件单次解码 1310 段
//! 通顺中文（99%），分块管线 467 段英文幻觉循环。本测试按块的边界逐块解码，
//! 区分「短输入本身有毒」与「滚动 prompt 传染」。
//!
//! ```bash
//! ECHO_ABLATE=1 ECHO_TEST_AUDIO=<m4a> ECHO_WHISPER_MODEL=<bin> \
//!   cargo test --features mcp-bin --test chunk_level_ablation -- --ignored --nocapture
//! ```

use echo_memory_lib::audio::{self, plan_chunks, read_normalized_wav};
use echo_memory_lib::whisper::{WhisperAdapter, WhisperSession};
use std::path::PathBuf;

fn brief(text: &str, n: usize) -> String {
    text.chars().take(n).collect()
}

/// 会话复用的对照实验：同一块音频用四种方式解码，找出复用到底改了什么。
///   fresh   —— 每次重新加载模型（现状基准）
///   first   —— 新会话的第一次调用（无历史）
///   third   —— 同一会话解过前两块之后的第三次调用（有历史）
///   third_again —— 同一会话紧接着再解同一块（历史 = 自己）
/// 只有 third 与 fresh 不同，才能把差异归因到「历史残留」。
#[test]
#[ignore = "requires ECHO_ABLATE=1, ECHO_TEST_AUDIO and a Whisper model"]
fn session_reuse_matches_fresh_loads() {
    if std::env::var("ECHO_ABLATE")
        .map(|v| v != "1")
        .unwrap_or(true)
    {
        panic!("消融测试需要 ECHO_ABLATE=1 显式开启");
    }
    let audio_path = PathBuf::from(std::env::var("ECHO_TEST_AUDIO").expect("ECHO_TEST_AUDIO"));
    let model = PathBuf::from(std::env::var("ECHO_WHISPER_MODEL").expect("ECHO_WHISPER_MODEL"));

    let scratch = std::env::temp_dir().join(format!("echo-session-reuse-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&scratch).unwrap();
    let wav = scratch.join("p.wav");
    audio::preprocess(&audio_path, &wav).unwrap();
    let samples = read_normalized_wav(&wav).unwrap();
    let chunks = plan_chunks(&samples);

    let adapter = WhisperAdapter::detect_with_model_path(Some(model.to_str().unwrap())).unwrap();
    let slice_of = |index: usize| &samples[chunks[index].sample_start..chunks[index].sample_end];

    // 计时：验证「context 复用 + state 每块新建」相对「整个 context 每块重建」的收益。
    let started = std::time::Instant::now();
    let fresh = adapter.transcribe_samples(slice_of(2), "zh", None).unwrap();
    let fresh_secs = started.elapsed().as_secs_f32();
    let fresh_text: String = fresh.iter().map(|s| s.text.as_str()).collect();

    let session = WhisperSession::load(&model).unwrap();
    let started = std::time::Instant::now();
    let first = session.transcribe(slice_of(2), "zh", None).unwrap();
    let first_secs = started.elapsed().as_secs_f32();
    let first_text: String = first.iter().map(|s| s.text.as_str()).collect();

    // 同一会话解过前两块之后再解第三块：历史不得改变结果。
    session.transcribe(slice_of(0), "zh", None).unwrap();
    session.transcribe(slice_of(1), "zh", None).unwrap();
    let third = session.transcribe(slice_of(2), "zh", None).unwrap();
    let third_text: String = third.iter().map(|s| s.text.as_str()).collect();
    let third_again = session.transcribe(slice_of(2), "zh", None).unwrap();
    let third_again_text: String = third_again.iter().map(|s| s.text.as_str()).collect();

    println!("fresh      ({fresh_secs:.1}s): {fresh_text}");
    println!("first      ({first_secs:.1}s): {first_text}");
    println!("third      : {third_text}");
    println!("third_again: {third_again_text}");
    println!(
        "fresh==first {} | fresh==third {} | fresh==third_again {} | third==third_again {}",
        fresh_text == first_text,
        fresh_text == third_text,
        fresh_text == third_again_text,
        third_text == third_again_text
    );
    for (label, other) in [
        ("first", &first),
        ("third", &third),
        ("third_again", &third_again),
    ] {
        assert_eq!(
            fresh.len(),
            other.len(),
            "{label} 段数与 fresh 不一致：fresh={} {label}={}",
            fresh.len(),
            other.len()
        );
    }

    // 冷启动的第一次调用必须和重新加载模型逐字一致：这是会话复用的安全底线。
    assert_eq!(
        first_text, fresh_text,
        "新会话首次解码就与重新加载不一致，会话复用本身有问题"
    );
    // 有历史之后也必须一致，否则说明前几次解码的状态泄漏到了这一次。
    assert_eq!(
        third_text, fresh_text,
        "解过前两块之后，同一块音频的解码结果变了——历史状态泄漏"
    );
    assert_eq!(
        third_again_text, fresh_text,
        "同一块连解两次结果不同——历史状态泄漏"
    );

    std::fs::remove_dir_all(&scratch).ok();
}

#[test]
#[ignore = "requires ECHO_ABLATE=1, ECHO_TEST_AUDIO and a Whisper model"]
fn decode_first_chunks_one_by_one() {
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

    let scratch = std::env::temp_dir().join(format!("echo-chunk-ablate-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&scratch).unwrap();
    let wav = scratch.join("p.wav");
    audio::preprocess(&audio_path, &wav).unwrap();
    let samples = read_normalized_wav(&wav).unwrap();
    let chunks = plan_chunks(&samples);
    println!("分块数: {}", chunks.len());

    let adapter = WhisperAdapter::detect_with_model_path(Some(model.to_str().unwrap())).unwrap();

    // 逐块解码，前 4 块 + 最后 1 块；每块分别试「无 prompt」与「带上一块尾巴 prompt」。
    let mut last_tail = String::new();
    let indices: Vec<usize> = {
        let mut v: Vec<usize> = (0..chunks.len().min(4)).collect();
        if chunks.len() > 5 {
            v.push(chunks.len() - 1);
        }
        v
    };
    for index in indices {
        let chunk = &chunks[index];
        let slice = &samples[chunk.sample_start..chunk.sample_end];
        let without = adapter.transcribe_samples(slice, "zh", None).unwrap();
        let without_text: String = without.iter().map(|s| s.text.clone()).collect();
        let report_without = echo_memory_lib::degeneration::assess_transcript_degeneration(
            &without.iter().map(|s| s.text.clone()).collect::<Vec<_>>(),
            "zh",
        );
        println!(
            "块{index} [{:.0}s-{:.0}s] 无prompt: {}段 中文占比{:.0}% 重复率{:.0}% 退化={} | {}",
            chunk.accept_start_ms as f32 / 1000.0,
            chunk.accept_end_ms as f32 / 1000.0,
            without.len(),
            report_without.cjk_ratio * 100.0,
            report_without.repeat_ratio * 100.0,
            report_without.degenerate,
            brief(&without_text, 80)
        );

        if !last_tail.is_empty() {
            let with = adapter
                .transcribe_samples(slice, "zh", Some(&last_tail))
                .unwrap();
            let with_text: String = with.iter().map(|s| s.text.clone()).collect();
            let report_with = echo_memory_lib::degeneration::assess_transcript_degeneration(
                &with.iter().map(|s| s.text.clone()).collect::<Vec<_>>(),
                "zh",
            );
            println!(
                "块{index} 带prompt: {}段 中文占比{:.0}% 重复率{:.0}% 退化={} | {}",
                with.len(),
                report_with.cjk_ratio * 100.0,
                report_with.repeat_ratio * 100.0,
                report_with.degenerate,
                brief(&with_text, 80)
            );
        }
        // 滚动 prompt 的构造方式与 commands.rs 一致：上一块已接受文本尾 80 字符。
        last_tail = without_text
            .chars()
            .rev()
            .take(80)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
    }

    std::fs::remove_dir_all(&scratch).ok();
}
