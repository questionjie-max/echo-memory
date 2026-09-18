//! Local Whisper adapter: embedded macOS Metal. It never sends audio to a network service.
//!
//! 模型只从本应用自己的模型目录和环境变量里找，不读其它软件的数据目录 —— 依赖别的 App
//! 留下的模型文件，在没装那个 App 的机器上会让转写直接不可用。

use crate::error::{AppError, AppResult};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// 应用内下载的推荐模型。尺寸与校验值只在这里定义一次，
/// 前端和后端的其它位置都从这里读，避免多处字面量各自漂移。
pub const RECOMMENDED_MODEL_ID: &str = "large-v3-turbo-q5_0";
/// GGML 文件名前缀：正式文件是 `{stem}.bin`，下载临时文件是 `.{stem}.download`。
pub const RECOMMENDED_MODEL_STEM: &str = "ggml-large-v3-turbo-q5_0";
pub const RECOMMENDED_MODEL_BYTES: u64 = 574_041_195;
pub const RECOMMENDED_MODEL_SHA256: &str =
    "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2";
pub const RECOMMENDED_MODEL_URL: &str =
    "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo-q5_0.bin";

pub fn recommended_model_file() -> String {
    format!("{RECOMMENDED_MODEL_STEM}.bin")
}

/// 下载临时文件与正式文件同目录，网络中断后靠它续传。
pub fn recommended_model_partial() -> String {
    format!(".{RECOMMENDED_MODEL_STEM}.download")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WhisperSegment {
    pub start_ms: i64,
    pub end_ms: i64,
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct WhisperAdapter {
    model: PathBuf,
}

/// 模型目录里的有效模型文件：`.bin` 结尾，且不是下载中的临时文件。
pub fn is_whisper_model_file(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    !name.starts_with('.')
        && path.extension().and_then(|extension| extension.to_str()) == Some("bin")
        && path.is_file()
}

/// 列出模型目录里可用的模型文件，文件名排序保证顺序稳定。
pub fn library_model_files(models_dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(models_dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| is_whisper_model_file(path))
        .collect();
    files.sort();
    files
}

impl WhisperAdapter {
    pub fn detect() -> AppResult<Self> {
        Self::detect_with_model_path(None)
    }

    pub fn detect_with_model_path(model_path: Option<&str>) -> AppResult<Self> {
        if !cfg!(target_os = "macos") {
            return Err(AppError::Import(
                "当前平台不支持内嵌 Whisper 引擎".to_owned(),
            ));
        }
        if let Some(model) = model_path
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
        {
            if !model.is_file() {
                return Err(AppError::Import("选择的 Whisper 模型文件不存在".to_owned()));
            }
            return Ok(Self { model });
        }
        if let Some(model) = embedded_model() {
            return Ok(Self { model });
        }
        Err(AppError::Import(
            "尚未安装转写模型。请在设置里下载推荐模型，或选择本机已有的 GGML 模型文件。".into(),
        ))
    }

    pub fn model_path(&self) -> Option<&Path> {
        Some(&self.model)
    }

    pub fn model_name(&self) -> String {
        self.model
            .file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or("whisper-rs")
            .to_owned()
    }

    pub fn transcribe_samples(
        &self,
        samples: &[f32],
        language: &str,
        initial_prompt: Option<&str>,
    ) -> AppResult<Vec<WhisperSegment>> {
        #[cfg(target_os = "macos")]
        {
            embedded_transcribe_samples(&self.model, samples, language, initial_prompt)
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (samples, language, initial_prompt);
            Err(AppError::Import(
                "当前平台不支持内嵌 Whisper 引擎".to_owned(),
            ))
        }
    }
}

/// 没有显式配置模型时的回退：先看 `WHISPER_MODEL_PATH`，再在应用自己的模型目录里挑一个。
fn embedded_model() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("WHISPER_MODEL_PATH").map(PathBuf::from) {
        return Some(path);
    }
    pick_library_model(&crate::state::default_library_root().join("models"))
}

/// 在应用自己的模型目录里挑当前模型：优先应用内下载的推荐模型，否则取体积最大的那个。
fn pick_library_model(models_dir: &Path) -> Option<PathBuf> {
    let recommended = models_dir.join(recommended_model_file());
    if recommended.is_file() {
        return Some(recommended);
    }
    let mut candidates: Vec<(u64, PathBuf)> = library_model_files(models_dir)
        .into_iter()
        .filter_map(|path| Some((path.metadata().ok()?.len(), path)))
        .collect();
    candidates.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));
    candidates.into_iter().next().map(|(_, path)| path)
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

/// whisperX 输出的是 `SPEAKER_00` 这类原始标签，转成用户能直接读的中文序号。
/// 认不出来的标签原样保留 —— 用户自己重命名过的说话人不该被覆盖。
pub fn localize_speaker_label(raw: Option<&str>) -> String {
    let Some(raw) = raw.map(str::trim).filter(|label| !label.is_empty()) else {
        return "未知".to_owned();
    };
    let Some(index) = raw.strip_prefix("SPEAKER_") else {
        return raw.to_owned();
    };
    match index.parse::<u32>() {
        Ok(number) => format!("说话人 {}", number + 1),
        Err(_) => raw.to_owned(),
    }
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

    fn scratch_models_dir(label: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("echo-memory-{label}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

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
    fn picks_recommended_model_first() {
        let dir = scratch_models_dir("recommended");
        std::fs::write(dir.join("ggml-medium.bin"), vec![0_u8; 4096]).unwrap();
        std::fs::write(dir.join(recommended_model_file()), b"lmgg").unwrap();
        assert_eq!(
            pick_library_model(&dir).unwrap(),
            dir.join(recommended_model_file())
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn falls_back_to_the_largest_model() {
        let dir = scratch_models_dir("largest");
        std::fs::write(dir.join("ggml-small.bin"), vec![0_u8; 1024]).unwrap();
        std::fs::write(dir.join("ggml-medium.bin"), vec![0_u8; 4096]).unwrap();
        assert_eq!(
            pick_library_model(&dir).unwrap().file_name().unwrap(),
            "ggml-medium.bin"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn ignores_partial_downloads_and_foreign_files() {
        let dir = scratch_models_dir("ignore");
        std::fs::write(dir.join(recommended_model_partial()), vec![0_u8; 4096]).unwrap();
        std::fs::write(dir.join("notes.txt"), b"x").unwrap();
        assert!(pick_library_model(&dir).is_none());
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 干净机器回归：模型来源只能是我们自己的模型目录或环境变量。
    /// 曾经这里会去读另一个 App（Meetily）的模型目录，在有那个 App 的机器上测着没问题，
    /// 换台机器就让转写直接不可用。
    #[test]
    fn never_borrows_another_apps_model_directory() {
        // 拆开拼接，否则断言自身就把要禁止的字符串写进了源码。
        let foreign_bundle_id = ["com", "meetily", "ai"].join(".");
        assert!(!include_str!("whisper.rs").contains(&foreign_bundle_id));
        assert!(!include_str!("commands.rs").contains(&foreign_bundle_id));
    }

    #[test]
    fn localizes_raw_speaker_labels() {
        assert_eq!(localize_speaker_label(Some("SPEAKER_00")), "说话人 1");
        assert_eq!(localize_speaker_label(Some("SPEAKER_01")), "说话人 2");
        assert_eq!(localize_speaker_label(Some("SPEAKER_11")), "说话人 12");
        // 用户改过的名字原样保留，认不出的标签也不乱改。
        assert_eq!(localize_speaker_label(Some("张三")), "张三");
        assert_eq!(localize_speaker_label(Some("SPEAKER_A")), "SPEAKER_A");
        assert_eq!(localize_speaker_label(Some("  ")), "未知");
        assert_eq!(localize_speaker_label(None), "未知");
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
