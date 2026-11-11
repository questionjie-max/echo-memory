//! Privacy-gated third-party ASR transports and long-audio orchestration.

use crate::audio::{plan_provider_chunks, read_normalized_wav, AudioChunk};
use crate::error::{AppError, AppResult};
use crate::external_ai_gate::{
    credentials_are_complete, get_asr_credentials, require_external_asr_consent,
};
use crate::library::ManagedLibrary;
use crate::memory::{assert_endpoint_still_safe, validate_external_base_url};
use crate::provider_catalog;
use crate::types::TranscriptSegmentInput;
use crate::whisper::{localize_speaker_label, WhisperSegment};
use base64::Engine as _;
use chrono::Utc;
use hmac::{Hmac, Mac};
use serde::Deserialize;
use serde_json::{json, Value};
use sha1::Sha1;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::time::Duration;

type HmacSha1 = Hmac<Sha1>;

#[derive(Debug, Clone)]
pub struct ExternalAsrRequest {
    pub provider_id: String,
    pub base_url: String,
    pub model: String,
    pub credentials: BTreeMap<String, String>,
    pub audio_path: PathBuf,
    pub language: String,
    pub duration_ms: i64,
}

#[derive(Debug)]
pub struct ExternalAsrResult {
    pub segments: Vec<TranscriptSegmentInput>,
    pub preprocessing_json: String,
}

/// Runs a transport only after the audio-upload consent gate and credential lookup.
pub fn transcribe_with_transport<F>(
    library: &ManagedLibrary,
    audio_path: &Path,
    language: &str,
    duration_ms: i64,
    transport: F,
) -> AppResult<ExternalAsrResult>
where
    F: FnOnce(ExternalAsrRequest) -> AppResult<ExternalAsrResult>,
{
    let settings = require_external_asr_consent(library)?;
    let credentials = get_asr_credentials()?
        .ok_or_else(|| AppError::Invalid("请先配置第三方转写凭据".to_owned()))?;
    if !credentials_are_complete(&settings.transcription_provider, Some(&credentials)) {
        return Err(AppError::Invalid(
            "第三方转写凭据不完整，请重新填写".to_owned(),
        ));
    }
    let base_url = validate_external_base_url(&settings.transcription_base_url)?;
    if settings.transcription_model.trim().is_empty() {
        return Err(AppError::Invalid("第三方转写模型不能为空".to_owned()));
    }
    transport(ExternalAsrRequest {
        provider_id: settings.transcription_provider,
        base_url,
        model: settings.transcription_model.trim().to_owned(),
        credentials,
        audio_path: audio_path.to_path_buf(),
        language: language.trim().to_owned(),
        duration_ms,
    })
}

/// 用一段 1 秒测试音真实调用一次转写接口，验证地址、凭据、模型和协议。
///
/// 多数 ASR 服务商没有免费鉴权探针；这里会发一次极小的真实请求，可能产生
/// 可忽略的少量费用。测试音没有可识别语音时，服务商返回合法空结果也算连通。
pub fn probe_external_asr_connection(library: &ManagedLibrary) -> AppResult<()> {
    let probe_path =
        std::env::temp_dir().join(format!("echo-asr-probe-{}.wav", uuid::Uuid::new_v4()));
    let samples = (0..16_000)
        .map(|index| {
            let phase = index as f32 * 440.0 * std::f32::consts::TAU / 16_000.0;
            phase.sin() * 0.05
        })
        .collect::<Vec<_>>();
    write_wav_chunk(&probe_path, &samples)?;
    let result = transcribe_with_transport(library, &probe_path, "zh", 1_000, |request| {
        match send_transcription_request(&request) {
            Ok(segments) => Ok(ExternalAsrResult {
                segments,
                preprocessing_json: "{}".to_owned(),
            }),
            Err(error) if is_empty_transcript_response(&error) => Ok(ExternalAsrResult {
                segments: Vec::new(),
                preprocessing_json: "{}".to_owned(),
            }),
            Err(error) => Err(error),
        }
    });
    let _ = std::fs::remove_file(&probe_path);
    result.map(|_| ())
}

fn is_empty_transcript_response(error: &AppError) -> bool {
    let message = error.to_string();
    message.contains("没有返回可用文本") || message.contains("没有返回可用结果")
}

/// Sends one already-normalized audio chunk using the provider protocol.
pub fn send_transcription_request(
    request: &ExternalAsrRequest,
) -> AppResult<Vec<TranscriptSegmentInput>> {
    if !request.audio_path.is_file() {
        return Err(AppError::Import("待转写音频文件不存在".to_owned()));
    }
    let capability =
        provider_catalog::capability(&request.provider_id, "transcription").ok_or_else(|| {
            AppError::Invalid("该服务商不支持第三方转写".to_owned())
        })?;
    match capability.protocol.as_str() {
        "openai-multipart" => send_multipart(request, MultipartFlavor::OpenAi),
        "zhipu-multipart" => send_multipart(request, MultipartFlavor::Zhipu),
        "dashscope-chat-audio" => send_dashscope(request),
        "tencent-flash" => send_tencent(request),
        "doubao-flash" => send_doubao(request),
        other => Err(AppError::Invalid(format!(
            "不支持的第三方转写协议：{other}"
        ))),
    }
}

/// Splits long audio according to provider limits and transcribes chunks with
/// bounded overlap. Completed chunks are cached so retry only sends missing
/// segments and does not bill the same audio twice.
pub fn transcribe_long_audio<F>(
    request: &ExternalAsrRequest,
    cache_dir: &Path,
    mut progress: F,
) -> AppResult<Vec<TranscriptSegmentInput>>
where
    F: FnMut(usize, usize) -> AppResult<()>,
{
    let capability =
        provider_catalog::capability(&request.provider_id, "transcription").ok_or_else(|| {
            AppError::Invalid("该服务商不支持第三方转写".to_owned())
        })?;
    let samples = read_normalized_wav(&request.audio_path)?;
    let max_duration = capability.max_duration_ms.unwrap_or(1_200_000);
    let max_bytes = capability.max_file_bytes.unwrap_or(24 * 1024 * 1024);
    let chunks = plan_provider_chunks(
        &samples,
        max_duration,
        max_bytes,
        2_000,
        capability.returns_timestamps,
    );
    let cache_namespace = cache_namespace(request);
    let cache_root = cache_dir.join(&cache_namespace);
    std::fs::create_dir_all(&cache_root)?;
    let mut all = Vec::<MergedSegment>::new();
    for (index, chunk) in chunks.iter().enumerate() {
        progress(index, chunks.len())?;
        let cache_path = cache_root.join(format!("segment-{:04}.json", index + 1));
        let relative_segments = if cache_path.is_file() {
            serde_json::from_str::<Vec<CachedSegment>>(
                &std::fs::read_to_string(&cache_path)
                    .map_err(|error| AppError::Import(format!("无法读取转写分段缓存：{error}")))?,
            )
            .map_err(|_| AppError::Import("第三方转写分段缓存已损坏".to_owned()))?
            .into_iter()
            .map(CachedSegment::into_input)
            .collect::<Vec<_>>()
        } else {
            let chunk_path = cache_root.join(format!("segment-{:04}.wav", index + 1));
            write_wav_chunk(&chunk_path, &samples[chunk.sample_start..chunk.sample_end])?;
            let mut chunk_request = request.clone();
            chunk_request.audio_path = chunk_path.clone();
            chunk_request.duration_ms = chunk.accept_end_ms - chunk.accept_start_ms;
            let result = send_transcription_request(&chunk_request).map_err(|error| {
                AppError::ExternalAi(format!(
                    "第 {}/{} 段转写失败：{}",
                    index + 1,
                    chunks.len(),
                    error
                ))
            });
            let _ = std::fs::remove_file(&chunk_path);
            let segments = result?;
            let cached = segments
                .iter()
                .map(CachedSegment::from_input)
                .collect::<Vec<_>>();
            let serialized = serde_json::to_vec(&cached)
                .map_err(|_| AppError::Import("无法序列化转写分段缓存".to_owned()))?;
            let temporary = cache_path.with_extension("json.tmp");
            std::fs::write(&temporary, serialized)?;
            std::fs::rename(&temporary, &cache_path)?;
            segments
        };
        merge_chunk_segments(
            &mut all,
            relative_segments,
            chunk,
            capability.returns_timestamps,
            request.duration_ms,
        );
        progress(index + 1, chunks.len())?;
    }
    let mut normalized = all
        .into_iter()
        .filter(|segment| !segment.text.trim().is_empty())
        .map(|segment| TranscriptSegmentInput {
            start_ms: segment.start_ms.max(0),
            end_ms: segment.end_ms.max(segment.start_ms + 1),
            speaker_label: segment.speaker_label.or_else(|| Some("未知".to_owned())),
            original_text: segment.text.trim().to_owned(),
        })
        .collect::<Vec<_>>();
    normalize_input_segments(&mut normalized);
    Ok(normalized)
}

fn cache_namespace(request: &ExternalAsrRequest) -> String {
    let digest = Sha256::digest(format!("{}\n{}", request.model, request.language).as_bytes());
    let suffix = digest[..6]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("{}-{suffix}", safe_filename(&request.provider_id))
}

#[derive(Debug)]
struct MergedSegment {
    start_ms: i64,
    end_ms: i64,
    text: String,
    speaker_label: Option<String>,
}

fn merge_chunk_segments(
    merged: &mut Vec<MergedSegment>,
    relative: Vec<TranscriptSegmentInput>,
    chunk: &AudioChunk,
    returns_timestamps: bool,
    total_duration_ms: i64,
) {
    let offset = chunk.sample_start as i64 * 1_000 / 16_000;
    for segment in relative {
        let start = (segment.start_ms + offset).min(total_duration_ms);
        let end = (segment.end_ms + offset)
            .max(start + 1)
            .min(total_duration_ms.max(start + 1));
        if returns_timestamps {
            let midpoint = (segment.start_ms + segment.end_ms) / 2 + offset;
            if midpoint <= chunk.accept_start_ms || midpoint >= chunk.accept_end_ms {
                continue;
            }
        }
        let candidate = MergedSegment {
            start_ms: start,
            end_ms: end,
            text: segment.original_text,
            speaker_label: segment.speaker_label,
        };
        let Some(previous) = merged.last_mut() else {
            merged.push(candidate);
            continue;
        };
        if returns_timestamps {
            let previous_whisper = WhisperSegment {
                start_ms: previous.start_ms,
                end_ms: previous.end_ms,
                text: previous.text.clone(),
            };
            let candidate_whisper = WhisperSegment {
                start_ms: candidate.start_ms,
                end_ms: candidate.end_ms,
                text: candidate.text.clone(),
            };
            if crate::audio::is_overlap_duplicate(&previous_whisper, &candidate_whisper) {
                continue;
            }
            let mut merge_target = previous_whisper;
            if crate::audio::merge_overlap_continuation(&mut merge_target, &candidate_whisper) {
                previous.text = merge_target.text;
                previous.end_ms = previous.end_ms.max(candidate.end_ms);
                if previous.speaker_label.is_none() {
                    previous.speaker_label = candidate.speaker_label;
                }
                continue;
            }
        }
        merged.push(candidate);
    }
}

fn normalize_input_segments(segments: &mut [TranscriptSegmentInput]) {
    segments.sort_by_key(|segment| (segment.start_ms, segment.end_ms));
    let mut previous_end = 0;
    for segment in segments {
        segment.start_ms = segment.start_ms.max(previous_end);
        segment.end_ms = segment.end_ms.max(segment.start_ms + 1);
        previous_end = segment.end_ms;
    }
}

#[derive(Debug, Deserialize, serde::Serialize)]
struct CachedSegment {
    start_ms: i64,
    end_ms: i64,
    speaker_label: Option<String>,
    original_text: String,
}

impl CachedSegment {
    fn from_input(segment: &TranscriptSegmentInput) -> Self {
        Self {
            start_ms: segment.start_ms,
            end_ms: segment.end_ms,
            speaker_label: segment.speaker_label.clone(),
            original_text: segment.original_text.clone(),
        }
    }

    fn into_input(self) -> TranscriptSegmentInput {
        TranscriptSegmentInput {
            start_ms: self.start_ms,
            end_ms: self.end_ms,
            speaker_label: self.speaker_label,
            original_text: self.original_text,
        }
    }
}

pub(crate) fn write_wav_chunk(path: &Path, samples: &[f32]) -> AppResult<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 16_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec)
        .map_err(|error| AppError::Import(format!("无法生成转写分段音频：{error}")))?;
    for sample in samples {
        writer
            .write_sample((sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
            .map_err(|error| AppError::Import(format!("无法写入转写分段音频：{error}")))?;
    }
    writer
        .finalize()
        .map_err(|error| AppError::Import(format!("无法完成转写分段音频：{error}")))
}

fn safe_filename(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect()
}

#[derive(Clone, Copy)]
enum MultipartFlavor {
    OpenAi,
    Zhipu,
}

fn send_multipart(
    request: &ExternalAsrRequest,
    flavor: MultipartFlavor,
) -> AppResult<Vec<TranscriptSegmentInput>> {
    let endpoint = endpoint_for_base(&request.base_url, "audio/transcriptions")?;
    assert_endpoint_still_safe(&endpoint)?;
    let file = File::open(&request.audio_path)
        .map_err(|error| AppError::Import(format!("无法读取待转写音频：{error}")))?;
    let boundary = format!("----echo-asr-{}", uuid::Uuid::new_v4());
    let prefix = multipart_prefix(&boundary, &request.audio_path);
    let suffix = multipart_suffix(&boundary, request, flavor);
    let file_len = file
        .metadata()
        .map_err(|error| AppError::Import(format!("无法读取待转写音频大小：{error}")))?
        .len();
    let content_len = (prefix.len() as u64)
        .checked_add(file_len)
        .and_then(|value| value.checked_add(suffix.len() as u64))
        .ok_or_else(|| AppError::Import("音频请求长度溢出".to_owned()))?;
    let body = MultipartAudioReader {
        prefix: Cursor::new(prefix),
        file,
        suffix: Cursor::new(suffix),
        phase: 0,
    };
    let response = agent()
        .post(&endpoint)
        .set(
            "Authorization",
            &format!("Bearer {}", credential(request, "apiKey")?),
        )
        .set(
            "Content-Type",
            &format!("multipart/form-data; boundary={boundary}"),
        )
        .set("Content-Length", &content_len.to_string())
        .send(body)
        .map_err(|error| request_error(error, &request.credentials))?;
    let body = read_response_body(response, &request.credentials)?;
    parse_openai_response(&body, request.duration_ms)
}

fn endpoint_for_base(base_url: &str, suffix: &str) -> AppResult<String> {
    let base = validate_external_base_url(base_url)?;
    Ok(if base.ends_with(&format!("/{suffix}")) {
        base
    } else {
        format!("{base}/{suffix}")
    })
}

fn multipart_prefix(boundary: &str, audio_path: &Path) -> Vec<u8> {
    let extension = audio_path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("wav");
    let content_type = match extension.to_ascii_lowercase().as_str() {
        "mp3" => "audio/mpeg",
        "m4a" | "mp4" => "audio/mp4",
        "wav" => "audio/wav",
        _ => "application/octet-stream",
    };
    format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"audio.{extension}\"\r\nContent-Type: {content_type}\r\n\r\n"
    )
    .into_bytes()
}

fn multipart_suffix(
    boundary: &str,
    request: &ExternalAsrRequest,
    flavor: MultipartFlavor,
) -> Vec<u8> {
    let mut value = format!(
        "\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"model\"\r\n\r\n{}",
        request.model
    );
    if matches!(flavor, MultipartFlavor::OpenAi) {
        value.push_str(&format!(
            "\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"response_format\"\r\n\r\nverbose_json"
        ));
        if !request.language.is_empty() && request.language != "auto" {
            value.push_str(&format!(
                "\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"language\"\r\n\r\n{}",
                request.language
            ));
        }
    } else {
        value.push_str(&format!(
            "\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"stream\"\r\n\r\nfalse"
        ));
    }
    value.push_str(&format!("\r\n--{boundary}--\r\n"));
    value.into_bytes()
}

struct MultipartAudioReader {
    prefix: Cursor<Vec<u8>>,
    file: File,
    suffix: Cursor<Vec<u8>>,
    phase: u8,
}

impl Read for MultipartAudioReader {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if self.phase == 0 {
            let read = self.prefix.read(buffer)?;
            if read == 0 {
                self.phase = 1;
                return self.file.read(buffer);
            }
            return Ok(read);
        }
        if self.phase == 1 {
            let read = self.file.read(buffer)?;
            if read == 0 {
                self.phase = 2;
                return self.suffix.read(buffer);
            }
            return Ok(read);
        }
        self.suffix.read(buffer)
    }
}

fn send_dashscope(request: &ExternalAsrRequest) -> AppResult<Vec<TranscriptSegmentInput>> {
    let endpoint = endpoint_for_base(&request.base_url, "chat/completions")?;
    assert_endpoint_still_safe(&endpoint)?;
    let audio = std::fs::read(&request.audio_path)
        .map_err(|error| AppError::Import(format!("无法读取待转写音频：{error}")))?;
    let data = format!(
        "data:audio/wav;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(audio)
    );
    let mut payload = json!({
        "model": request.model,
        "messages": [{
            "role": "user",
            "content": [{"type": "input_audio", "input_audio": {"data": data}}]
        }],
        "stream": false,
        "asr_options": {"enable_itn": false}
    });
    if !request.language.is_empty() && request.language != "auto" {
        payload["asr_options"]["language"] = Value::String(request.language.clone());
    }
    let response = agent()
        .post(&endpoint)
        .set(
            "Authorization",
            &format!("Bearer {}", credential(request, "apiKey")?),
        )
        .set("Content-Type", "application/json")
        .send_json(payload)
        .map_err(|error| request_error(error, &request.credentials))?;
    let body = read_response_body(response, &request.credentials)?;
    let value = parse_json_body(&body)?;
    let content = value
        .pointer("/choices/0/message/content")
        .ok_or_else(|| AppError::ExternalAi("阿里百炼转写响应缺少文本".to_owned()))?;
    let text = content_as_text(content).ok_or_else(|| {
        AppError::ExternalAi("阿里百炼转写响应不是有效文本".to_owned())
    })?;
    plain_text_segments(&text, request.duration_ms)
}

fn send_tencent(request: &ExternalAsrRequest) -> AppResult<Vec<TranscriptSegmentInput>> {
    let app_id = credential(request, "appId")?;
    let secret_id = credential(request, "secretId")?;
    let secret_key = credential(request, "secretKey")?;
    let timestamp = Utc::now().timestamp();
    let (endpoint, signature) =
        tencent_signed_endpoint(app_id, secret_id, secret_key, timestamp, request);
    assert_endpoint_still_safe(&endpoint)?;
    let audio = std::fs::read(&request.audio_path)
        .map_err(|error| AppError::Import(format!("无法读取待转写音频：{error}")))?;
    let response = agent()
        .post(&endpoint)
        .set("Authorization", &signature)
        .set("Content-Type", "application/octet-stream")
        .send_bytes(&audio)
        .map_err(|error| request_error(error, &request.credentials))?;
    let body = read_response_body(response, &request.credentials)?;
    parse_tencent_response(&body, request.duration_ms, &request.credentials)
}

fn tencent_signed_endpoint(
    app_id: &str,
    secret_id: &str,
    secret_key: &str,
    timestamp: i64,
    request: &ExternalAsrRequest,
) -> (String, String) {
    let speaker_diarization = if request.model.starts_with("16k_zh") {
        "1"
    } else {
        "0"
    };
    let mut params = vec![
        ("convert_num_mode", "1".to_owned()),
        ("engine_type", request.model.clone()),
        ("filter_dirty", "0".to_owned()),
        ("filter_modal", "0".to_owned()),
        ("filter_punc", "0".to_owned()),
        ("first_channel_only", "1".to_owned()),
        ("secretid", secret_id.to_owned()),
        ("speaker_diarization", speaker_diarization.to_owned()),
        ("timestamp", timestamp.to_string()),
        ("voice_format", "wav".to_owned()),
        ("word_info", "0".to_owned()),
    ];
    params.sort_by(|left, right| left.0.cmp(right.0));
    let query = params
        .iter()
        .map(|(key, value)| format!("{}={}", percent_encode(key), percent_encode(value)))
        .collect::<Vec<_>>()
        .join("&");
    let path = format!("/asr/flash/v1/{app_id}");
    let sign_plain = format!("POSTasr.cloud.tencent.com{path}?{query}");
    let mut mac =
        HmacSha1::new_from_slice(secret_key.as_bytes()).expect("HMAC accepts arbitrary keys");
    mac.update(sign_plain.as_bytes());
    let signature = base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes());
    (
        format!("https://asr.cloud.tencent.com{path}?{query}"),
        signature,
    )
}

fn send_doubao(request: &ExternalAsrRequest) -> AppResult<Vec<TranscriptSegmentInput>> {
    let endpoint = request.base_url.trim().trim_end_matches('/').to_owned();
    assert_endpoint_still_safe(&endpoint)?;
    let api_key = credential(request, "apiKey")?;
    let audio = std::fs::read(&request.audio_path)
        .map_err(|error| AppError::Import(format!("无法读取待转写音频：{error}")))?;
    let payload = json!({
        "user": {"uid": credential_fingerprint(api_key)},
        "audio": {
            "format": "wav",
            "rate": 16_000,
            "bits": 16,
            "channel": 1,
            "data": base64::engine::general_purpose::STANDARD.encode(audio)
        },
        "request": {
            "model_name": "bigmodel",
            "enable_itn": true,
            "enable_punc": true,
            "enable_ddc": true,
            "enable_speaker_info": true,
            "show_utterances": true
        }
    });
    let response = agent()
        .post(&endpoint)
        .set("X-Api-Key", api_key)
        .set("X-Api-Resource-Id", &request.model)
        .set("X-Api-Request-Id", &uuid::Uuid::new_v4().to_string())
        .set("X-Api-Sequence", "-1")
        .set("Content-Type", "application/json")
        .send_json(payload)
        .map_err(|error| request_error(error, &request.credentials))?;
    let status_code = response
        .header("X-Api-Status-Code")
        .unwrap_or("20000000")
        .to_owned();
    let provider_message = response
        .header("X-Api-Message")
        .unwrap_or_default()
        .to_owned();
    let body = read_response_body(response, &request.credentials)?;
    if status_code != "20000000" {
        return Err(AppError::ExternalAi(format!(
            "豆包语音返回错误（{}）：{}",
            status_code,
            sanitize_provider_message(&provider_message, &request.credentials)
        )));
    }
    parse_doubao_response(&body, request.duration_ms)
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout(Duration::from_secs(300))
        .redirects(0)
        .build()
}

fn credential<'a>(request: &'a ExternalAsrRequest, key: &str) -> AppResult<&'a str> {
    request
        .credentials
        .get(key)
        .map(String::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| AppError::Invalid("第三方转写凭据不完整".to_owned()))
}

fn read_response_body(
    response: ureq::Response,
    credentials: &BTreeMap<String, String>,
) -> AppResult<String> {
    let status = response.status();
    let body = response
        .into_string()
        .map_err(|_| AppError::ExternalAi("第三方转写返回了无效响应".to_owned()))?;
    if !(200..300).contains(&status) {
        return Err(AppError::ExternalAi(format!(
            "第三方转写服务返回错误（HTTP {status}）：{}",
            sanitize_provider_message(&body, credentials)
        )));
    }
    Ok(body)
}

fn request_error(error: ureq::Error, credentials: &BTreeMap<String, String>) -> AppError {
    match error {
        ureq::Error::Status(code, response) => {
            let body = response.into_string().unwrap_or_default();
            AppError::ExternalAi(format!(
                "第三方转写服务返回错误（HTTP {code}）：{}",
                sanitize_provider_message(&body, credentials)
            ))
        }
        ureq::Error::Transport(_) => {
            AppError::ExternalAi("无法连接第三方转写服务，请检查地址与网络".to_owned())
        }
    }
}

pub(crate) fn sanitize_provider_message(raw: &str, credentials: &BTreeMap<String, String>) -> String {
    let value: Value = serde_json::from_str(raw).unwrap_or(Value::Null);
    let message = value
        .pointer("/error/message")
        .and_then(Value::as_str)
        .or_else(|| value.pointer("/Response/Error/Message").and_then(Value::as_str))
        .or_else(|| value.get("message").and_then(Value::as_str))
        .unwrap_or_else(|| raw.trim());
    let mut message = message.chars().take(500).collect::<String>();
    for secret in credentials.values() {
        if !secret.is_empty() {
            message = message.replace(secret, "***");
        }
    }
    if message.trim().is_empty() {
        "服务拒绝了本次请求".to_owned()
    } else {
        message
    }
}

fn credential_fingerprint(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    format!("echo-{}", &digest[..8].iter().map(|byte| format!("{byte:02x}")).collect::<String>())
}

fn parse_json_body(body: &str) -> AppResult<Value> {
    serde_json::from_str(body)
        .map_err(|_| AppError::ExternalAi("第三方转写响应不是有效 JSON".to_owned()))
}

fn content_as_text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Array(items) => Some(
            items
                .iter()
                .filter_map(|item| {
                    item.get("text")
                        .and_then(Value::as_str)
                        .or_else(|| item.as_str())
                })
                .collect::<Vec<_>>()
                .join(""),
        ),
        _ => None,
    }
}

#[derive(Debug, Deserialize)]
struct TranscriptionResponse {
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    segments: Option<Vec<TranscriptionSegment>>,
}

#[derive(Debug, Deserialize)]
struct TranscriptionSegment {
    #[serde(default)]
    start: Option<f64>,
    #[serde(default)]
    end: Option<f64>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    speaker: Option<Value>,
    #[serde(default)]
    speaker_label: Option<Value>,
    #[serde(default)]
    speaker_id: Option<Value>,
    #[serde(default)]
    person: Option<Value>,
}

fn value_as_text(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

fn speaker_from_segment(segment: &TranscriptionSegment) -> Option<String> {
    let raw = [
        segment.speaker.as_ref(),
        segment.speaker_label.as_ref(),
        segment.speaker_id.as_ref(),
        segment.person.as_ref(),
    ]
    .into_iter()
    .flatten()
    .find_map(value_as_text)?;
    let normalized = raw.trim();
    (!normalized.is_empty()).then(|| localize_speaker_label(Some(normalized)))
}

fn parse_openai_response(
    body: &str,
    duration_ms: i64,
) -> AppResult<Vec<TranscriptSegmentInput>> {
    let response: TranscriptionResponse = serde_json::from_str(body)
        .map_err(|_| AppError::ExternalAi("第三方转写响应不是有效 JSON".to_owned()))?;
    if let Some(segments) = response.segments {
        let mut inputs = Vec::new();
        for segment in segments {
            let text = segment.text.as_deref().unwrap_or("").trim().to_owned();
            if text.is_empty() {
                continue;
            }
            let start = seconds_to_ms(segment.start.unwrap_or(0.0));
            let end = seconds_to_ms(segment.end.unwrap_or(0.0)).max(start + 1);
            inputs.push(TranscriptSegmentInput {
                start_ms: start,
                end_ms: end,
                speaker_label: Some(
                    speaker_from_segment(&segment).unwrap_or_else(|| "未知".to_owned()),
                ),
                original_text: text,
            });
        }
        if !inputs.is_empty() {
            return Ok(inputs);
        }
    }
    let text = response.text.unwrap_or_default().trim().to_owned();
    if text.is_empty() {
        return Err(AppError::ExternalAi(
            "第三方转写没有返回可用文本".to_owned(),
        ));
    }
    plain_text_segments(&text, duration_ms)
}

#[derive(Debug, Deserialize)]
struct TencentResponse {
    #[serde(default)]
    code: i64,
    #[serde(default)]
    message: String,
    #[serde(default)]
    audio_duration: Option<i64>,
    #[serde(default)]
    flash_result: Vec<TencentFlashResult>,
}

#[derive(Debug, Deserialize)]
struct TencentFlashResult {
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    sentence_list: Vec<TencentSentence>,
}

#[derive(Debug, Deserialize)]
struct TencentSentence {
    #[serde(default)]
    text: String,
    #[serde(default)]
    start_time: i64,
    #[serde(default)]
    end_time: i64,
    #[serde(default)]
    speaker_id: Option<i64>,
}

fn parse_tencent_response(
    body: &str,
    fallback_duration_ms: i64,
    credentials: &BTreeMap<String, String>,
) -> AppResult<Vec<TranscriptSegmentInput>> {
    let response: TencentResponse = serde_json::from_str(body)
        .map_err(|_| AppError::ExternalAi("腾讯云转写响应不是有效 JSON".to_owned()))?;
    if response.code != 0 {
        return Err(AppError::ExternalAi(format!(
            "腾讯云转写失败（{}）：{}",
            response.code,
            sanitize_provider_message(&response.message, credentials)
        )));
    }
    let mut segments = Vec::new();
    for item in response.flash_result {
        for sentence in item.sentence_list {
            let text = sentence.text.trim().to_owned();
            if text.is_empty() {
                continue;
            }
            segments.push(TranscriptSegmentInput {
                start_ms: sentence.start_time,
                end_ms: sentence.end_time.max(sentence.start_time + 1),
                speaker_label: sentence
                    .speaker_id
                    .map(|speaker| localize_speaker_label(Some(&speaker.to_string()))),
                original_text: text,
            });
        }
        if segments.is_empty() {
            if let Some(text) = item.text.filter(|text| !text.trim().is_empty()) {
                segments.push(TranscriptSegmentInput {
                    start_ms: 0,
                    end_ms: response.audio_duration.unwrap_or(fallback_duration_ms).max(1),
                    speaker_label: None,
                    original_text: text.trim().to_owned(),
                });
            }
        }
    }
    if segments.is_empty() {
        return Err(AppError::ExternalAi(
            "腾讯云转写没有返回可用文本".to_owned(),
        ));
    }
    Ok(segments)
}

#[derive(Debug, Deserialize)]
struct DoubaoResponse {
    #[serde(default)]
    audio_info: Option<DoubaoAudioInfo>,
    #[serde(default)]
    result: Option<DoubaoResult>,
}

#[derive(Debug, Deserialize)]
struct DoubaoAudioInfo {
    #[serde(default)]
    duration: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct DoubaoResult {
    #[serde(default)]
    text: String,
    #[serde(default)]
    utterances: Vec<DoubaoUtterance>,
}

#[derive(Debug, Deserialize)]
struct DoubaoUtterance {
    #[serde(default)]
    text: String,
    #[serde(default)]
    start_time: i64,
    #[serde(default)]
    end_time: i64,
    #[serde(default)]
    additions: Option<DoubaoAdditions>,
}

#[derive(Debug, Deserialize)]
struct DoubaoAdditions {
    #[serde(default)]
    speaker: Option<String>,
}

fn parse_doubao_response(
    body: &str,
    fallback_duration_ms: i64,
) -> AppResult<Vec<TranscriptSegmentInput>> {
    let response: DoubaoResponse = serde_json::from_str(body)
        .map_err(|_| AppError::ExternalAi("豆包语音转写响应不是有效 JSON".to_owned()))?;
    let Some(result) = response.result else {
        return Err(AppError::ExternalAi(
            "豆包语音转写没有返回结果".to_owned(),
        ));
    };
    let mut segments = result
        .utterances
        .into_iter()
        .filter_map(|utterance| {
            let text = utterance.text.trim().to_owned();
            if text.is_empty() {
                return None;
            }
            Some(TranscriptSegmentInput {
                start_ms: utterance.start_time,
                end_ms: utterance.end_time.max(utterance.start_time + 1),
                speaker_label: utterance
                    .additions
                    .and_then(|additions| additions.speaker)
                    .filter(|speaker| !speaker.trim().is_empty())
                    .map(|speaker| localize_speaker_label(Some(&speaker))),
                original_text: text,
            })
        })
        .collect::<Vec<_>>();
    if segments.is_empty() && !result.text.trim().is_empty() {
        let duration = response
            .audio_info
            .and_then(|info| info.duration)
            .unwrap_or(fallback_duration_ms)
            .max(1);
        segments = plain_text_segments(result.text.trim(), duration)?;
    }
    if segments.is_empty() {
        return Err(AppError::ExternalAi(
            "豆包语音转写没有返回可用文本".to_owned(),
        ));
    }
    Ok(segments)
}

fn seconds_to_ms(seconds: f64) -> i64 {
    if !seconds.is_finite() || seconds < 0.0 {
        0
    } else {
        (seconds * 1000.0).round() as i64
    }
}

fn plain_text_segments(
    text: &str,
    duration_ms: i64,
) -> AppResult<Vec<TranscriptSegmentInput>> {
    let text = text.trim();
    if text.is_empty() {
        return Err(AppError::ExternalAi(
            "第三方转写没有返回可用文本".to_owned(),
        ));
    }
    Ok(synthesize_segments(text, duration_ms))
}

fn synthesize_segments(text: &str, duration_ms: i64) -> Vec<TranscriptSegmentInput> {
    let mut chunks = Vec::new();
    let mut current = String::new();
    for character in text.chars() {
        current.push(character);
        if current.chars().count() >= 180 || matches!(character, '。' | '！' | '？' | '；' | '\n')
        {
            chunks.push(std::mem::take(&mut current));
        }
    }
    if !current.trim().is_empty() {
        chunks.push(current);
    }
    if chunks.is_empty() {
        return Vec::new();
    }
    let total_chars = chunks
        .iter()
        .map(|chunk| chunk.chars().count())
        .sum::<usize>()
        .max(1) as i64;
    let total_duration = duration_ms.max(chunks.len() as i64);
    let mut start = 0;
    chunks
        .into_iter()
        .map(|chunk| {
            let chars = chunk.chars().count().max(1) as i64;
            let mut end = start + (total_duration * chars / total_chars).max(1);
            end = end.min(total_duration);
            let input = TranscriptSegmentInput {
                start_ms: start,
                end_ms: end,
                speaker_label: Some("未知".to_owned()),
                original_text: chunk.trim().to_owned(),
            };
            start = end;
            input
        })
        .filter(|input| !input.original_text.is_empty())
        .collect()
}

fn percent_encode(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::external_ai_gate::{parse_credentials, AsrApiKeyEnvGuard};
    use std::cell::Cell;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;
    use uuid::Uuid;

    fn temp_library() -> (ManagedLibrary, PathBuf) {
        let root = std::env::temp_dir().join(format!("external-asr-{}", Uuid::new_v4()));
        let library = ManagedLibrary::open(&root).unwrap();
        (library, root)
    }

    fn write_test_wav(path: &Path, seconds: u32) {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 16_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(path, spec).unwrap();
        for _ in 0..(16_000 * seconds) {
            writer.write_sample(320_i16).unwrap();
        }
        writer.finalize().unwrap();
    }

    fn serve_one_response(
        status: &str,
        headers: &[(&str, &str)],
        response_body: &str,
    ) -> (String, thread::JoinHandle<String>) {
        let (base_url, handle) =
            serve_responses(vec![(status.to_owned(), response_body.to_owned())], headers);
        (
            base_url,
            thread::spawn(move || handle.join().unwrap().into_iter().next().unwrap()),
        )
    }

    fn serve_responses(
        responses: Vec<(String, String)>,
        headers: &[(&str, &str)],
    ) -> (String, thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let headers = headers
            .iter()
            .map(|(key, value)| format!("{key}: {value}\r\n"))
            .collect::<String>();
        let handle = thread::spawn(move || {
            let mut received = Vec::new();
            let total = responses.len();
            for (index, (status, response_body)) in responses.into_iter().enumerate() {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let request = read_http_request(&mut stream);
                write!(
                    stream,
                    "HTTP/1.1 {status}\r\n{headers}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    response_body.len(),
                    response_body
                )
                .unwrap();
                received.push(String::from_utf8_lossy(&request).into_owned());
                if index + 1 == total {
                    break;
                }
            }
            received
        });
        (format!("http://{address}"), handle)
    }

    fn read_http_request(stream: &mut std::net::TcpStream) -> Vec<u8> {
        let mut request = Vec::new();
        let mut buffer = [0_u8; 8_192];
        loop {
            let read = stream.read(&mut buffer).unwrap();
            if read == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..read]);
            let Some(header_end) = request.windows(4).position(|window| window == b"\r\n\r\n")
            else {
                continue;
            };
            let request_headers =
                String::from_utf8_lossy(&request[..header_end]).to_ascii_lowercase();
            let content_length = request_headers
                .lines()
                .find_map(|line| line.strip_prefix("content-length:"))
                .and_then(|value| value.trim().parse::<usize>().ok())
                .unwrap_or(0);
            if request.len() >= header_end + 4 + content_length {
                break;
            }
        }
        request
    }

    fn request(
        provider_id: &str,
        base_url: String,
        model: &str,
        credentials: &[(&str, &str)],
    ) -> (ExternalAsrRequest, PathBuf) {
        let audio_path =
            std::env::temp_dir().join(format!("asr-request-{}.wav", Uuid::new_v4()));
        std::fs::write(&audio_path, b"AUDIO-BYTES").unwrap();
        (
            ExternalAsrRequest {
                provider_id: provider_id.to_owned(),
                base_url,
                model: model.to_owned(),
                credentials: credentials
                    .iter()
                    .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
                    .collect(),
                audio_path: audio_path.clone(),
                language: "zh".to_owned(),
                duration_ms: 2_000,
            },
            audio_path,
        )
    }

    #[test]
    fn missing_audio_consent_blocks_transport_before_it_runs() {
        let (library, root) = temp_library();
        let repository = library.repository();
        let mut settings = repository.external_ai_settings(false, false).unwrap();
        settings.transcription_mode = "external".to_owned();
        settings.transcription_provider = "zhipu".to_owned();
        settings.transcription_base_url = "https://api.example.com/v1".to_owned();
        settings.transcription_model = "glm-asr-2512".to_owned();
        repository
            .update_external_ai_settings(&settings, false, false)
            .unwrap();
        let calls = Cell::new(0usize);
        let error =
            transcribe_with_transport(&library, &root.join("audio.wav"), "zh", 1_000, |_| {
                calls.set(calls.get() + 1);
                Ok(ExternalAsrResult {
                    segments: Vec::new(),
                    preprocessing_json: "{}".to_owned(),
                })
            })
            .unwrap_err();
        assert!(error.to_string().contains("音频上传说明"), "{error}");
        assert_eq!(calls.get(), 0);
        drop(library);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn consented_transport_runs_after_credential_lookup() {
        let token = ["test", "asr", "token"].join("-");
        let _key =
            AsrApiKeyEnvGuard::set(&serde_json::json!({"apiKey": token}).to_string());
        let (library, root) = temp_library();
        let repository = library.repository();
        let mut settings = repository.external_ai_settings(false, false).unwrap();
        settings.transcription_mode = "external".to_owned();
        settings.transcription_provider = "zhipu".to_owned();
        settings.transcription_base_url = "https://api.example.com/v1".to_owned();
        settings.transcription_model = "glm-asr-2512".to_owned();
        repository
            .update_external_ai_settings(&settings, false, true)
            .unwrap();
        let mut settings = repository.external_ai_settings(false, true).unwrap();
        settings.audio_upload_consent_at = Some("2026-09-25T00:00:00Z".to_owned());
        repository
            .update_external_ai_settings(&settings, false, true)
            .unwrap();
        let calls = Cell::new(0usize);
        let result =
            transcribe_with_transport(&library, &root.join("audio.wav"), "zh", 1_000, |request| {
                calls.set(calls.get() + 1);
                assert_eq!(request.provider_id, "zhipu");
                assert_eq!(request.credentials["apiKey"], token);
                Ok(ExternalAsrResult {
                    segments: vec![TranscriptSegmentInput {
                        start_ms: 0,
                        end_ms: 1_000,
                        speaker_label: Some("未知".to_owned()),
                        original_text: "测试".to_owned(),
                    }],
                    preprocessing_json: "{}".to_owned(),
                })
            })
            .unwrap();
        assert_eq!(calls.get(), 1);
        assert_eq!(result.segments.len(), 1);
        drop(library);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn legacy_single_key_is_read_as_api_key_credentials() {
        assert_eq!(parse_credentials("sk-legacy")["apiKey"], "sk-legacy");
        let parsed = parse_credentials(r#"{"appId":"1","secretId":"2","secretKey":"3"}"#);
        assert_eq!(parsed["secretId"], "2");
    }

    #[test]
    fn plain_text_response_is_split_into_timestamped_segments() {
        let inputs =
            parse_openai_response(r#"{"text":"第一句话。第二句话。第三句话。"}"#, 3_000)
                .unwrap();
        assert!(inputs.len() >= 2);
        assert!(inputs
            .windows(2)
            .all(|pair| pair[0].end_ms <= pair[1].start_ms));
        assert_eq!(inputs.last().unwrap().end_ms, 3_000);
    }

    #[test]
    fn native_speaker_fields_are_parsed_without_guessing() {
        let body = r#"{
            "segments": [
                {"text": "A", "speaker": "SPEAKER_00"},
                {"text": "B", "speaker_label": "SPEAKER_01"},
                {"text": "C", "speaker_id": 2},
                {"text": "D", "person": "张三"},
                {"text": "E"}
            ]
        }"#;
        let inputs = parse_openai_response(body, 5_000).unwrap();
        assert_eq!(
            inputs
                .iter()
                .map(|input| input.speaker_label.as_deref())
                .collect::<Vec<_>>(),
            vec![
                Some("说话人 1"),
                Some("说话人 2"),
                Some("2"),
                Some("张三"),
                Some("未知")
            ]
        );
    }

    #[test]
    fn openai_multipart_posts_audio_and_metadata() {
        let (base_url, server) = serve_one_response("200 OK", &[], r#"{"text":"本地转写结果。"}"#);
        let (request, audio_path) = request(
            "openai",
            format!("{base_url}/v1"),
            "whisper-1",
            &[("apiKey", "test-asr-token")],
        );
        let segments = send_transcription_request(&request).unwrap();
        assert_eq!(segments[0].original_text, "本地转写结果。");
        let received = server.join().unwrap();
        assert!(received.starts_with("POST /v1/audio/transcriptions HTTP/1.1"));
        let lower = received.to_ascii_lowercase();
        assert!(lower.contains("authorization: bearer test-asr-token"));
        assert!(received.contains("whisper-1"));
        assert!(received.contains("AUDIO-BYTES"));
        let _ = std::fs::remove_file(audio_path);
    }

    #[test]
    fn dashscope_uses_chat_audio_data_uri() {
        let response = r#"{"choices":[{"message":{"content":"阿里转写。"}}]}"#;
        let (base_url, server) = serve_one_response("200 OK", &[], response);
        let (request, audio_path) = request(
            "dashscope",
            format!("{base_url}/compatible-mode/v1"),
            "qwen3-asr-flash",
            &[("apiKey", "dash-token")],
        );
        let segments = send_transcription_request(&request).unwrap();
        assert_eq!(segments[0].original_text, "阿里转写。");
        let received = server.join().unwrap();
        assert!(received.starts_with("POST /compatible-mode/v1/chat/completions"));
        assert!(received.contains(r#""type":"input_audio""#));
        assert!(received.contains(r#""data":"data:audio/wav;base64,"#));
        let _ = std::fs::remove_file(audio_path);
    }

    #[test]
    fn tencent_query_is_sorted_and_signature_is_hmac_sha1_base64() {
        let (request, audio_path) = request(
            "tencent",
            "https://asr.cloud.tencent.com/asr/flash/v1".to_owned(),
            "16k_zh_en",
            &[("appId", "125"), ("secretId", "sid"), ("secretKey", "skey")],
        );
        let (endpoint, signature) =
            tencent_signed_endpoint("125", "sid", "skey", 1_700_000_000, &request);
        assert!(endpoint.starts_with("https://asr.cloud.tencent.com/asr/flash/v1/125?"));
        assert!(endpoint.contains("secretid=sid"));
        assert!(endpoint.contains("timestamp=1700000000"));
        assert!(signature.len() > 16);
        let _ = std::fs::remove_file(audio_path);
    }

    #[test]
    fn tencent_response_uses_timestamps_and_speakers() {
        let body = r#"{"code":0,"message":"","audio_duration":2000,"flash_result":[{"sentence_list":[{"text":"腾讯结果","start_time":10,"end_time":900,"speaker_id":2}]}]}"#;
        let segments = parse_tencent_response(body, 2_000, &BTreeMap::new()).unwrap();
        assert_eq!(segments[0].speaker_label.as_deref(), Some("2"));
        assert_eq!(segments[0].start_ms, 10);
    }

    #[test]
    fn doubao_sends_required_headers_and_parses_utterances() {
        let token = ["doubao", "token"].join("-");
        let body = r#"{"audio_info":{"duration":2000},"result":{"text":"豆包结果","utterances":[{"text":"豆包结果","start_time":100,"end_time":800,"additions":{"speaker":"1"}}]}}"#;
        let (base_url, server) = serve_one_response(
            "200 OK",
            &[("X-Api-Status-Code", "20000000"), ("X-Api-Message", "OK")],
            body,
        );
        let (request, audio_path) = request(
            "doubao",
            base_url,
            "volc.bigasr.auc_turbo",
            &[("apiKey", token.as_str())],
        );
        let segments = send_transcription_request(&request).unwrap();
        assert_eq!(segments[0].original_text, "豆包结果");
        assert_eq!(segments[0].speaker_label.as_deref(), Some("1"));
        let received = server.join().unwrap();
        let lower = received.to_ascii_lowercase();
        assert!(lower.contains(&format!("x-api-key: {token}")));
        assert!(lower.contains("x-api-resource-id: volc.bigasr.auc_turbo"));
        assert!(received.contains(r#""enable_speaker_info":true"#));
        assert!(received.contains(r#""show_utterances":true"#));
        assert!(received.contains(r#""format":"wav""#));
        assert!(received.contains(r#""data":"QVVESU8tQllURVM=""#));
        let request_body = received.split("\r\n\r\n").nth(1).unwrap_or_default();
        assert!(!request_body.contains(&token), "API Key 不应进入请求体");
        assert!(request_body.contains(r#""uid":"echo-"#));
        let _ = std::fs::remove_file(audio_path);
    }

    #[test]
    fn short_audio_stays_one_provider_request() {
        let root = std::env::temp_dir().join(format!("asr-short-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let audio_path = root.join("audio.wav");
        write_test_wav(&audio_path, 2);
        let (base_url, server) =
            serve_responses(vec![("200 OK".to_owned(), r#"{"text":"一次完成。"}"#.to_owned())], &[]);
        let request = ExternalAsrRequest {
            provider_id: "zhipu".to_owned(),
            base_url,
            model: "glm-asr-2512".to_owned(),
            credentials: BTreeMap::from([("apiKey".to_owned(), "token".to_owned())]),
            audio_path,
            language: "zh".to_owned(),
            duration_ms: 2_000,
        };
        let segments =
            transcribe_long_audio(&request, &root.join("cache"), |_, _| Ok(())).unwrap();
        assert_eq!(segments[0].original_text, "一次完成。");
        assert_eq!(server.join().unwrap().len(), 1);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn long_audio_retry_reuses_completed_segment_cache() {
        let root = std::env::temp_dir().join(format!("asr-resume-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let audio_path = root.join("audio.wav");
        write_test_wav(&audio_path, 31);
        let cache_dir = root.join("cache");
        let (first_base_url, first_server) = serve_responses(
            vec![
                ("200 OK".to_owned(), r#"{"text":"第一段。"}"#.to_owned()),
                (
                    "500 Internal Server Error".to_owned(),
                    r#"{"message":"second failed"}"#.to_owned(),
                ),
            ],
            &[],
        );
        let mut request = ExternalAsrRequest {
            provider_id: "zhipu".to_owned(),
            base_url: first_base_url,
            model: "glm-asr-2512".to_owned(),
            credentials: BTreeMap::from([("apiKey".to_owned(), "token".to_owned())]),
            audio_path,
            language: "zh".to_owned(),
            duration_ms: 31_000,
        };
        let mut progress = Vec::new();
        let error = transcribe_long_audio(&request, &cache_dir, |current, total| {
            progress.push((current, total));
            Ok(())
        })
        .unwrap_err();
        assert!(error.to_string().contains("第 2/2 段"), "{error}");
        assert_eq!(progress.last(), Some(&(1, 2)));
        assert_eq!(first_server.join().unwrap().len(), 2);

        let namespace = std::fs::read_dir(&cache_dir)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let cached = std::fs::read_dir(namespace)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.path().extension().is_some_and(|value| value == "json"))
            .count();
        assert_eq!(cached, 1);

        let (second_base_url, second_server) =
            serve_responses(vec![("200 OK".to_owned(), r#"{"text":"第二段。"}"#.to_owned())], &[]);
        request.base_url = second_base_url;
        let segments =
            transcribe_long_audio(&request, &cache_dir, |_, _| Ok(())).unwrap();
        assert_eq!(second_server.join().unwrap().len(), 1, "只应补发缺失分段");
        let text = segments
            .iter()
            .map(|segment| segment.original_text.as_str())
            .collect::<Vec<_>>()
            .join("");
        assert!(text.contains("第一段。"), "{text}");
        assert!(text.contains("第二段。"), "{text}");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn probe_treats_valid_empty_transcript_as_connected() {
        let token = ["probe", "token"].join("-");
        let _key = AsrApiKeyEnvGuard::set(&serde_json::json!({"apiKey": token}).to_string());
        let (library, root) = temp_library();
        let repository = library.repository();
        let mut settings = repository.external_ai_settings(false, false).unwrap();
        settings.transcription_mode = "external".to_owned();
        settings.transcription_provider = "custom".to_owned();
        settings.transcription_model = "whisper-test".to_owned();
        let (base_url, server) =
            serve_responses(vec![("200 OK".to_owned(), r#"{"text":""}"#.to_owned())], &[]);
        settings.transcription_base_url = base_url;
        repository
            .update_external_ai_settings(&settings, false, true)
            .unwrap();
        let mut settings = repository.external_ai_settings(false, true).unwrap();
        settings.audio_upload_consent_at = Some("2026-09-25T00:00:00Z".to_owned());
        repository
            .update_external_ai_settings(&settings, false, true)
            .unwrap();

        probe_external_asr_connection(&library).unwrap();
        assert_eq!(server.join().unwrap().len(), 1);
        drop(library);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn overlapping_timestamped_chunks_merge_without_repeating_the_joint() {
        let mut merged = Vec::new();
        let first = AudioChunk {
            sample_start: 0,
            sample_end: 32_000,
            accept_start_ms: 0,
            accept_end_ms: 2_000,
        };
        merge_chunk_segments(
            &mut merged,
            vec![TranscriptSegmentInput {
                start_ms: 0,
                end_ms: 1_900,
                speaker_label: Some("未知".to_owned()),
                original_text: "我们讨论下一步计划".to_owned(),
            }],
            &first,
            true,
            5_000,
        );
        let second = AudioChunk {
            sample_start: 24_000,
            sample_end: 48_000,
            accept_start_ms: 2_000,
            accept_end_ms: 5_000,
        };
        merge_chunk_segments(
            &mut merged,
            vec![TranscriptSegmentInput {
                start_ms: 0,
                end_ms: 1_300,
                speaker_label: Some("未知".to_owned()),
                original_text: "下一步计划并确认负责人".to_owned(),
            }],
            &second,
            true,
            5_000,
        );
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].text, "我们讨论下一步计划并确认负责人");
    }

    #[test]
    fn provider_errors_redact_credentials() {
        let secret = ["sk", "secret"].join("-");
        let body = format!(r#"{{"error":{{"message":"bad {secret}"}}}}"#);
        let (base_url, server) = serve_one_response("401 Unauthorized", &[], &body);
        let (request, audio_path) = request(
            "openai",
            format!("{base_url}/v1"),
            "whisper-1",
            &[("apiKey", secret.as_str())],
        );
        let error = send_transcription_request(&request).unwrap_err().to_string();
        assert!(error.contains("***"), "{error}");
        assert!(!error.contains(&secret), "{error}");
        let _ = server.join();
        let _ = std::fs::remove_file(audio_path);
    }
}
