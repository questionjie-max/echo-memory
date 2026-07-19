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
}
