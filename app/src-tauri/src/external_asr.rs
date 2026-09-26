//! Privacy-gated third-party ASR using an OpenAI-compatible transcription endpoint.

use crate::error::{AppError, AppResult};
use crate::external_ai_gate::{get_asr_api_key, require_external_asr_consent};
use crate::library::ManagedLibrary;
use crate::memory::{assert_endpoint_still_safe, validate_external_base_url};
use crate::types::TranscriptSegmentInput;
use crate::whisper::localize_speaker_label;
use serde::Deserialize;
use serde_json::Value;
use std::fs::File;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct ExternalAsrRequest {
    pub base_url: String,
    pub model: String,
    pub api_key: String,
    pub audio_path: PathBuf,
    pub language: String,
    pub duration_ms: i64,
}

#[derive(Debug)]
pub struct ExternalAsrResult {
    pub segments: Vec<TranscriptSegmentInput>,
    pub preprocessing_json: String,
}

/// Runs a transport only after the audio-upload consent gate and ASR key lookup.
///
/// Keeping the gate here makes every caller share the same ordering: no file open,
/// preprocessing, or network construction can happen before consent is verified.
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
    let api_key = get_asr_api_key()?
        .ok_or_else(|| AppError::Invalid("请先配置第三方转写 API Key".to_owned()))?;
    let base_url = validate_external_base_url(&settings.transcription_base_url)?;
    if settings.transcription_model.trim().is_empty() {
        return Err(AppError::Invalid("第三方转写模型不能为空".to_owned()));
    }
    transport(ExternalAsrRequest {
        base_url,
        model: settings.transcription_model.trim().to_owned(),
        api_key,
        audio_path: audio_path.to_path_buf(),
        language: language.trim().to_owned(),
        duration_ms,
    })
}

/// Sends one multipart upload and converts either segmented or plain-text responses
/// into the repository's normalized transcript segments.
pub fn send_transcription_request(
    request: &ExternalAsrRequest,
) -> AppResult<Vec<TranscriptSegmentInput>> {
    if !request.audio_path.is_file() {
        return Err(AppError::Import("待转写音频文件不存在".to_owned()));
    }
    let endpoint = endpoint_for_base(&request.base_url)?;
    assert_endpoint_still_safe(&endpoint)?;
    let file = File::open(&request.audio_path)
        .map_err(|error| AppError::Import(format!("无法读取待转写音频：{error}")))?;
    let boundary = format!("----echo-asr-{}", uuid::Uuid::new_v4());
    let prefix = multipart_prefix(&boundary, &request.audio_path);
    let suffix = multipart_suffix(&boundary, request);
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
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout(Duration::from_secs(180))
        .redirects(0)
        .build();
    let response = agent
        .post(&endpoint)
        .set("Authorization", &format!("Bearer {}", request.api_key))
        .set(
            "Content-Type",
            &format!("multipart/form-data; boundary={boundary}"),
        )
        .set("Content-Length", &content_len.to_string())
        .send(body)
        .map_err(|error| match error {
            ureq::Error::Status(code, response) => AppError::ExternalAi(format!(
                "第三方转写服务返回错误（HTTP {}）：{}",
                code,
                safe_provider_message(response)
            )),
            ureq::Error::Transport(_) => {
                AppError::ExternalAi("无法连接第三方转写服务，请检查地址与网络".to_owned())
            }
        })?;
    let body = response
        .into_string()
        .map_err(|_| AppError::ExternalAi("第三方转写返回了无效响应".to_owned()))?;
    parse_transcription_response(&body, request.duration_ms)
}

fn endpoint_for_base(base_url: &str) -> AppResult<String> {
    let base = validate_external_base_url(base_url)?;
    Ok(if base.ends_with("/audio/transcriptions") {
        base
    } else {
        format!("{base}/audio/transcriptions")
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

fn multipart_suffix(boundary: &str, request: &ExternalAsrRequest) -> Vec<u8> {
    let mut value = format!(
        "\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"model\"\r\n\r\n{}\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"response_format\"\r\n\r\nverbose_json",
        request.model
    );
    if !request.language.is_empty() && request.language != "auto" {
        value.push_str(&format!(
            "\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"language\"\r\n\r\n{}",
            request.language
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

fn parse_transcription_response(
    body: &str,
    duration_ms: i64,
) -> AppResult<Vec<TranscriptSegmentInput>> {
    let response: TranscriptionResponse = serde_json::from_str(body)
        .map_err(|_| AppError::ExternalAi("第三方转写响应不是有效 JSON".to_owned()))?;
    if let Some(segments) = response.segments {
        let mut inputs = Vec::new();
        for segment in segments {
            let speaker_label = speaker_from_segment(&segment).unwrap_or_else(|| "未知".to_owned());
            let text = segment.text.unwrap_or_default().trim().to_owned();
            if text.is_empty() {
                continue;
            }
            let start = seconds_to_ms(segment.start.unwrap_or(0.0));
            let end = seconds_to_ms(segment.end.unwrap_or(0.0)).max(start + 1);
            inputs.push(TranscriptSegmentInput {
                start_ms: start,
                end_ms: end,
                speaker_label: Some(speaker_label),
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
    Ok(synthesize_segments(&text, duration_ms))
}

fn seconds_to_ms(seconds: f64) -> i64 {
    if !seconds.is_finite() || seconds < 0.0 {
        0
    } else {
        (seconds * 1000.0).round() as i64
    }
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

fn safe_provider_message(response: ureq::Response) -> String {
    let body = response
        .into_string()
        .unwrap_or_default()
        .chars()
        .take(500)
        .collect::<String>();
    let value: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
    value
        .pointer("/error/message")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| "服务拒绝了本次请求".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::external_ai_gate::AsrApiKeyEnvGuard;
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

    fn serve_one_response(response_body: &str) -> (String, thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let response_body = response_body.to_owned();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
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
                let headers = String::from_utf8_lossy(&request[..header_end]).to_ascii_lowercase();
                let content_length = headers
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length:"))
                    .and_then(|value| value.trim().parse::<usize>().ok())
                    .unwrap_or(0);
                if request.len() >= header_end + 4 + content_length {
                    break;
                }
            }
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response_body.len(),
                response_body
            )
            .unwrap();
            String::from_utf8_lossy(&request).into_owned()
        });
        (format!("http://{address}"), handle)
    }

    #[test]
    fn missing_audio_consent_blocks_transport_before_it_runs() {
        let (library, root) = temp_library();
        let repository = library.repository();
        let mut settings = repository.external_ai_settings(false, false).unwrap();
        settings.processing_mode = "external".to_owned();
        settings.transcription_base_url = "https://api.example.com/v1".to_owned();
        settings.transcription_model = "asr-model".to_owned();
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
    fn missing_asr_provider_blocks_transport_before_it_runs() {
        let (library, root) = temp_library();
        let repository = library.repository();
        let mut settings = repository.external_ai_settings(false, false).unwrap();
        settings.processing_mode = "external".to_owned();
        settings.transcription_provider = "none".to_owned();
        settings.transcription_base_url = "https://api.example.com/v1".to_owned();
        settings.transcription_model = "asr-model".to_owned();
        settings.audio_upload_consent_at = Some("2026-09-25T00:00:00Z".to_owned());
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

        assert!(error.to_string().contains("完整配置"), "{error}");
        assert_eq!(calls.get(), 0);
        drop(library);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn consented_transport_runs_after_key_lookup() {
        let _key = AsrApiKeyEnvGuard::set("asr-secret");
        let (library, root) = temp_library();
        let repository = library.repository();
        let mut settings = repository.external_ai_settings(false, false).unwrap();
        settings.processing_mode = "external".to_owned();
        settings.transcription_base_url = "https://api.example.com/v1".to_owned();
        settings.transcription_model = "asr-model".to_owned();
        settings.audio_upload_consent_at = Some("2026-09-25T00:00:00Z".to_owned());
        repository
            .update_external_ai_settings(&settings, false, true)
            .unwrap();
        let calls = Cell::new(0usize);
        let result =
            transcribe_with_transport(&library, &root.join("audio.wav"), "zh", 1_000, |request| {
                calls.set(calls.get() + 1);
                assert_eq!(request.model, "asr-model");
                assert_eq!(request.api_key, "asr-secret");
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
    fn plain_text_response_is_split_into_timestamped_segments() {
        let inputs =
            parse_transcription_response(r#"{"text":"第一句话。第二句话。第三句话。"}"#, 3_000)
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

        let inputs = parse_transcription_response(body, 5_000).unwrap();

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
        assert_eq!(inputs[0].original_text, "A");
    }

    #[test]
    fn transcription_request_posts_multipart_audio_and_metadata() {
        let (base_url, server) = serve_one_response(r#"{"text":"本地转写结果。"}"#);
        let audio_path = std::env::temp_dir().join(format!("asr-request-{}.wav", Uuid::new_v4()));
        std::fs::write(&audio_path, b"AUDIO-BYTES").unwrap();
        let request = ExternalAsrRequest {
            base_url: format!("{base_url}/v1"),
            model: "qwen3-asr-flash".to_owned(),
            api_key: "asr-secret".to_owned(),
            audio_path: audio_path.clone(),
            language: "zh".to_owned(),
            duration_ms: 2_000,
        };

        let segments = send_transcription_request(&request).unwrap();

        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].original_text, "本地转写结果。");
        let received = server.join().unwrap();
        assert!(
            received.starts_with("POST /v1/audio/transcriptions HTTP/1.1"),
            "{received}"
        );
        let received_lower = received.to_ascii_lowercase();
        assert!(received_lower.contains("authorization: bearer asr-secret"));
        assert!(received_lower.contains("content-type: multipart/form-data; boundary="));
        assert!(received.contains(r#"name="model""#));
        assert!(received.contains("qwen3-asr-flash"));
        assert!(received.contains(r#"name="response_format""#));
        assert!(received.contains("verbose_json"));
        assert!(received.contains(r#"name="language""#));
        assert!(received.contains("AUDIO-BYTES"));
        let _ = std::fs::remove_file(audio_path);
    }
}
