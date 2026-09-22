use crate::error::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;

const SAMPLE_RATE: usize = 16_000;
const ENHANCED_FILTERS: &str = "highpass=f=80,lowpass=f=7600,loudnorm=I=-16:LRA=7:TP=-1.5";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioPreprocessorStatus {
    pub enhanced_available: bool,
    pub engine: String,
    pub executable_path: Option<String>,
    pub version: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreprocessingMetadata {
    pub engine: String,
    pub enhanced: bool,
    pub filters: Vec<String>,
    pub sample_rate: u32,
    pub channels: u16,
}

#[derive(Debug, Clone)]
pub struct AudioChunk {
    pub sample_start: usize,
    pub sample_end: usize,
    pub accept_start_ms: i64,
    pub accept_end_ms: i64,
}

pub fn preprocessor_status() -> AudioPreprocessorStatus {
    let executable = find_ffmpeg();
    let version = executable.as_ref().and_then(|path| {
        Command::new(path)
            .arg("-version")
            .output()
            .ok()
            .filter(|output| output.status.success())
            .and_then(|output| String::from_utf8(output.stdout).ok())
            .and_then(|output| output.lines().next().map(str::to_owned))
    });
    AudioPreprocessorStatus {
        enhanced_available: executable.is_some() && version.is_some(),
        engine: if executable.is_some() && version.is_some() {
            "ffmpeg-enhanced".to_owned()
        } else {
            "macos-compatible".to_owned()
        },
        executable_path: executable.map(|path| path.to_string_lossy().to_string()),
        version,
    }
}

pub fn preprocess(input: &Path, output: &Path) -> AppResult<PreprocessingMetadata> {
    if let Some(ffmpeg) = find_ffmpeg() {
        let result = Command::new(ffmpeg)
            .args(["-hide_banner", "-nostdin", "-y", "-i"])
            .arg(input)
            .args([
                "-vn",
                "-map_metadata",
                "-1",
                "-ac",
                "1",
                "-ar",
                "16000",
                "-c:a",
                "pcm_s16le",
                "-af",
                ENHANCED_FILTERS,
            ])
            .arg(output)
            .output()
            .map_err(|error| AppError::Import(format!("无法启动增强音频预处理：{error}")))?;
        if result.status.success() {
            return Ok(PreprocessingMetadata {
                engine: "ffmpeg-enhanced".to_owned(),
                enhanced: true,
                filters: vec![
                    "highpass-80hz".to_owned(),
                    "lowpass-7600hz".to_owned(),
                    "loudnorm--16lufs".to_owned(),
                ],
                sample_rate: 16_000,
                channels: 1,
            });
        }
        let _ = std::fs::remove_file(output);
    }
    let result = Command::new("/usr/bin/afconvert")
        .args(["-f", "WAVE", "-d", "LEI16@16000", "-c", "1"])
        .arg(input)
        .arg(output)
        .output()
        .map_err(|error| AppError::Import(format!("无法启动系统音频预处理：{error}")))?;
    if !result.status.success() {
        let _ = std::fs::remove_file(output);
        return Err(AppError::Import(
            "增强与系统兼容预处理均失败，原始音频仍已保留".to_owned(),
        ));
    }
    Ok(PreprocessingMetadata {
        engine: "macos-compatible".to_owned(),
        enhanced: false,
        filters: Vec::new(),
        sample_rate: 16_000,
        channels: 1,
    })
}

pub fn read_normalized_wav(path: &Path) -> AppResult<Vec<f32>> {
    let reader = hound::WavReader::open(path)
        .map_err(|error| AppError::Import(format!("无法读取预处理音频：{error}")))?;
    let spec = reader.spec();
    if spec.channels != 1 || spec.sample_rate != 16_000 {
        return Err(AppError::Import("预处理音频不是 16kHz 单声道".to_owned()));
    }
    reader
        .into_samples::<i16>()
        .map(|sample| {
            sample
                .map(|value| value as f32 / i16::MAX as f32)
                .map_err(|error| AppError::Import(format!("音频样本无效：{error}")))
        })
        .collect()
}

pub fn plan_chunks(samples: &[f32]) -> Vec<AudioChunk> {
    let duration_ms = samples.len() as i64 * 1_000 / SAMPLE_RATE as i64;
    if duration_ms <= 90_000 {
        return vec![AudioChunk {
            sample_start: 0,
            sample_end: samples.len(),
            accept_start_ms: 0,
            accept_end_ms: duration_ms,
        }];
    }
    let boundaries = core_boundaries(samples);
    boundaries
        .windows(2)
        .map(|window| {
            let core_start = window[0];
            let core_end = window[1];
            let padding = 2 * SAMPLE_RATE;
            AudioChunk {
                sample_start: core_start.saturating_sub(padding),
                sample_end: (core_end + padding).min(samples.len()),
                accept_start_ms: core_start as i64 * 1_000 / SAMPLE_RATE as i64,
                accept_end_ms: core_end as i64 * 1_000 / SAMPLE_RATE as i64,
            }
        })
        .collect()
}

fn core_boundaries(samples: &[f32]) -> Vec<usize> {
    let mut boundaries = vec![0];
    let mut start = 0usize;
    while samples.len().saturating_sub(start) > 60 * SAMPLE_RATE {
        let minimum = start + 35 * SAMPLE_RATE;
        let target = start + 45 * SAMPLE_RATE;
        let maximum = (start + 55 * SAMPLE_RATE).min(samples.len());
        let boundary = quiet_boundaries(samples, minimum, maximum)
            .into_iter()
            .min_by_key(|candidate| candidate.abs_diff(target))
            .unwrap_or((start + 60 * SAMPLE_RATE).min(samples.len()));
        boundaries.push(boundary);
        start = boundary;
    }
    boundaries.push(samples.len());
    boundaries
}

fn quiet_boundaries(samples: &[f32], start: usize, end: usize) -> Vec<usize> {
    let frame = SAMPLE_RATE / 50;
    let minimum_frames = 23;
    let mut run_start = None;
    let mut candidates = Vec::new();
    for position in (start..end).step_by(frame) {
        let slice_end = (position + frame).min(samples.len());
        let rms = (samples[position..slice_end]
            .iter()
            .map(|sample| sample * sample)
            .sum::<f32>()
            / (slice_end - position).max(1) as f32)
            .sqrt();
        if rms <= 0.008 {
            run_start.get_or_insert(position);
        } else if let Some(begin) = run_start.take() {
            if position.saturating_sub(begin) >= minimum_frames * frame {
                candidates.push(begin + (position - begin) / 2);
            }
        }
    }
    if let Some(begin) = run_start {
        if end.saturating_sub(begin) >= minimum_frames * frame {
            candidates.push(begin + (end - begin) / 2);
        }
    }
    candidates
}

pub fn is_overlap_duplicate(
    previous: &crate::whisper::WhisperSegment,
    candidate: &crate::whisper::WhisperSegment,
) -> bool {
    if previous.end_ms < candidate.start_ms || candidate.end_ms < previous.start_ms {
        return false;
    }
    let left = compact_text(&previous.text);
    let right = compact_text(&candidate.text);
    if left.is_empty() || right.is_empty() {
        return false;
    }
    left == right
        || left.contains(&right)
        || right.contains(&left)
        || bigram_similarity(&left, &right) >= 0.72
}

/// 跨块重叠区的「接续」合并。同一段音频被相邻两块各自解码了一次，两边切出的片段
/// 边界不同：上一条的结尾和候选的开头说的是同一句话。此时候选不是重复段，不能丢
/// （丢了她就丢了边界后面的新内容），直接留下又会在成稿里出现
/// 「……推荐模型的推荐模型的大小是574兆」这种接缝。
///
/// 做法：在上一条结尾附近找候选开头的最长重合（≥4 个有效字符，允许上一条在重合
/// 之后还剩几个字——那是上一块对重叠区的另一次解码）。把重合部分从候选里去掉，
/// 剩余文本并回上一条；候选覆盖得上一条的尾部时，上一条那段尾巴也一并对成候选的
/// 版本。上一条的结束时间取两者较晚者。
/// 返回 false 表示不构成接续，调用方按原有判重逻辑处理。
pub fn merge_overlap_continuation(
    previous: &mut crate::whisper::WhisperSegment,
    candidate: &crate::whisper::WhisperSegment,
) -> bool {
    if previous.end_ms < candidate.start_ms || candidate.end_ms < previous.start_ms {
        return false;
    }
    let (left_text, left_origin) = compact_with_origin(&previous.text);
    let (right_text, right_origin) = compact_with_origin(&candidate.text);
    let left: Vec<char> = left_text.chars().collect();
    let right: Vec<char> = right_text.chars().collect();
    if left.is_empty() || right.is_empty() {
        return false;
    }
    // 从最长可能的重合往下找，命中即止：重合越长越可信。
    let limit = right.len().min(left.len());
    let mut hit = None;
    for overlap in (MIN_CONTINUATION_CHARS..=limit).rev() {
        let head = &right[..overlap];
        let earliest = left.len().saturating_sub(overlap + MAX_TAIL_SLACK);
        if let Some(offset) = left[earliest..]
            .windows(overlap)
            .position(|window| window == head)
        {
            hit = Some((overlap, earliest + offset + overlap));
            break;
        }
    }
    let Some((overlap, match_end)) = hit else {
        return false;
    };
    // 重合的字符数映射回候选原文的字节位置，从那里开始接；
    // 重合数等于候选全长时没有可接内容，交给调用方按重复段处理。
    let cut = right_origin
        .get(overlap)
        .copied()
        .unwrap_or(candidate.text.len());
    let rest = candidate.text[cut..].trim();
    if rest.is_empty() {
        return false;
    }
    // 上一条在重合之后还剩的内容，是同一段重叠音频的另一次解码。候选覆盖得到达段
    // 音频时，用候选的版本替换掉，否则会留下「从音频和音频文本都不上传」这种残尾。
    if match_end < left.len() && candidate.end_ms >= previous.end_ms {
        let keep = left_origin
            .get(match_end)
            .copied()
            .unwrap_or(previous.text.len());
        previous.text.truncate(keep);
    }
    previous.text.push_str(rest);
    previous.end_ms = previous.end_ms.max(candidate.end_ms);
    true
}

/// 接续判定的最小重合字符数。低于这个长度，「然后」「就是」这类常用连接词
/// 也会被误判成同一段音频，宁可留下接缝也不能吃掉正常内容。
const MIN_CONTINUATION_CHARS: usize = 4;

/// 重合点允许落在上一条结尾之后多少个字以内。上一块对重叠区末尾的识别可能多出
/// 几个字，不给这点余量就会漏掉真实接缝。
const MAX_TAIL_SLACK: usize = 4;

/// 压缩文本（去标点、转小写）同时记录每个压缩字符在原文中的字符位置，
/// 这样按重合字数裁剪原文时不必重新数字符。
fn compact_with_origin(text: &str) -> (String, Vec<usize>) {
    let normalized = crate::transcript::normalize_chinese(text);
    let mut compacted = String::new();
    let mut origin = Vec::new();
    for (index, character) in normalized.char_indices() {
        if !character.is_alphanumeric() {
            continue;
        }
        for lowered in character.to_lowercase() {
            compacted.push(lowered);
            origin.push(index);
        }
    }
    (compacted, origin)
}

fn compact_text(text: &str) -> String {
    crate::transcript::normalize_chinese(text)
        .chars()
        .filter(|character| character.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn bigram_similarity(left: &str, right: &str) -> f32 {
    use std::collections::HashSet;
    let grams = |text: &str| {
        let chars = text.chars().collect::<Vec<_>>();
        chars
            .windows(2)
            .map(|pair| pair.iter().collect::<String>())
            .collect::<HashSet<_>>()
    };
    let left = grams(left);
    let right = grams(right);
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    let intersection = left.intersection(&right).count() as f32;
    2.0 * intersection / (left.len() + right.len()) as f32
}

fn find_ffmpeg() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(path) = std::env::var_os("ECHO_FFMPEG_BIN") {
        candidates.push(PathBuf::from(path));
    }
    if let Ok(executable) = std::env::current_exe() {
        if let Some(macos) = executable.parent() {
            candidates.push(macos.join("../Resources/ffmpeg"));
        }
    }
    candidates.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/ffmpeg"));
    candidates.into_iter().find(|path| path.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_audio_uses_one_chunk() {
        let chunks = plan_chunks(&vec![0.0; 80 * SAMPLE_RATE]);
        assert_eq!(chunks.len(), 1);
    }

    #[test]
    fn long_audio_chunks_have_bounded_core_duration() {
        let chunks = plan_chunks(&vec![0.02; 130 * SAMPLE_RATE]);
        assert!(chunks.len() >= 3);
        assert!(chunks
            .iter()
            .all(|chunk| chunk.accept_end_ms - chunk.accept_start_ms <= 60_000));
    }

    #[test]
    fn enhanced_filter_graph_contains_required_processing() {
        assert!(ENHANCED_FILTERS.contains("highpass=f=80"));
        assert!(ENHANCED_FILTERS.contains("lowpass=f=7600"));
        assert!(ENHANCED_FILTERS.contains("loudnorm=I=-16"));
    }

    #[test]
    fn bundled_arm64_preprocessor_is_discoverable() {
        let status = preprocessor_status();
        assert!(status.enhanced_available, "status: {status:?}");
        assert_eq!(status.engine, "ffmpeg-enhanced");
        assert!(status
            .version
            .is_some_and(|version| version.contains("7.1.1")));
    }

    #[test]
    fn quiet_region_near_target_is_used_as_chunk_boundary() {
        let mut samples = vec![0.02; 110 * SAMPLE_RATE];
        samples[44 * SAMPLE_RATE..46 * SAMPLE_RATE].fill(0.0);
        let chunks = plan_chunks(&samples);
        assert!((44_000..=46_000).contains(&chunks[0].accept_end_ms));
    }

    #[test]
    fn overlapping_near_identical_whisper_text_is_deduplicated() {
        let previous = crate::whisper::WhisperSegment {
            start_ms: 40_000,
            end_ms: 46_000,
            text: "我们下一步讨论产品发布计划".to_owned(),
        };
        let candidate = crate::whisper::WhisperSegment {
            start_ms: 44_000,
            end_ms: 49_000,
            text: "我们下一步讨论产品发布的计划".to_owned(),
        };
        assert!(is_overlap_duplicate(&previous, &candidate));
    }

    fn segment(start_ms: i64, end_ms: i64, text: &str) -> crate::whisper::WhisperSegment {
        crate::whisper::WhisperSegment {
            start_ms,
            end_ms,
            text: text.to_owned(),
        }
    }

    #[test]
    fn boundary_continuation_merges_repeated_tail_into_previous() {
        // 06-long 第 1 个边界：上块结尾「…推荐模型的」与本块开头「推荐模型的大小是574兆」
        // 说的是同一段音频，重合 5 个字，合并后应接成完整句子。
        let mut previous = segment(
            53_000,
            62_000,
            "下一步要做的是自动推荐热词,第二个议题是模型下载链路,推荐模型的",
        );
        let candidate = segment(60_800, 64_100, "推荐模型的大小是574兆");
        assert!(merge_overlap_continuation(&mut previous, &candidate));
        assert_eq!(
            previous.text,
            "下一步要做的是自动推荐热词,第二个议题是模型下载链路,推荐模型的大小是574兆"
        );
        assert_eq!(previous.end_ms, 64_100);
    }

    #[test]
    fn boundary_continuation_handles_short_overlap_and_misheard_tail() {
        // 06-long 第 3 个边界：重合只有 4 个字，且上一块把重叠区末尾多听成了「音频」。
        // 候选覆盖得上一条的尾部，那段残尾应被候选的版本替换掉。
        let mut previous = segment(170_000, 182_000, "隐私口径也要相应调整,从音频和音频。");
        let candidate = segment(180_800, 183_600, "从音频和文本都不上传");
        assert!(merge_overlap_continuation(&mut previous, &candidate));
        assert_eq!(previous.text, "隐私口径也要相应调整,从音频和文本都不上传");
        assert_eq!(previous.end_ms, 183_600);
    }

    #[test]
    fn boundary_continuation_keeps_tail_when_candidate_does_not_cover_it() {
        // 候选比上一条短：重合点之后上一条还有候选没覆盖到的内容，不能截，
        // 否则会平白丢掉文字。
        let mut previous = segment(0, 10_000, "我们先对齐一下排期,然后确认负责人");
        let candidate = segment(9_000, 9_500, "然后确认负责人和时间");
        assert!(merge_overlap_continuation(&mut previous, &candidate));
        assert_eq!(previous.text, "我们先对齐一下排期,然后确认负责人和时间");
    }

    #[test]
    fn boundary_continuation_ignores_unrelated_or_disjoint_text() {
        // 时间不重叠：不是同一段音频，不能合。
        let mut previous = segment(0, 1_000, "我们下午三点开会");
        let candidate = segment(5_000, 6_000, "我们下午三点开会");
        assert!(!merge_overlap_continuation(&mut previous, &candidate));
        assert_eq!(previous.text, "我们下午三点开会");

        // 时间重叠但内容无关：合了就会吃掉正常内容。
        let mut previous = segment(0, 5_000, "今天先讲第一件事");
        let candidate = segment(4_000, 8_000, "然后再讲第二件事");
        assert!(!merge_overlap_continuation(&mut previous, &candidate));
        assert_eq!(previous.text, "今天先讲第一件事");

        // 重合太短（常用连接词级别）：留给原有判重逻辑，不合并。
        let mut previous = segment(0, 5_000, "这个事情然后");
        let candidate = segment(4_000, 8_000, "然后我们就定了");
        assert!(!merge_overlap_continuation(&mut previous, &candidate));
    }

    #[test]
    fn boundary_continuation_with_nothing_new_left_is_not_a_merge() {
        // 候选整条都是上一条的结尾：这是纯重复，交给 is_overlap_duplicate。
        let mut previous = segment(0, 5_000, "大家下午好");
        let candidate = segment(4_000, 6_000, "大家下午好");
        assert!(!merge_overlap_continuation(&mut previous, &candidate));
        assert!(is_overlap_duplicate(&previous, &candidate));
    }
}
