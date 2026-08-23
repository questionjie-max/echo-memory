//! Local Whisper adapter: embedded macOS Metal or `whisper.cpp` CLI. It never sends audio to a network service.

use crate::error::{AppError, AppResult};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WhisperSegment {
    pub start_ms: i64,
    pub end_ms: i64,
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct WhisperAdapter {
    engine: Engine,
}

#[derive(Debug, Clone)]
enum Engine {
    Cli(PathBuf),
    #[cfg(target_os = "macos")]
    Embedded(PathBuf),
}

impl WhisperAdapter {
    pub fn detect() -> AppResult<Self> {
        Self::detect_with_model_path(None)
    }

    pub fn detect_with_model_path(model_path: Option<&str>) -> AppResult<Self> {
        #[cfg(target_os = "macos")]
        if let Some(model) = model_path
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
        {
            if !model.is_file() {
                return Err(AppError::Import("选择的 Whisper 模型文件不存在".to_owned()));
            }
            return Ok(Self {
                engine: Engine::Embedded(model),
            });
        }
        #[cfg(target_os = "macos")]
        if let Some(model) = embedded_model() {
            return Ok(Self {
                engine: Engine::Embedded(model),
            });
        }
        if let Some(executable) = std::env::var_os("WHISPER_CPP_BIN")
            .map(PathBuf::from)
            .or_else(find_on_path)
        {
            return Ok(Self {
                engine: Engine::Cli(executable),
            });
        }
        Err(AppError::Import(
            "未找到 whisper.cpp 或本机 Whisper 模型。设置 WHISPER_CPP_BIN 或 WHISPER_MODEL_PATH。"
                .into(),
        ))
    }

    pub fn model_path(&self) -> Option<&Path> {
        match &self.engine {
            Engine::Cli(_) => None,
            #[cfg(target_os = "macos")]
            Engine::Embedded(model) => Some(model),
        }
    }

    pub fn transcribe(
        &self,
        audio_path: &Path,
        language: &str,
        output_prefix: &Path,
    ) -> AppResult<Vec<WhisperSegment>> {
        match &self.engine {
            Engine::Cli(executable) => {
                self.transcribe_cli(executable, audio_path, language, output_prefix)
            }
            #[cfg(target_os = "macos")]
            Engine::Embedded(model) => embedded_transcribe(model, audio_path, language),
        }
    }

    pub fn model_name(&self) -> String {
        match &self.engine {
            Engine::Cli(_) => "whisper.cpp-cli".to_owned(),
            #[cfg(target_os = "macos")]
            Engine::Embedded(model) => model
                .file_stem()
                .and_then(|name| name.to_str())
                .unwrap_or("whisper-rs")
                .to_owned(),
        }
    }

    pub fn transcribe_samples(
        &self,
        samples: &[f32],
        language: &str,
        initial_prompt: Option<&str>,
    ) -> AppResult<Vec<WhisperSegment>> {
        match &self.engine {
            #[cfg(target_os = "macos")]
            Engine::Embedded(model) => {
                embedded_transcribe_samples(model, samples, language, initial_prompt)
            }
            Engine::Cli(_) => Err(AppError::Import(
                "增强分块转写需要本地 Whisper 模型文件".to_owned(),
            )),
        }
    }

    fn transcribe_cli(
        &self,
        executable: &Path,
        audio_path: &Path,
        language: &str,
        output_prefix: &Path,
    ) -> AppResult<Vec<WhisperSegment>> {
        let output = Command::new(executable)
            .args([
                "-f",
                &audio_path.to_string_lossy(),
                "-l",
                language,
                "-oj",
                "-of",
                &output_prefix.to_string_lossy(),
            ])
            .output()
            .map_err(|error| AppError::Import(format!("无法启动 whisper.cpp: {error}")))?;
        if !output.status.success() {
            return Err(AppError::Import(format!(
                "whisper.cpp 转写失败: {}",
                redact_message(&String::from_utf8_lossy(&output.stderr))
            )));
        }
        let json = std::fs::read_to_string(output_prefix.with_extension("json"))
            .map_err(|error| AppError::Import(format!("whisper.cpp 未生成 JSON 输出: {error}")))?;
        parse_whisper_json(&json)
    }
}

#[cfg(target_os = "macos")]
fn embedded_model() -> Option<PathBuf> {
    std::env::var_os("WHISPER_MODEL_PATH")
        .map(PathBuf::from)
        .or_else(|| {
            let home = std::env::var_os("HOME")?;
            let path = PathBuf::from(home)
                .join("Library/Application Support/com.meetily.ai/models/ggml-small.bin");
            path.is_file().then_some(path)
        })
}

#[cfg(target_os = "macos")]
fn embedded_transcribe(
    model: &Path,
    audio: &Path,
    language: &str,
) -> AppResult<Vec<WhisperSegment>> {
    let wav = std::env::temp_dir().join(format!("echo-memory-{}.wav", uuid::Uuid::new_v4()));
    let converted = Command::new("/usr/bin/afconvert")
        .args([
            "-f",
            "WAVE",
            "-d",
            "LEI16@16000",
            "-c",
            "1",
            &audio.to_string_lossy(),
            &wav.to_string_lossy(),
        ])
        .output()
        .map_err(|error| AppError::Import(format!("无法调用系统音频转换器: {error}")))?;
    if !converted.status.success() {
        return Err(AppError::Import(format!(
            "音频转换失败: {}",
            redact_message(&String::from_utf8_lossy(&converted.stderr))
        )));
    }
    let samples = hound::WavReader::open(&wav)
        .map_err(|error| AppError::Import(format!("无法读取转换后的音频: {error}")))?
        .into_samples::<i16>()
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| AppError::Import(format!("音频样本无效: {error}")))?;
    let _ = std::fs::remove_file(&wav);
    let audio: Vec<f32> = samples
        .into_iter()
        .map(|sample| sample as f32 / i16::MAX as f32)
        .collect();
    embedded_transcribe_samples(model, &audio, language, None)
}

#[cfg(target_os = "macos")]
fn embedded_transcribe_samples(
    model: &Path,
    audio: &[f32],
    language: &str,
    initial_prompt: Option<&str>,
) -> AppResult<Vec<WhisperSegment>> {
    use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};
    silence_whisper_logs();
    let context = WhisperContext::new_with_params(
        &model.to_string_lossy(),
        WhisperContextParameters::default(),
    )
    .map_err(|error| AppError::Import(format!("无法加载 Whisper 模型: {error}")))?;
    let mut state = context
        .create_state()
        .map_err(|error| AppError::Import(format!("无法创建 Whisper 状态: {error}")))?;
    let mut params = FullParams::new(SamplingStrategy::BeamSearch {
        beam_size: 5,
        patience: 1.0,
    });
    params.set_language((language != "auto").then_some(language));
    if let Some(prompt) = initial_prompt.filter(|prompt| !prompt.trim().is_empty()) {
        params.set_initial_prompt(prompt);
    }
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_special(false);
    params.set_print_timestamps(false);
    state
        .full(params, audio)
        .map_err(|error| AppError::Import(format!("Whisper 转写失败: {error}")))?;
    let count = state
        .full_n_segments()
        .map_err(|error| AppError::Import(format!("无法读取 Whisper 片段: {error}")))?;
    let mut segments = Vec::new();
    for index in 0..count {
        let text = state
            .full_get_segment_text_lossy(index)
            .map_err(|error| AppError::Import(format!("无法读取 Whisper 文本: {error}")))?
            .trim()
            .to_owned();
        if !text.is_empty() {
            segments.push(WhisperSegment {
                start_ms: state
                    .full_get_segment_t0(index)
                    .map_err(|error| AppError::Import(error.to_string()))?
                    * 10,
                end_ms: state
                    .full_get_segment_t1(index)
                    .map_err(|error| AppError::Import(error.to_string()))?
                    * 10,
                text,
            });
        }
    }
    if segments.is_empty() {
        return Err(AppError::Import("Whisper 没有生成可用片段".into()));
    }
    Ok(segments)
}

#[cfg(target_os = "macos")]
fn silence_whisper_logs() {
    use std::sync::Once;
    static INSTALLED: Once = Once::new();
    INSTALLED.call_once(|| unsafe {
        // whisper.cpp's default diagnostics can contain recognized token text.
        whisper_rs::set_log_callback(Some(discard_whisper_log), std::ptr::null_mut());
    });
}

#[cfg(target_os = "macos")]
unsafe extern "C" fn discard_whisper_log(
    _: u32,
    _: *const std::os::raw::c_char,
    _: *mut std::ffi::c_void,
) {
}

fn find_on_path() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for directory in std::env::split_paths(&path) {
        for name in ["whisper-cli", "main"] {
            let candidate = directory.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

pub fn parse_whisper_json(input: &str) -> AppResult<Vec<WhisperSegment>> {
    let value: Value = serde_json::from_str(input)
        .map_err(|error| AppError::Import(format!("whisper.cpp JSON 无法解析: {error}")))?;
    let entries = value
        .pointer("/result/transcription")
        .or_else(|| value.get("segments"))
        .and_then(Value::as_array)
        .ok_or_else(|| AppError::Import("whisper.cpp JSON 缺少片段".into()))?;
    let mut segments = Vec::with_capacity(entries.len());
    for entry in entries {
        let text = entry
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        if text.is_empty() {
            continue;
        }
        let start_ms = offset_ms(entry, "from")?;
        let end_ms = offset_ms(entry, "to")?;
        if end_ms < start_ms {
            return Err(AppError::Import("whisper.cpp 片段时间范围无效".into()));
        }
        segments.push(WhisperSegment {
            start_ms,
            end_ms,
            text: text.to_owned(),
        });
    }
    if segments.is_empty() {
        return Err(AppError::Import("whisper.cpp 没有可用片段".into()));
    }
    Ok(segments)
}

fn offset_ms(entry: &Value, key: &str) -> AppResult<i64> {
    if let Some(value) = entry
        .pointer(&format!("/offsets/{key}"))
        .and_then(Value::as_i64)
    {
        return Ok(value / 10);
    }
    let timestamp = entry
        .pointer(&format!("/timestamps/{key}"))
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::Import("whisper.cpp 片段缺少时间戳".into()))?;
    parse_timestamp(timestamp)
}

fn parse_timestamp(value: &str) -> AppResult<i64> {
    let normalized = value.replace(',', ".");
    let parts: Vec<_> = normalized.split(':').collect();
    if parts.len() != 3 {
        return Err(AppError::Import("whisper.cpp 时间戳格式无效".into()));
    }
    let hours: i64 = parts[0]
        .parse()
        .map_err(|_| AppError::Import("whisper.cpp 时间戳格式无效".into()))?;
    let minutes: i64 = parts[1]
        .parse()
        .map_err(|_| AppError::Import("whisper.cpp 时间戳格式无效".into()))?;
    let seconds: f64 = parts[2]
        .parse()
        .map_err(|_| AppError::Import("whisper.cpp 时间戳格式无效".into()))?;
    Ok(((hours * 3_600 + minutes * 60) as f64 * 1_000.0 + seconds * 1_000.0).round() as i64)
}

fn redact_message(message: &str) -> String {
    message
        .lines()
        .next()
        .unwrap_or("未知错误")
        .chars()
        .take(240)
        .collect()
}

/* --------------------- v0.5.0：whisperX 外置引擎（说话人分离） --------------------- */

/// 探测 whisperX CLI：优先 ECHO_WHISPERX_BIN，其次 PATH。
pub fn whisperx_path() -> Option<std::path::PathBuf> {
    if let Ok(path) = std::env::var("ECHO_WHISPERX_BIN") {
        let candidate = std::path::PathBuf::from(path.trim());
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    let path = std::env::var_os("PATH")?;
    for directory in std::env::split_paths(&path) {
        let candidate = directory.join("whisperx");
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct WhisperxSegment {
    pub start: f64,
    pub end: f64,
    pub text: String,
    #[serde(default)]
    pub speaker: Option<String>,
}

/// 解析 whisperx --output_format json 的输出，映射为带说话人标注的片段。
pub fn parse_whisperx_json(input: &str) -> AppResult<Vec<WhisperxSegment>> {
    let value: Value = serde_json::from_str(input)
        .map_err(|error| AppError::Import(format!("whisperX JSON 无法解析: {error}")))?;
    let segments = value
        .get("segments")
        .and_then(Value::as_array)
        .ok_or_else(|| AppError::Import("whisperX 输出缺少 segments".to_owned()))?;
    let mut parsed = Vec::new();
    for segment in segments {
        let Ok(item) = serde_json::from_value::<WhisperxSegment>(segment.clone()) else {
            continue;
        };
        if item.text.trim().is_empty() {
            continue;
        }
        parsed.push(item);
    }
    if parsed.is_empty() {
        return Err(AppError::Import("whisperX 没有生成可用片段".to_owned()));
    }
    Ok(parsed)
}

/// 运行 whisperX 转写（含说话人分离）。hf_token 用于 pyannote 模型授权。
pub fn transcribe_whisperx(
    binary: &std::path::Path,
    wav_path: &std::path::Path,
    language: &str,
    hf_token: Option<&str>,
) -> AppResult<Vec<WhisperxSegment>> {
    let output_dir = wav_path
        .parent()
        .map(|parent| parent.to_path_buf())
        .unwrap_or_else(std::env::temp_dir);
    let mut command = std::process::Command::new(binary);
    command
        .arg(wav_path)
        .arg("--language")
        .arg(if language == "auto" { "en" } else { language })
        .arg("--diarize")
        .arg("--output_format")
        .arg("json")
        .arg("--output_dir")
        .arg(&output_dir)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped());
    if let Some(token) = hf_token {
        command.arg("--hf_token").arg(token);
    }
    let output = command
        .output()
        .map_err(|error| AppError::Import(format!("无法启动 whisperX：{error}")))?;
    if !output.status.success() {
        let message = String::from_utf8_lossy(&output.stderr);
        return Err(AppError::Import(format!(
            "whisperX 转写失败：{}",
            message.trim().chars().take(400).collect::<String>()
        )));
    }
    let stem = wav_path
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let json_path = output_dir.join(format!("{stem}.json"));
    let content = std::fs::read_to_string(&json_path)
        .map_err(|error| AppError::Import(format!("无法读取 whisperX 输出：{error}")))?;
    let segments = parse_whisperx_json(&content)?;
    let _ = std::fs::remove_file(&json_path);
    Ok(segments)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_offsets_from_whisper_json() {
        let segments = parse_whisper_json(r#"{"result":{"transcription":[{"offsets":{"from":1250,"to":2750},"text":"  你好  "}]}}"#).unwrap();
        assert_eq!(
            segments,
            vec![WhisperSegment {
                start_ms: 125,
                end_ms: 275,
                text: "你好".into()
            }]
        );
    }
    #[test]
    fn parses_timestamp_fallback() {
        let segments = parse_whisper_json(r#"{"segments":[{"timestamps":{"from":"00:01:02,500","to":"00:01:03,000"},"text":"内容"}]}"#).unwrap();
        assert_eq!(segments[0].start_ms, 62_500);
    }
    #[test]
    fn rejects_invalid_output() {
        assert!(parse_whisper_json("not json").is_err());
        assert!(parse_whisper_json(r#"{"segments":[{"text":"无时间"}]}"#).is_err());
    }

    #[test]
    #[ignore = "requires a local Whisper model and ECHO_TEST_AUDIO"]
    fn transcribes_local_audio() {
        let audio = std::env::var("ECHO_TEST_AUDIO").expect("ECHO_TEST_AUDIO is required");
        let output = std::env::temp_dir().join("echo-memory-whisper-smoke");
        let segments = WhisperAdapter::detect()
            .unwrap()
            .transcribe(Path::new(&audio), "zh", &output)
            .unwrap();
        assert!(!segments.is_empty());
        assert!(segments
            .iter()
            .all(|segment| segment.end_ms >= segment.start_ms));
    }
    #[test]
    fn whisperx_json_parses_speaker_labels() {
        let fixture = r#"{"segments":[
            {"start":0.0,"end":2.5,"text":"大家好","speaker":"SPEAKER_00"},
            {"start":2.5,"end":4.0,"text":"你好","speaker":"SPEAKER_01"},
            {"start":4.0,"end":5.0,"text":"  "}
        ]}"#;
        let segments = parse_whisperx_json(fixture).unwrap();
        assert_eq!(segments.len(), 2);
        assert_eq!(segments[0].speaker.as_deref(), Some("SPEAKER_00"));
        assert!((segments[0].end - 2.5).abs() < f64::EPSILON);
        assert!(parse_whisperx_json("{}").is_err());
    }
}
