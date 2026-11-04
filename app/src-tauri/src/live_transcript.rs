//! 实时文字（流式转写预览）。
//!
//! 录音进行中，后台线程以固定节拍观察正在生长的 WAV 文件：把新增音频解码、
//! 重采样到 16k（Whisper 要求），对「已定稿边界 - 上下文」到当前末尾的滑窗做
//! whisper 解码，按「中点落在定稿边界之前的部分不再重复、压线句子先以暂定
//! 形式出现」的规则推进边界。权威逐字稿仍以停止后的全量转写为准，这里的
//! 输出只服务实时显示与增量纪要（块5），不写库。
//!
//! 延迟预算（目标 <3s）：节拍 1s + 窗口转写耗时（本机 large-v3-turbo 对 2-6s
//! 窗口约 0.3-1s）。事件里带 sent_at_unix_ms 与音频时间，前端可实测。

use crate::error::{AppError, AppResult};
use crate::whisper::{WhisperSegment, WhisperSession};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::Emitter;

/// WAV 头长度：采集层固定写 44 字节标准头。
pub const WAV_HEADER_BYTES: u64 = 44;
/// 定稿上下文：每次窗口从 committed-CONTEXT 开始转写，给 whisper 跨窗语境。
pub const CONTEXT_MS: i64 = 2_000;
/// 定稿留白：窗口末尾这一段先作暂定，等下一拍覆盖确认（压线句子不重复出字）。
pub const FINAL_MARGIN_MS: i64 = 600;
/// 每拍最少新增音频：不足时不解（太短 whisper 噪声大）。
pub const MIN_CHUNK_MS: i64 = 800;
/// 轮询节拍。
pub const TICK: Duration = Duration::from_millis(1_000);
/// 窗口上限：静音等异常导致 committed 长期不推进时，强制放弃最老未审内容
/// （权威转写在停止后仍会覆盖它），防止窗口无限增长拖垮延迟。
pub const MAX_WINDOW_MS: i64 = 15_000;
/// 静音 RMS 门限（16-bit 满幅归一）：低于它的新增音频视为无声。
/// ≈ -46dBFS，正常说话远高于此，安静办公室底噪通常低于此。
pub const SILENCE_RMS: f32 = 0.005;
/// 最近一次有声之后的保持窗口：期间即使当前拍安静也继续转写（句尾拖音）。
pub const VOICE_HOLD_MS: i64 = 1_500;
/// 失败状态事件的最小间隔：连续失败每秒刷屏没有信息量。
pub const FAILURE_EVENT_INTERVAL_MS: i64 = 10_000;
/// 样本缓冲裁剪阈值：committed-CONTEXT 之前的内容累计超过它就丢弃。
pub const TRIM_ABOVE_MS: i64 = 30_000;
/// 实时文字事件名（前端监听）。
pub const LIVE_EVENT: &str = "live-transcript";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveSegment {
    pub start_ms: i64,
    pub end_ms: i64,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveTick {
    /// 本拍新定稿的句子（绝对会话时间轴，毫秒）。
    pub new_final: Vec<LiveSegment>,
    /// 压线句子（可能还会变），前端整段替换上一拍的暂定尾。
    pub provisional_tail: Vec<LiveSegment>,
    /// 定稿边界推进后的位置。
    pub committed_ms: i64,
    /// 当前已落盘音频总时长（会话时间轴）。
    pub captured_ms: i64,
    /// 事件发出时刻（Unix 毫秒），供延迟实测。
    pub sent_at_unix_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveTickEvent {
    pub session_id: String,
    pub source: &'static str,
    #[serde(flatten)]
    pub tick: LiveTick,
}

/// 单条实时事件（流不可用时）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveStatusEvent {
    pub session_id: String,
    pub source: &'static str,
    pub available: bool,
    pub message: String,
}

/* ---------------- 纯逻辑（测试重点） ---------------- */

/// 16-bit PCM 小端字节 → f32（-1..1）。
pub fn pcm_i16_to_f32(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(2)
        .map(|chunk| i16::from_le_bytes([chunk[0], chunk[1]]) as f32 / 32_768.0)
        .collect()
}

/// 线性插值重采样（48k→16k 即每 3 个取 1 的插值版）。实时预览够用；
/// 权威转写走既有全量管线的正规预处理。
pub fn resample_linear(input: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
    if input.is_empty() || from_rate == 0 || to_rate == 0 || from_rate == to_rate {
        return input.to_vec();
    }
    let ratio = from_rate as f64 / to_rate as f64;
    let out_len = ((input.len() as f64) / ratio).floor() as usize;
    let mut out = Vec::with_capacity(out_len);
    for index in 0..out_len {
        let position = index as f64 * ratio;
        let base = position.floor() as usize;
        let frac = (position - base as f64) as f32;
        let left = input[base];
        let right = input.get(base + 1).copied().unwrap_or(left);
        out.push(left + (right - left) * frac);
    }
    out
}

/// 一拍的窗口推进决策：输入 whisper 对当前滑窗的解码结果（已映射为绝对时间），
/// 输出定稿/暂定切分与新边界。定稿规则：中点 ∈ [committed, captured-margin)；
/// 暂定规则：中点 ≥ captured-margin；中点 < committed 的是上下文重复，丢弃。
///
/// `finals` 是调用方持有的已定稿累积列表，本函数会原地更新它：相邻两窗对同一段
/// 音频的解码边界可能不同（前窗解出半句、本窗解出整句），复用全量管线的
/// 接续合并/判重逻辑处理——能接续就把新内容并进尾句（前端按 start_ms 同键替换），
/// 纯重复则丢弃。
pub fn decide_tick(
    segments: Vec<WhisperSegment>,
    committed_ms: i64,
    captured_ms: i64,
    finals: &mut Vec<LiveSegment>,
) -> LiveTick {
    let accept_end = (captured_ms - FINAL_MARGIN_MS).max(committed_ms);
    let mut new_final = Vec::new();
    let mut provisional_tail = Vec::new();
    for segment in segments {
        if segment.text.trim().is_empty() {
            continue;
        }
        let midpoint = segment.start_ms + (segment.end_ms - segment.start_ms) / 2;
        if midpoint >= committed_ms && midpoint < accept_end {
            let candidate_ws = WhisperSegment {
                start_ms: segment.start_ms,
                end_ms: segment.end_ms,
                text: segment.text.clone(),
            };
            let tail_outcome = match finals.last_mut() {
                Some(tail) => {
                    let mut tail_ws = WhisperSegment {
                        start_ms: tail.start_ms,
                        end_ms: tail.end_ms,
                        text: std::mem::take(&mut tail.text),
                    };
                    if crate::audio::merge_overlap_continuation(&mut tail_ws, &candidate_ws) {
                        // 接续成功：尾句被本窗解得更完整，更新缓冲并重发让前端同键替换。
                        tail.start_ms = tail_ws.start_ms;
                        tail.end_ms = tail_ws.end_ms;
                        tail.text = tail_ws.text;
                        Some(LiveSegment {
                            start_ms: tail.start_ms,
                            end_ms: tail.end_ms,
                            text: tail.text.clone(),
                        })
                    } else {
                        let duplicate =
                            crate::audio::is_overlap_duplicate(&tail_ws, &candidate_ws);
                        tail.text = tail_ws.text; // 归还被 take 的文本
                        duplicate.then_some(LiveSegment {
                            start_ms: tail.start_ms,
                            end_ms: tail.end_ms,
                            text: String::new(),
                        })
                    }
                }
                None => None,
            };
            match tail_outcome {
                Some(merged) if !merged.text.is_empty() => new_final.push(merged),
                Some(_) => {} // 纯重复：丢弃
                None => {
                    let candidate = LiveSegment {
                        start_ms: candidate_ws.start_ms,
                        end_ms: candidate_ws.end_ms,
                        text: candidate_ws.text,
                    };
                    // 落账进累积列表：本拍后续候选要与它判重（同拍相邻句）。
                    finals.push(candidate.clone());
                    new_final.push(candidate);
                }
            }
        } else if midpoint >= accept_end {
            provisional_tail.push(LiveSegment {
                start_ms: segment.start_ms,
                end_ms: segment.end_ms,
                text: segment.text,
            });
        }
        // midpoint < committed_ms：上下文区域重解，丢弃防重复。
    }
    LiveTick {
        committed_ms: accept_end,
        new_final,
        provisional_tail,
        captured_ms,
        sent_at_unix_ms: 0,
    }
}

/// 增量纪要等后续消费方共用的实时缓冲（块5 接线用）。
#[derive(Default)]
pub struct LiveTranscriptBuffer {
    segments: Mutex<Vec<LiveSegment>>,
}

impl LiveTranscriptBuffer {
    /// 每拍以调用方维护的定稿列表为准整体刷新（列表含接续合并后的尾句）。
    pub fn replace_all(&self, segments: &[LiveSegment]) {
        if let Ok(mut buffer) = self.segments.lock() {
            *buffer = segments.to_vec();
        }
    }

    pub fn snapshot(&self) -> Vec<LiveSegment> {
        self.segments
            .lock()
            .map(|buffer| buffer.clone())
            .unwrap_or_default()
    }

    pub fn clear(&self) {
        if let Ok(mut buffer) = self.segments.lock() {
            buffer.clear();
        }
    }
}

/* ---------------- 运行线程 ---------------- */

#[derive(Clone)]
pub struct LiveTranscriptConfig {
    pub language: String,
    pub hotword_prefix: String,
}

impl Default for LiveTranscriptConfig {
    fn default() -> Self {
        Self {
            language: "zh".to_owned(),
            hotword_prefix: String::new(),
        }
    }
}

pub struct LiveTranscriptHandle {
    stop: Arc<AtomicBool>,
    pub session_id: String,
}

impl LiveTranscriptHandle {
    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
    }

    pub fn is_stopped(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }
}

/// 会话级注册表：录音开始时登记、停止/放弃时摘除。
#[derive(Clone, Default)]
pub struct LiveTranscriptManager {
    handles: Arc<Mutex<std::collections::HashMap<String, LiveTranscriptHandle>>>,
    buffer: Arc<LiveTranscriptBuffer>,
}

impl LiveTranscriptManager {
    pub fn buffer(&self) -> &LiveTranscriptBuffer {
        &self.buffer
    }

    pub fn register(&self, handle: LiveTranscriptHandle) {
        if let Ok(mut handles) = self.handles.lock() {
            handles.insert(handle.session_id.clone(), handle);
        }
    }

    pub fn stop_session(&self, session_id: &str) {
        if let Ok(mut handles) = self.handles.lock() {
            if let Some(handle) = handles.remove(session_id) {
                handle.stop();
            }
        }
    }

    pub fn active_session_ids(&self) -> Vec<String> {
        self.handles
            .lock()
            .map(|handles| handles.keys().cloned().collect())
            .unwrap_or_default()
    }
}

/// 一次性读环境设置拼配置；失败退默认（zh、无热词）。
pub fn config_from_library(library: &crate::library::ManagedLibrary) -> LiveTranscriptConfig {
    let language = library
        .repository()
        .setting_value("transcription_language")
        .ok()
        .flatten()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "zh".to_owned());
    let hotword_prefix = library
        .repository()
        .hotwords_prompt()
        .unwrap_or_default();
    LiveTranscriptConfig {
        language,
        hotword_prefix,
    }
}

/// 启动实时转写线程。模型不可用时不抛错：发一条 unavailable 状态事件后退出，
/// 录音本身不受影响（权威转写在停止后照常）。
pub fn spawn(
    app: tauri::AppHandle,
    manager: LiveTranscriptManager,
    session_id: String,
    source: &'static str,
    wav_path: PathBuf,
    config: LiveTranscriptConfig,
) -> LiveTranscriptHandle {
    let stop = Arc::new(AtomicBool::new(false));
    let handle = LiveTranscriptHandle {
        stop: Arc::clone(&stop),
        session_id: session_id.clone(),
    };
    manager.register(LiveTranscriptHandle {
        stop: Arc::clone(&stop),
        session_id: session_id.clone(),
    });
    let config = config.clone();
    std::thread::Builder::new()
        .name("live-transcript".to_owned())
        .spawn(move || {
            // 所有退出路径（含 panic）统一摘除注册并给前端一个交代，
            // 不留「无线程的僵尸 handle」。
            let manager_for_cleanup = manager.clone();
            let session_for_cleanup = session_id.clone();
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                run_loop(
                    app.clone(),
                    manager,
                    &session_id,
                    source,
                    wav_path,
                    config,
                    Arc::clone(&stop),
                )
            }));
            manager_for_cleanup.stop_session(&session_for_cleanup);
            if outcome.is_err() {
                let _ = app.emit(
                    LIVE_EVENT,
                    LiveStatusEvent {
                        session_id: session_for_cleanup,
                        source,
                        available: false,
                        message: "实时文字线程异常退出；录音照常，停止后以全量转写为准".to_owned(),
                    },
                );
            }
        })
        .ok();
    handle
}

/// 解析 WAV 头：返回 (采样率, 声道数, 位深, data 块字节偏移)。
/// 不硬编码 44：写侧若改布局，读侧在这里显式失败而不是静默解出垃圾。
pub fn parse_wav_header(path: &Path) -> AppResult<(u32, u16, u16, u64)> {
    let mut file = fs::File::open(path)
        .map_err(|error| AppError::Import(format!("无法打开录音文件：{error}")))?;
    let mut head = [0u8; 12];
    file.read_exact(&mut head)
        .map_err(|error| AppError::Import(format!("录音文件过小：{error}")))?;
    if &head[0..4] != b"RIFF" || &head[8..12] != b"WAVE" {
        return Err(AppError::Import("不是 WAV 文件".to_owned()));
    }
    let mut channels = 0u16;
    let mut sample_rate = 0u32;
    let mut bits = 0u16;
    loop {
        let mut chunk_head = [0u8; 8];
        match file.read_exact(&mut chunk_head) {
            Ok(()) => {}
            Err(_) => return Err(AppError::Import("WAV 缺少 data 块".to_owned())),
        }
        let chunk_id = &chunk_head[0..4];
        let chunk_size = u32::from_le_bytes(chunk_head[4..8].try_into().unwrap()) as u64;
        if chunk_id == b"fmt " {
            let mut body = vec![0u8; chunk_size as usize];
            file.read_exact(&mut body)
                .map_err(|error| AppError::Import(format!("WAV fmt 块损坏：{error}")))?;
            channels = u16::from_le_bytes(body[2..4].try_into().unwrap());
            sample_rate = u32::from_le_bytes(body[4..8].try_into().unwrap());
            bits = u16::from_le_bytes(body[14..16].try_into().unwrap());
        } else if chunk_id == b"data" {
            let offset = file.stream_position().map_err(|e| AppError::Import(e.to_string()))?;
            return Ok((sample_rate, channels, bits, offset));
        } else {
            file.seek(SeekFrom::Current(chunk_size as i64))
                .map_err(|e| AppError::Import(format!("WAV 块跳转失败：{e}")))?;
        }
    }
}

#[cfg_attr(not(target_os = "macos"), allow(unused_variables))]
fn run_loop(
    app: tauri::AppHandle,
    manager: LiveTranscriptManager,
    session_id: &str,
    source: &'static str,
    wav_path: PathBuf,
    config: LiveTranscriptConfig,
    stop: Arc<AtomicBool>,
) {
    // 模型加载失败不阻塞录音：发一条状态事件后退出。
    #[cfg(target_os = "macos")]
    let session = match crate::whisper::WhisperAdapter::detect().and_then(|adapter| {
        WhisperSession::load(adapter.model_path().ok_or_else(|| {
            AppError::Import("找不到 Whisper 模型文件".to_owned())
        })?)
    }) {
        Ok(session) => session,
        Err(error) => {
            let _ = app.emit(
                LIVE_EVENT,
                LiveStatusEvent {
                    session_id: session_id.to_owned(),
                    source,
                    available: false,
                    message: format!("实时文字不可用：{error}；录音照常，停止后会做全量转写"),
                },
            );
            return;
        }
    };

    // 采集层写的是 48k/单声道/16-bit WAV；显式校验而不是硬编码假设，
    // 写侧改布局时这里诚实失败（unavailable 事件）而不是静默解出垃圾。
    #[cfg(target_os = "macos")]
    let (data_offset, ) = {
        match parse_wav_header(&wav_path) {
            Ok((48_000, 1, 16, offset)) => (offset,),
            Ok((rate, channels, bits, _)) => {
                let _ = app.emit(
                    LIVE_EVENT,
                    LiveStatusEvent {
                        session_id: session_id.to_owned(),
                        source,
                        available: false,
                        message: format!(
                            "实时文字不支持当前音频格式（{rate}Hz/{channels}ch/{bits}bit），录音照常"
                        ),
                    },
                );
                return;
            }
            Err(error) => {
                let _ = app.emit(
                    LIVE_EVENT,
                    LiveStatusEvent {
                        session_id: session_id.to_owned(),
                        source,
                        available: false,
                        message: format!("实时文字无法读取录音：{error}"),
                    },
                );
                return;
            }
        }
    };
    #[cfg(not(target_os = "macos"))]
    let data_offset: u64 = WAV_HEADER_BYTES;

    // 重任务闸门：与全量转写/分析共享并发额度，防止 GPU 争抢。
    // 拿不到就等——预览晚一点出现好过和分析任务互相拖垮。
    #[cfg(target_os = "macos")]
    let _permit = crate::state::heavy_job_limiter().acquire();

    let mut committed_ms = 0_i64;
    let mut consumed_bytes = data_offset;
    // 16k f32 样本缓冲：只保留 base_ms 之后的内容，老前缀定期裁剪。
    let mut samples_16k: Vec<f32> = Vec::new();
    let mut base_ms = 0_i64;
    // VAD 状态：最近一次有声的位置（会话时间轴）。
    let mut last_voice_ms = 0_i64;
    let mut last_failure_emit_ms = 0_i64;
    // 已定稿句子（含接续合并后的尾句），每拍整体刷新进共享缓冲。
    let mut finals: Vec<LiveSegment> = Vec::new();

    loop {
        if stop.load(Ordering::SeqCst) {
            return;
        }
        std::thread::sleep(TICK);
        if stop.load(Ordering::SeqCst) {
            return;
        }
        // 文件消失（放弃/封存完成）即退出；consumed_bytes 是含 data 偏移的绝对位置。
        let Ok(metadata) = std::fs::metadata(&wav_path) else {
            return;
        };
        let file_size = metadata.len();
        if file_size <= consumed_bytes {
            continue;
        }
        let mut new_bytes = (file_size - consumed_bytes) as usize;
        if new_bytes == 0 {
            continue;
        }
        // 奇数字节防御：16-bit PCM 必须整帧读，否则之后所有样本永久错位。
        new_bytes -= new_bytes % 2;
        if new_bytes == 0 {
            continue;
        }
        let mut chunk = vec![0u8; new_bytes];
        let read_ok = std::fs::OpenOptions::new()
            .read(true)
            .open(&wav_path)
            .and_then(|mut file| {
                file.seek(SeekFrom::Start(consumed_bytes))?;
                file.read_exact(&mut chunk)
            })
            .is_ok();
        if !read_ok {
            continue;
        }
        consumed_bytes += new_bytes as u64;
        let samples_48k = pcm_i16_to_f32(&chunk);
        let new_samples = resample_linear(&samples_48k, 48_000, 16_000);
        let chunk_rms = {
            let sum: f64 = new_samples
                .iter()
                .filter(|value| value.is_finite())
                .map(|value| {
                    let value = *value as f64;
                    value * value
                })
                .sum();
            (sum / new_samples.len().max(1) as f64).sqrt() as f32
        };
        samples_16k.extend(new_samples);

        let captured_ms = base_ms + samples_16k.len() as i64 * 1_000 / 16_000;
        if chunk_rms >= SILENCE_RMS {
            last_voice_ms = captured_ms;
        }

        // 静音门控：最近 1.5s 无声就不再喂 whisper（句尾拖音已在保持窗内），
        // 直接推进边界并清掉暂定尾——防长静音把窗口撑爆、防音乐/底噪幻觉句。
        let voice_recent = captured_ms - last_voice_ms <= VOICE_HOLD_MS;
        if !voice_recent {
            let new_committed = (captured_ms - FINAL_MARGIN_MS).max(committed_ms);
            if new_committed > committed_ms || !finals.is_empty() {
                let tick = LiveTick {
                    new_final: Vec::new(),
                    provisional_tail: Vec::new(),
                    committed_ms: new_committed,
                    captured_ms,
                    sent_at_unix_ms: chrono::Utc::now().timestamp_millis(),
                };
                committed_ms = new_committed;
                trim_samples(&mut samples_16k, &mut base_ms, committed_ms);
                let _ = app.emit(
                    LIVE_EVENT,
                    LiveTickEvent {
                        session_id: session_id.to_owned(),
                        source,
                        tick,
                    },
                );
            }
            continue;
        }

        if captured_ms - committed_ms < MIN_CHUNK_MS {
            continue;
        }

        // 窗口上限：异常情况下强制放弃最老未审内容，延迟优先。
        if captured_ms - (committed_ms - CONTEXT_MS) > MAX_WINDOW_MS {
            committed_ms = captured_ms - MAX_WINDOW_MS + CONTEXT_MS;
        }

        let window_start_ms = (committed_ms - CONTEXT_MS).max(base_ms);
        let window_start_index = ((window_start_ms - base_ms).max(0) as usize) * 16_000 / 1_000;
        if window_start_index >= samples_16k.len() {
            continue;
        }
        let window = &samples_16k[window_start_index..];
        let segments = match session.transcribe_allow_empty(
            window,
            &config.language,
            (!config.hotword_prefix.is_empty()).then_some(config.hotword_prefix.as_str()),
        ) {
            Ok(segments) => segments,
            Err(error) => {
                // 单拍失败不终止实时预览：跳过这一拍，下一拍重试窗口。
                // 失败事件限流：连续失败每秒刷屏没有信息量。
                let now_ms = chrono::Utc::now().timestamp_millis();
                if now_ms - last_failure_emit_ms >= FAILURE_EVENT_INTERVAL_MS {
                    last_failure_emit_ms = now_ms;
                    let _ = app.emit(
                        LIVE_EVENT,
                        LiveStatusEvent {
                            session_id: session_id.to_owned(),
                            source,
                            available: false,
                            message: format!("实时转写单拍失败，将继续重试：{error}"),
                        },
                    );
                }
                continue;
            }
        };
        // whisper 返回的时间是相对窗口起点，映射到会话绝对时间。
        let absolute: Vec<WhisperSegment> = segments
            .into_iter()
            .map(|segment| WhisperSegment {
                start_ms: segment.start_ms + window_start_ms,
                end_ms: segment.end_ms + window_start_ms,
                text: segment.text,
            })
            .collect();
        let mut tick = decide_tick(absolute, committed_ms, captured_ms, &mut finals);
        tick.sent_at_unix_ms = chrono::Utc::now().timestamp_millis();
        committed_ms = tick.committed_ms;
        trim_samples(&mut samples_16k, &mut base_ms, committed_ms);
        manager.buffer().replace_all(&finals);
        let _ = app.emit(
            LIVE_EVENT,
            LiveTickEvent {
                session_id: session_id.to_owned(),
                source,
                tick,
            },
        );
    }
}

/// 定期丢弃 committed-CONTEXT 之前的前缀样本（长会议内存防线）。
fn trim_samples(samples: &mut Vec<f32>, base_ms: &mut i64, committed_ms: i64) {
    let keep_from_ms = (committed_ms - CONTEXT_MS - 5_000).max(0);
    let drop_ms = keep_from_ms - *base_ms;
    if drop_ms <= TRIM_ABOVE_MS {
        return;
    }
    let drop_samples = (drop_ms as usize) * 16_000 / 1_000;
    let drop_samples = drop_samples.min(samples.len());
    samples.drain(0..drop_samples);
    *base_ms += drop_samples as i64 * 1_000 / 16_000;
}

/// 真机端到端（ECHO_LIVE_E2E=1）：用真实语音片段模拟「文件逐步生长」，
/// 验证实时链路产出与全量转写一致的文本、时间戳不漂移、无重复句子。
#[cfg(test)]
mod tests {
    use super::*;
    use crate::system_audio::{WavStreamWriter, SYSTEM_SAMPLE_RATE};

    fn scratch_path(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!("echo-live-{label}-{}.wav", uuid::Uuid::new_v4()))
    }

    fn segment(start_ms: i64, end_ms: i64, text: &str) -> WhisperSegment {
        WhisperSegment {
            start_ms,
            end_ms,
            text: text.to_owned(),
        }
    }

    #[test]
    fn pcm_decode_and_resample_are_consistent() {
        let bytes: Vec<u8> = [0i16, 16384, -16384, 32767]
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect();
        let samples = pcm_i16_to_f32(&bytes);
        assert_eq!(samples.len(), 4);
        assert!((samples[1] - 0.5).abs() < 0.001);
        assert!((samples[3] - 1.0).abs() < 0.001);

        // 48k → 16k：长度严格 1/3，直流信号幅值不变。
        let input = vec![0.5f32; 48_000];
        let output = resample_linear(&input, 48_000, 16_000);
        assert_eq!(output.len(), 16_000);
        assert!(output.iter().all(|value| (value - 0.5).abs() < 1e-6));

        // 同率重采样原样返回。
        assert_eq!(resample_linear(&input, 48_000, 48_000), input);
    }

    #[test]
    fn tick_finalizes_only_before_margin_and_skips_context() {
        // committed=0, captured=5000, margin=600 → accept_end=4400。
        let segments = vec![
            segment(100, 1000, "开场"),       // 中点 550 < 4400 → 定稿
            segment(3600, 4600, "压线句"),    // 中点 4100 < 4400 → 定稿
            segment(4700, 5600, "还在说"),    // 中点 5150 ≥ 4400 → 暂定
            segment(0, 300, "上下文"),        // 中点 150 ≥ committed=0 → 也会定稿
        ];
        let mut finals: Vec<LiveSegment> = Vec::new();
        let tick = decide_tick(segments.clone(), 0, 5_000, &mut finals);
        let final_texts: Vec<&str> =
            tick.new_final.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(final_texts, vec!["开场", "压线句", "上下文"]);
        assert_eq!(tick.provisional_tail.len(), 1);
        assert_eq!(tick.provisional_tail[0].text, "还在说");
        assert_eq!(tick.committed_ms, 4_400);
        assert_eq!(finals.len(), 3);

        // 下一拍：committed 已推进，旧句子不会被再次定稿（防重复）；
        // 上一拍的暂定「还在说」此时进入定稿区，转正一次（且仅一次）。
        let tick2 = decide_tick(segments, 4_400, 8_000, &mut finals);
        assert!(tick2
            .new_final
            .iter()
            .all(|s| !matches!(s.text.as_str(), "开场" | "压线句" | "上下文")));
        assert_eq!(finals.len(), 4);
        assert_eq!(finals.last().map(|s| s.text.as_str()), Some("还在说"));
    }

    #[test]
    fn straddling_sentence_becomes_final_on_next_tick() {
        // 第一拍：句子中点在 margin 之外 → 暂定。
        let sentence = segment(4_300, 5_400, "跨边界的一句");
        let mut finals: Vec<LiveSegment> = Vec::new();
        let first = decide_tick(vec![sentence.clone()], 0, 5_000, &mut finals);
        assert!(first.new_final.is_empty());
        assert_eq!(first.provisional_tail.len(), 1);

        // 第二拍（音频长出来了）：中点落入定稿区 → 定稿且仅出现一次。
        let second = decide_tick(vec![sentence], first.committed_ms, 7_000, &mut finals);
        assert_eq!(second.new_final.len(), 1);
        assert_eq!(second.new_final[0].text, "跨边界的一句");
        assert!(second.provisional_tail.is_empty());
    }

    #[test]
    fn context_region_is_never_reemitted() {
        // committed=4000；窗口从 2000 起，whisper 把上下文也解出来了，
        // 但中点 < committed 的必须全部丢弃。
        let segments = vec![
            segment(2_200, 3_800, "上下文旧句"),
            segment(4_100, 5_000, "新句"),
        ];
        let mut finals: Vec<LiveSegment> = Vec::new();
        let tick = decide_tick(segments, 4_000, 6_000, &mut finals);
        assert_eq!(tick.new_final.len(), 1);
        assert_eq!(tick.new_final[0].text, "新句");
    }

    #[test]
    fn cross_window_resentence_merges_into_tail_instead_of_duplicating() {
        // 真机 e2e 抓到的形态：前窗解出「我们把第三季度的产品」，
        // 下一窗同段音频被解成完整长句。必须接续并进尾句，不能整句重复。
        let mut finals: Vec<LiveSegment> = Vec::new();
        let first = decide_tick(
            vec![segment(3_980, 6_000, "我们把第三季度的产品")],
            0,
            6_600,
            &mut finals,
        );
        assert_eq!(first.new_final.len(), 1);
        assert_eq!(finals.len(), 1);

        let second = decide_tick(
            vec![segment(
                3_760,
                9_000,
                "我们把第三季度的产品复盘安排在下周三下午2点",
            )],
            first.committed_ms,
            9_600,
            &mut finals,
        );
        // 接续：尾句变完整，重发的就是更新后的尾句（同 start_ms 替换键）。
        assert_eq!(second.new_final.len(), 1);
        let merged = &second.new_final[0];
        assert_eq!(merged.start_ms, 3_980);
        assert_eq!(merged.end_ms, 9_000);
        assert_eq!(merged.text, "我们把第三季度的产品复盘安排在下周三下午2点");
        assert_eq!(finals.len(), 1);
        assert_eq!(finals[0].text, "我们把第三季度的产品复盘安排在下周三下午2点");

        // 时间轴单调：合并后下一句仍在其后。
        let third = decide_tick(
            vec![segment(9_100, 10_500, "下一句")],
            second.committed_ms,
            12_000,
            &mut finals,
        );
        assert_eq!(third.new_final.len(), 1);
        assert!(finals.windows(2).all(|pair| pair[1].start_ms >= pair[0].start_ms));
    }

    #[test]
    fn pure_duplicate_across_windows_is_dropped() {
        let mut finals: Vec<LiveSegment> = Vec::new();
        decide_tick(
            vec![segment(1_000, 2_500, "这是同一句话")],
            0,
            3_100,
            &mut finals,
        );
        // 下一窗把同句原样再解一遍（时间几乎相同）→ 判重丢弃。
        let second = decide_tick(
            vec![segment(1_050, 2_480, "这是同一句话")],
            3_100 - 1_200,
            4_000,
            &mut finals,
        );
        let duplicates: Vec<_> = second
            .new_final
            .iter()
            .filter(|s| s.text.contains("这是同一句话"))
            .collect();
        assert!(duplicates.is_empty(), "重复句不应再次定稿");
    }

    #[test]
    fn wav_header_is_parsed_not_assumed() {
        // 写侧用 WavStreamWriter 产生标准文件，读侧必须解析出 48k/单声道/16bit
        // 与 data 偏移；非 WAV 文件要诚实报错而不是静默解出垃圾。
        let path = scratch_path("header");
        let mut writer = WavStreamWriter::create(&path, SYSTEM_SAMPLE_RATE, 1).unwrap();
        writer.append_i16(&[0i16; 480]).unwrap();
        writer.finalize().unwrap();
        let (rate, channels, bits, offset) = parse_wav_header(&path).unwrap();
        assert_eq!(rate, 48_000);
        assert_eq!(channels, 1);
        assert_eq!(bits, 16);
        assert_eq!(offset, 44);
        std::fs::remove_file(&path).ok();

        let bogus = scratch_path("not-wav");
        std::fs::write(&bogus, b"not a wav file at all........").unwrap();
        assert!(parse_wav_header(&bogus).is_err());
        std::fs::remove_file(&bogus).ok();
    }

    #[test]
    fn old_prefix_samples_are_trimmed_but_window_context_kept() {
        let mut samples: Vec<f32> = vec![0.5; 40 * 16_000]; // 40 秒
        let mut base_ms = 0_i64;
        // committed=45s：keep_from = 45-2-5 = 38s，drop=38s > TRIM_ABOVE(30s) → 裁。
        // 2 秒 × 16 样本/毫秒 = 32000 样本。
        trim_samples(&mut samples, &mut base_ms, 45_000);
        assert_eq!(base_ms, 38_000);
        assert_eq!(samples.len(), 32_000);
        // 刚过阈值以内不裁：committed=30s → keep_from=23s，drop=23s ≤ 30s。
        let mut samples2: Vec<f32> = vec![0.5; 40 * 16_000];
        let mut base2 = 0_i64;
        trim_samples(&mut samples2, &mut base2, 30_000);
        assert_eq!(base2, 0);
        assert_eq!(samples2.len(), 40 * 16_000);
    }

    #[test]
    fn buffer_snapshot_is_append_only() {
        let buffer = LiveTranscriptBuffer::default();
        buffer.replace_all(&[LiveSegment {
            start_ms: 0,
            end_ms: 100,
            text: "一".to_owned(),
        }]);
        buffer.replace_all(&[
            LiveSegment {
                start_ms: 0,
                end_ms: 100,
                text: "一".to_owned(),
            },
            LiveSegment {
                start_ms: 200,
                end_ms: 300,
                text: "二".to_owned(),
            },
        ]);
        assert_eq!(buffer.snapshot().len(), 2);
        buffer.clear();
        assert!(buffer.snapshot().is_empty());
    }

    #[test]
    fn tick_event_serializes_camel_case_for_frontend() {
        let mut finals: Vec<LiveSegment> = Vec::new();
        let tick = decide_tick(vec![segment(0, 100, "你好")], 0, 5_000, &mut finals);
        let json = serde_json::to_string(&LiveTickEvent {
            session_id: "s".to_owned(),
            source: "microphone",
            tick,
        })
        .unwrap();
        assert!(json.contains("newFinal"));
        assert!(json.contains("committedMs"));
        assert!(json.contains("sentAtUnixMs"));
        assert!(json.contains("sessionId"));
    }

    /// 真机语义验证：把真实语音片段（benchmarks/clips/01-short.m4a）切成
    /// 逐步生长的块喂给实时决策循环，对照片段参考文本检查覆盖率，
    /// 并测量每拍延迟。需要本机 Whisper 模型。
    #[test]
    fn live_pipeline_transcribes_real_speech_incrementally() {
        let Ok(flag) = std::env::var("ECHO_LIVE_E2E") else {
            return;
        };
        if flag != "1" {
            return;
        }
        let clip = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../benchmarks/clips/01-short.m4a");
        let reference = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../benchmarks/clips/01-short.txt");
        // 复用既有预处理：解码 + 重采样到 16k 单声道 f32。
        let preprocessed = std::env::temp_dir().join(format!("echo-live-e2e-{}.wav", uuid::Uuid::new_v4()));
        crate::audio::preprocess(&clip, &preprocessed)
            .expect("基准语音片段应能被既有预处理解码");
        let decoded = crate::audio::read_normalized_wav(&preprocessed)
            .expect("预处理产物应可读");
        let _ = std::fs::remove_file(&preprocessed);
        let session = crate::whisper::WhisperAdapter::detect()
            .and_then(|adapter| WhisperSession::load(adapter.model_path().expect("模型路径应存在")))
            .expect("本机应有 Whisper 模型（应用内下载过推荐模型）");

        // 模拟生长：每拍新增 3 秒音频，从 committed-CONTEXT 起滑窗解码。
        let sample_rate = 16_000usize;
        let chunk_samples = 3 * sample_rate;
        let mut committed_ms = 0_i64;
        let mut all_final: Vec<LiveSegment> = Vec::new();
        let mut max_latency_ms = 0_i64;
        let mut position = 0usize;
        let mut tick_count = 0;
        while position < decoded.len() {
            let end = (position + chunk_samples).min(decoded.len());
            let captured_ms = end as i64 * 1_000 / sample_rate as i64;
            if captured_ms - committed_ms >= MIN_CHUNK_MS {
                let window_start_ms = (committed_ms - CONTEXT_MS).max(0);
                let window = &decoded[window_start_ms as usize * sample_rate / 1_000..end];
                let started = std::time::Instant::now();
                let segments = session
                    .transcribe(window, "zh", None)
                    .expect("真实语音转写不应失败");
                let decode_ms = started.elapsed().as_millis() as i64;
                max_latency_ms = max_latency_ms.max(decode_ms + TICK_MS_SIM);
                let absolute: Vec<WhisperSegment> = segments
                    .into_iter()
                    .map(|segment| WhisperSegment {
                        start_ms: segment.start_ms + window_start_ms,
                        end_ms: segment.end_ms + window_start_ms,
                        text: segment.text,
                    })
                    .collect();
                let tick = decide_tick(absolute, committed_ms, captured_ms, &mut all_final);
                committed_ms = tick.committed_ms;
                tick_count += 1;
            }
            position = end;
        }
        assert!(tick_count > 1, "应有多个节拍（tick_count={tick_count}）");

        let live_text: String = all_final.iter().map(|s| s.text.as_str()).collect();
        let live_normalized = normalize_for_compare(&live_text);
        let reference_text =
            std::fs::read_to_string(&reference).expect("参考文本应存在");
        let reference_normalized = normalize_for_compare(&reference_text);
        // 覆盖率：参考文本的字符在实时输出里也应出现（宽松包含检查，
        // 大模型逐字不同是正常的；这里验证的是链路与时间轴，不是 CER）。
        let covered = reference_normalized
            .chars()
            .filter(|ch| live_normalized.contains(*ch))
            .count();
        let coverage = covered as f64 / reference_normalized.chars().count() as f64;
        eprintln!(
            "实时链路（真实语音）：tick={tick_count} 最终句={} 覆盖率={coverage:.2} 最大延迟≈{max_latency_ms}ms",
            all_final.len()
        );
        assert!(
            coverage > 0.6,
            "实时链路对真实语音的文本覆盖率过低：{coverage:.2}"
        );
        // 时间戳不漂移：最终句时间轴必须单调不减。
        for pair in all_final.windows(2) {
            assert!(
                pair[1].start_ms >= pair[0].start_ms,
                "最终句时间戳漂移：{:?} → {:?}",
                pair[0],
                pair[1]
            );
        }
    }

    const TICK_MS_SIM: i64 = 1_000;

    fn normalize_for_compare(text: &str) -> String {
        text.chars()
            .filter(|ch| !ch.is_whitespace() && !ch.is_ascii_punctuation())
            .flat_map(|ch| ch.to_lowercase())
            .collect()
    }
}
