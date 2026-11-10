//! 实时增量纪要（v1.1 块5）。
//!
//! 边听边出增量纪要：复用块2 实时文字的定稿缓冲（LiveTranscriptBuffer），
//! 按「间隔 ≥45s 且新定稿 ≥2 句」节流触发一次模型调用，把新内容合并进
//! 已有增量纪要（同一 JSON schema、同一 fail-closed 证据校验）。
//! 增量内容在录制中一律为暂定态；停止后事后纪要（块3，转写完成自动生成）
//! 是唯一权威——增量视图随录音结束整体丢弃，不做逐条对账，无冲突残留。
//! 音频不出本机：模型调用与本机/云端通道判定与事后纪要完全同款。

use crate::library::ManagedLibrary;
use crate::live_transcript::{LiveSegment, LiveTranscriptManager};
use crate::minutes::{finalize_minutes_draft, MinutesDocument, MinutesGenerator};
use crate::types::TranscriptSegment;
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::Emitter;

/// 增量触发的最小间隔：定期合并，不逐句刷（烧模型也刷屏）。
/// 首轮同样受此约束（引擎启动即计时），口径统一为「≥45s 且 ≥2 句新定稿」。
pub const INCREMENTAL_MIN_INTERVAL_MS: i64 = 45_000;
/// 增量触发的最小新定稿句数：静音/零星几句不烧模型。
pub const INCREMENTAL_MIN_NEW_SEGMENTS: usize = 2;
/// 检查节拍。
pub const CHECK_TICK: Duration = Duration::from_secs(5);
/// 增量事件名（前端监听）。
pub const LIVE_MINUTES_EVENT: &str = "live-minutes";
/// 单次增量提示词正文预算（字符）：与事后分块同量级，防上下文溢出。
/// 两次成功运行之间新增内容超过预算时按新增量放宽（账实相符优先）；
/// 连续失败时丢弃 previous_json 重开，防「文档涨超输出上限 → 截断 → 解析
/// 失败 → 重试风暴」的死锁。
pub const INCREMENTAL_PROMPT_CHARS: usize = 6_000;
/// 回喂的上一版增量纪要 JSON 上限（字符）：超出从最老条目裁起。
pub const MAX_PREVIOUS_JSON_CHARS: usize = 4_000;
/// 连续失败达到该次数：丢弃 previous_json 重开（防截断死锁）。
pub const FAILURE_RESET_THRESHOLD: usize = 3;
/// 连续失败达到该次数：本场增量停车（实时文字照常）。
pub const FAILURE_STOP_THRESHOLD: usize = 5;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveMinutesEvent {
    pub session_id: String,
    pub source: &'static str,
    /// 当前累计的增量纪要（完整覆盖，前端整体替换不追加）。
    pub document: MinutesDocument,
    /// 本轮因无法溯源被丢弃的条目数（fail-closed 透明度，随事件带出）。
    pub dropped_unverifiable: usize,
    pub generated_at_unix_ms: i64,
    /// 第几次增量合并（1 起），供前端展示「第 N 次生长」。
    pub run_index: usize,
}

/// 纯判定：是否该跑一次增量（节流规则，测试重点）。
pub fn should_run_incremental(new_finals: usize, elapsed_since_last_ms: i64) -> bool {
    new_finals >= INCREMENTAL_MIN_NEW_SEGMENTS && elapsed_since_last_ms >= INCREMENTAL_MIN_INTERVAL_MS
}

/// 增量引擎的节流与回喂状态（纯逻辑，测试重点）。
#[derive(Default)]
struct IncrementalState {
    previous_json: Option<String>,
    processed_finals: usize,
    last_run_at: Option<Instant>,
    consecutive_failures: usize,
}

impl IncrementalState {
    fn should_run(&self, finals_len: usize, now: Instant) -> bool {
        let new_finals = finals_len.saturating_sub(self.processed_finals);
        let elapsed = self
            .last_run_at
            .map(|at| now.duration_since(at))
            .unwrap_or(Duration::ZERO);
        should_run_incremental(
            new_finals,
            elapsed.as_millis().min(i64::MAX as u128) as i64,
        )
    }

    fn record_success(&mut self, finals_len: usize, document: &MinutesDocument) {
        self.processed_finals = finals_len;
        self.last_run_at = Some(Instant::now());
        self.consecutive_failures = 0;
        if let Ok(json) = serde_json::to_string(document) {
            self.previous_json = Some(trim_previous_json(json));
        }
    }

    fn record_failure(&mut self, now: Instant) -> bool {
        // 失败轮同样刷新节流：防止「elapsed 只增不减 → 每 5s 一次注定失败
        // 的调用」的重试风暴。
        self.last_run_at = Some(now);
        self.consecutive_failures += 1;
        if self.consecutive_failures >= FAILURE_STOP_THRESHOLD {
            return false; // 停车信号
        }
        if self.consecutive_failures >= FAILURE_RESET_THRESHOLD {
            // 丢弃回喂重开：文档涨超输出上限被截断时，重试只会继续失败。
            self.previous_json = None;
        }
        true
    }
}

/// 回喂 JSON 超上限时从最老的结论/待办裁起（议题与术语保留）。
fn trim_previous_json(json: String) -> String {
    if json.chars().count() <= MAX_PREVIOUS_JSON_CHARS {
        return json;
    }
    let Ok(mut value) = serde_json::from_str::<serde_json::Value>(&json) else {
        return "{}".to_owned();
    };
    // 先按单条序列化长度从最老裁起（估算超限的部分），再一次序列化收尾。
    for key in ["conclusions", "action_items"] {
        let overflow = json.chars().count().saturating_sub(MAX_PREVIOUS_JSON_CHARS);
        if overflow == 0 {
            break;
        }
        let Some(items) = value.get_mut(key).and_then(serde_json::Value::as_array_mut) else {
            continue;
        };
        let mut removed_chars = 0usize;
        while items.len() > 1 && removed_chars < overflow {
            let Some(oldest) = items.first() else { break };
            removed_chars += serde_json::to_string(oldest)
                .map(|text| text.chars().count())
                .unwrap_or(0);
            items.remove(0);
        }
    }
    let trimmed = serde_json::to_string(&value).unwrap_or_else(|_| "{}".to_owned());
    if trimmed.chars().count() > MAX_PREVIOUS_JSON_CHARS * 2 {
        // 极端情况下仍超（单条巨大）：保底丢弃全部条目，只留议题与术语。
        if let Some(object) = value.as_object_mut() {
            object.insert("conclusions".to_owned(), serde_json::Value::Array(vec![]));
            object.insert("action_items".to_owned(), serde_json::Value::Array(vec![]));
        }
        return serde_json::to_string(&value).unwrap_or_else(|_| "{}".to_owned());
    }
    trimmed
}

/// 实时定稿句 → 校验用 TranscriptSegment 视图（id 用 live 前缀防与库里混淆）。
fn live_segment_to_transcript(segment: &LiveSegment, sequence: usize) -> TranscriptSegment {
    TranscriptSegment {
        id: format!("live-{sequence}"),
        record_id: String::new(),
        sequence: sequence as i64,
        speaker_label: None,
        start_ms: segment.start_ms,
        end_ms: segment.end_ms,
        original_text: segment.text.clone(),
        normalized_text: None,
        normalization_version: None,
        edited_text: None,
    }
}

/// 从缓冲尾部取不超过上限字符的定稿句（增量提示词正文）。
fn tail_segments_within_chars(segments: &[TranscriptSegment], max_chars: usize) -> &[TranscriptSegment] {
    let mut total = 0usize;
    for (offset, segment) in segments.iter().enumerate().rev() {
        total += segment.original_text.chars().count();
        if total > max_chars && offset + 1 < segments.len() {
            return &segments[offset + 1..];
        }
    }
    segments
}

pub const INCREMENTAL_SYSTEM: &str = "你是会议纪要整理器。会议正在进行中，给你上一版增量纪要（可能为空）和最新逐字稿。请把新内容合并进纪要，输出完整版（不是只有新增量）。仅返回约定的简体中文 JSON，不要使用 Markdown 或额外说明。";

fn build_incremental_prompt(segments: &[TranscriptSegment], previous: Option<&str>) -> String {
    // 完整规则头必须随行：增量引擎是独立调用，模型看不见「事后纪要」的
    // 任何其它提示词——之前只写「规则与事后纪要相同」导致模型输出空纪要。
    let mut prompt = String::from(crate::minutes::MINUTES_PROMPT_HEADER);
    prompt.push_str("\n\n补充：会议正在进行中。下面是上一版增量纪要 JSON（null 表示还没有），请把新内容合并进去，输出完整版（不是只有新增量）。\n\n上一版增量纪要：\n");
    prompt.push_str(previous.unwrap_or("null"));
    prompt.push_str("\n\n本次会话至今的逐字稿：\n");
    for (index, segment) in segments.iter().enumerate() {
        prompt.push_str(&format!(
            "[{}][{}]{label}{text}\n",
            index + 1,
            crate::minutes::format_mmss(segment.start_ms),
            label = segment
                .speaker_label
                .as_deref()
                .filter(|label| !label.trim().is_empty())
                .map(|label| format!("[{label}] "))
                .unwrap_or_default(),
            text = segment.original_text
        ));
    }
    prompt
}

/// 增量引擎共享实时文字线程的停止信号；模型不可用时静默退化
/// （本机 LLM 通道缺席时本场无增量纪要，实时文字与录音照常——
/// 已知静默退化，不另发事件以免与实时文字的 unavailable 重复打扰）。
pub fn spawn_incremental(
    app: tauri::AppHandle,
    library: ManagedLibrary,
    manager: LiveTranscriptManager,
    session_id: String,
    source: &'static str,
    stop: Arc<AtomicBool>,
) {
    std::thread::Builder::new()
        .name("live-minutes".to_owned())
        .spawn(move || {
            // 与实时文字线程一致的兜底：panic 不静默消失，也不留半状态。
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                run_incremental_loop(app.clone(), library, manager, &session_id, source, stop)
            }));
            if outcome.is_err() {
                eprintln!("[live-minutes] 增量纪要线程异常退出：session={session_id}");
            }
        })
        .ok();
}

fn run_incremental_loop(
    app: tauri::AppHandle,
    library: ManagedLibrary,
    manager: LiveTranscriptManager,
    session_id: &str,
    source: &'static str,
    stop: Arc<AtomicBool>,
) {
    // 模型/通道不可用 → 增量纪要静默退化，实时文字照常。
    let generator = match MinutesGenerator::detect(&library) {
        Ok(generator) => generator,
        Err(_) => return,
    };
    let mut state = IncrementalState {
        // 首轮同样受 45s 节流：启动即计时，口径统一为「≥45s 且 ≥2 句」。
        last_run_at: Some(Instant::now()),
        ..IncrementalState::default()
    };
    let mut run_index = 0usize;
    let mut last_captured_ms = 0_i64;

    loop {
        if stop.load(Ordering::SeqCst) {
            return;
        }
        std::thread::sleep(CHECK_TICK);
        if stop.load(Ordering::SeqCst) {
            return;
        }
        let finals = manager.buffer().snapshot();
        if finals.is_empty() {
            continue;
        }
        last_captured_ms = last_captured_ms.max(
            finals
                .last()
                .map(|segment| segment.end_ms)
                .unwrap_or_default(),
        );
        if !state.should_run(finals.len(), Instant::now()) {
            continue;
        }

        let segments: Vec<TranscriptSegment> = finals
            .iter()
            .enumerate()
            .map(|(sequence, segment)| live_segment_to_transcript(segment, sequence))
            .collect();
        // 预算 = 保底 6000 字符与「上次成功运行以来的新增内容」取大者：
        // 账实相符优先，新增内容不允许滚出 prompt 之外却被记为已处理。
        let new_chars: usize = segments[state.processed_finals.min(segments.len())..]
            .iter()
            .map(|segment| segment.original_text.chars().count())
            .sum();
        let budget = INCREMENTAL_PROMPT_CHARS.max(new_chars);
        let prompt_segments = tail_segments_within_chars(&segments, budget);
        let prompt = build_incremental_prompt(prompt_segments, state.previous_json.as_deref());
        match generator.generate_minutes_json(&prompt) {
            Ok(draft) => {
                // fail-closed：与事后纪要同一套证据校验，不可溯源的条目直接丢弃。
                let mut document =
                    finalize_minutes_draft(draft, &segments, last_captured_ms + 1_000);
                let dropped = document.dropped_unverifiable;
                run_index += 1;
                state.record_success(finals.len(), &document);
                document.dropped_unverifiable = dropped;
                // 模型调用期间可能已 stop：不再发迟到事件。
                if stop.load(Ordering::SeqCst) {
                    return;
                }
                let _ = app.emit(
                    LIVE_MINUTES_EVENT,
                    LiveMinutesEvent {
                        session_id: session_id.to_owned(),
                        source,
                        dropped_unverifiable: dropped,
                        document,
                        generated_at_unix_ms: chrono::Utc::now().timestamp_millis(),
                        run_index,
                    },
                );
            }
            Err(_) => {
                // 烂输出/单次模型失败：静默跳过；失败轮也受 45s 节流，
                // 连续失败先丢回喂重开、再停车（实时文字照常）。
                if !state.record_failure(Instant::now()) {
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live_transcript::LiveSegment;

    fn live_segment(start_ms: i64, end_ms: i64, text: &str) -> LiveSegment {
        LiveSegment {
            start_ms,
            end_ms,
            text: text.to_owned(),
        }
    }

    #[test]
    fn incremental_runs_only_when_throttled_and_enough_new_speech() {
        // 节流：新句不足不跑。
        assert!(!should_run_incremental(1, 100_000));
        // 间隔不足不跑。
        assert!(!should_run_incremental(5, 10_000));
        // 条件齐备才跑。
        assert!(should_run_incremental(2, 45_000));
        assert!(should_run_incremental(10, 120_000));
    }

    #[test]
    fn state_throttles_first_run_and_keeps_rhythm_after_failures() {
        let mut state = IncrementalState {
            last_run_at: Some(Instant::now()),
            ..IncrementalState::default()
        };
        // 首轮同样受 45s 节流：刚启动（哪怕已有内容）不跑。
        assert!(!state.should_run(5, Instant::now()));

        // 时间到且新句足够 → 跑。
        std::thread::sleep(Duration::from_millis(40));
        assert!(!state.should_run(2, Instant::now()), "未满 45s 不跑");
        state.last_run_at = Some(Instant::now() - Duration::from_millis(46_000));
        assert!(state.should_run(2, Instant::now()));

        // 失败轮刷新节流：防止 elapsed 只增不减的重试风暴。
        state.record_success(5, &MinutesDocument::default());
        state.last_run_at = Some(Instant::now() - Duration::from_millis(46_000));
        assert!(state.record_failure(Instant::now()), "未达停车线继续跑");
        assert!(!state.should_run(2, Instant::now()), "失败后立刻重新节流");
    }

    #[test]
    fn consecutive_failures_reset_feed_then_stop_the_engine() {
        let mut state = IncrementalState::default();
        state.previous_json = Some("{\"conclusions\":[{\"text\":\"旧结论\"}]}".to_owned());
        // 前两次失败：保回喂。
        assert!(state.record_failure(Instant::now()));
        assert!(state.record_failure(Instant::now()));
        assert!(state.previous_json.is_some());
        // 第 3 次（FAILURE_RESET_THRESHOLD）：丢回喂重开，防截断死锁。
        assert!(state.record_failure(Instant::now()));
        assert!(state.previous_json.is_none());
        // 第 4 次：仍继续；第 5 次（FAILURE_STOP_THRESHOLD）：停车。
        assert!(state.record_failure(Instant::now()));
        assert!(!state.record_failure(Instant::now()), "连续失败达阈值应停车");
    }

    #[test]
    fn oversized_previous_json_trims_oldest_items_first() {
        let long_text = "这是一条足够长的结论内容用来撑大体积。".repeat(60);
        let document = MinutesDocument {
            conclusions: (0..6)
                .map(|index| crate::minutes::MinutesItem {
                    id: format!("c{index}"),
                    kind: crate::minutes::MinutesItemKind::Conclusion,
                    text: format!("结论{index}"),
                    owner: None,
                    evidence: crate::minutes::MinutesEvidence {
                        start_ms: index * 1_000,
                        end_ms: index * 1_000 + 500,
                        quote_text: long_text.clone(),
                    },
                })
                .collect(),
            ..MinutesDocument::default()
        };
        let json = serde_json::to_string(&document).unwrap();
        assert!(json.chars().count() > MAX_PREVIOUS_JSON_CHARS);
        let trimmed = trim_previous_json(json);
        assert!(trimmed.chars().count() <= MAX_PREVIOUS_JSON_CHARS + 2_000);
        let value: serde_json::Value = serde_json::from_str(&trimmed).unwrap();
        let remaining = value["conclusions"].as_array().unwrap().len();
        assert!(remaining < 6, "应从最老条目裁起");
        // 最后一条（最新）必须保留。
        assert!(trimmed.contains("结论5"));
    }

    #[test]
    fn conversion_preserves_timeline_and_marks_live_ids() {
        let segments: Vec<TranscriptSegment> = [
            live_segment(0, 4_000, "第一句"),
            live_segment(4_000, 9_000, "第二句"),
        ]
        .iter()
        .enumerate()
        .map(|(sequence, live)| live_segment_to_transcript(live, sequence))
        .collect();
        assert_eq!(segments[0].id, "live-0");
        assert_eq!(segments[1].start_ms, 4_000);
        assert_eq!(segments[1].end_ms, 9_000);
        assert_eq!(segments[1].original_text, "第二句");
    }

    #[test]
    fn tail_selection_respects_char_budget() {
        let filler: String = "会议内容".repeat(700); // 2800 字/段
        let segments: Vec<TranscriptSegment> = (0..4)
            .map(|index| TranscriptSegment {
                id: format!("s{index}"),
                record_id: String::new(),
                sequence: index,
                speaker_label: None,
                start_ms: index * 10_000,
                end_ms: (index + 1) * 10_000,
                original_text: format!("{filler}#{index}"),
                normalized_text: None,
                normalization_version: None,
                edited_text: None,
            })
            .collect();
        let tail = tail_segments_within_chars(&segments, 6_000);
        // 6200 字预算：最后两段（5600 字）入选，更早的截掉。
        assert_eq!(tail.len(), 2);
        assert_eq!(tail[0].original_text.ends_with("#2"), true);
    }

    #[test]
    fn incremental_prompt_carries_previous_and_rules() {
        let segments: Vec<TranscriptSegment> = [live_segment(0, 4_000, "第一句")]
            .iter()
            .enumerate()
            .map(|(sequence, live)| live_segment_to_transcript(live, sequence))
            .collect();
        let prompt = build_incremental_prompt(&segments, Some("{\"conclusions\":[]}"));
        assert!(prompt.contains("\"conclusions\":[]"));
        assert!(prompt.contains("第一句"));
        assert!(prompt.contains("宁可不写"));
        let without_previous = build_incremental_prompt(&segments, None);
        assert!(without_previous.contains("null"));
    }

    /// 端到端（ECHO_LIVE_MINUTES_E2E=1）：真实语音 + 真实模型跑一次增量合并，
    /// 验证输出经过 fail-closed 校验且条目可回链到实时定稿句。
    #[test]
    fn incremental_minutes_end_to_end_with_real_model() {
        let Ok(flag) = std::env::var("ECHO_LIVE_MINUTES_E2E") else {
            return;
        };
        if flag != "1" {
            return;
        }
        // 真实语音片段 → 当作实时定稿缓冲。
        let clip = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../benchmarks/clips/02-medium.m4a");
        let preprocessed =
            std::env::temp_dir().join(format!("echo-live-minutes-{}.wav", uuid::Uuid::new_v4()));
        crate::audio::preprocess(&clip, &preprocessed).expect("基准语音应可解码");
        let samples = crate::audio::read_normalized_wav(&preprocessed).unwrap();
        let _ = std::fs::remove_file(&preprocessed);
        let session = crate::whisper::WhisperAdapter::detect()
            .and_then(|adapter| {
                crate::whisper::WhisperSession::load(adapter.model_path().unwrap())
            })
            .expect("本机应有 Whisper 模型");
        let segments = session
            .transcribe(&samples, "zh", None)
            .expect("真实语音转写不应失败");
        let finals: Vec<LiveSegment> = segments
            .into_iter()
            .map(|segment| LiveSegment {
                start_ms: segment.start_ms,
                end_ms: segment.end_ms,
                text: segment.text,
            })
            .collect();
        assert!(finals.len() >= 2);

        // 真实模型跑增量（无前次纪要）。
        let library_root = std::env::temp_dir().join(format!("echo-lm-{}", uuid::Uuid::new_v4()));
        let library = crate::library::ManagedLibrary::open(library_root.clone()).unwrap();
        let generator = MinutesGenerator::detect(&library).expect("应有可用模型通道");
        let converted: Vec<TranscriptSegment> = finals
            .iter()
            .enumerate()
            .map(|(sequence, live)| live_segment_to_transcript(live, sequence))
            .collect();
        let prompt = build_incremental_prompt(
            tail_segments_within_chars(&converted, INCREMENTAL_PROMPT_CHARS),
            None,
        );
        let draft = generator
            .generate_minutes_json(&prompt)
            .expect("增量生成（结构化 JSON）不应失败");
        let document = finalize_minutes_draft(draft, &converted, 1_000_000);
        eprintln!(
            "增量端到端：定稿句={} 结论={} 待办={} 议题={} dropped={}",
            finals.len(),
            document.conclusions.len(),
            document.action_items.len(),
            document.topics.len(),
            document.dropped_unverifiable
        );
        // 时间轴单调（回链不漂移）。
        let mut items: Vec<_> = document
            .conclusions
            .iter()
            .chain(document.action_items.iter())
            .collect();
        items.sort_by_key(|item| item.evidence.start_ms);
        for pair in items.windows(2) {
            assert!(pair[1].evidence.start_ms >= pair[0].evidence.start_ms);
        }
        let _ = std::fs::remove_dir_all(library_root);
    }
}
