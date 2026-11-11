//! 第三方服务商连接诊断：结构化分类。
//!
//! 目标：用户测试失败时能明确回答「问题在凭据、网络、模型、权限还是输出格式」，
//! 而不是看到一句笼统的「失败」。分类规则：服务商返回的 error.code/error.type
//! 优先，HTTP 状态只作辅助信号；无法可靠分类时落到「模型或权限不可用」双动作。

use crate::library::ManagedLibrary;
use crate::provider_catalog;
use serde::Serialize;
use serde_json::json;
use std::collections::BTreeMap;
use std::io::Read;
use std::sync::mpsc;
use std::time::Duration;

/// 诊断阶段：失败发生在哪一层。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosisStage {
    /// /models 模型目录阶段。
    Catalog,
    /// 真实推理请求阶段（chat 或转写）。
    Inference,
    /// 模型不存在或账号无权使用。
    Model,
    /// 模型不支持该接口或参数。
    Protocol,
    /// 频率限制或额度不足。
    Quota,
    /// 传输层失败或超时。
    Network,
    /// 200 但响应无内容。
    Response,
    /// 200 有内容但不是合法 JSON。
    Format,
    /// 通过。
    Ok,
}

impl DiagnosisStage {
    /// 快照存储与前端一致的 snake_case 名称。
    pub fn as_str(&self) -> &'static str {
        match self {
            DiagnosisStage::Catalog => "catalog",
            DiagnosisStage::Inference => "inference",
            DiagnosisStage::Model => "model",
            DiagnosisStage::Protocol => "protocol",
            DiagnosisStage::Quota => "quota",
            DiagnosisStage::Network => "network",
            DiagnosisStage::Response => "response",
            DiagnosisStage::Format => "format",
            DiagnosisStage::Ok => "ok",
        }
    }
}

/// 建议动作：前端映射为中文文案与按钮。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SuggestedAction {
    /// 重填 Key / 查看厂商控制台。
    RefillKey,
    /// 换文本对话模型。
    SwitchModel,
    /// 检查账号开通、余额、权限、地域。
    CheckAccount,
    /// 检查网络/代理/地址。
    CheckNetwork,
    /// 等待后重试 / 检查额度。
    Retry,
    /// 兜底：同时给出「换模型」与「检查账号权限」两个动作。
    ChangeModelOrPermission,
}

/// 一次诊断的结构化结果。全部字段非敏感：服务商原文经脱敏与截断。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosisReport {
    pub target: String,
    pub stage: DiagnosisStage,
    pub status: String,
    pub model: String,
    pub http_status: Option<u16>,
    pub error_code: Option<String>,
    pub provider_message: Option<String>,
    pub tested_at: String,
    pub suggested_action: SuggestedAction,
    /// 仅在诊断通过时签发：一次性验证票据（save_and_verify 时校验）。
    pub verification_token: Option<String>,
}

/// 供分类器判定的失败信号（已从原始响应提取并脱敏）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProviderFailure {
    pub http_status: Option<u16>,
    /// 服务商 error.code / error.type，已脱敏截断。
    pub error_code: Option<String>,
    /// 传输层失败（连接失败、超时），未拿到 HTTP 响应。
    pub transport: bool,
}

/// 目录阶段（/models）失败分类。
pub fn classify_catalog_failure(failure: &ProviderFailure) -> (DiagnosisStage, SuggestedAction) {
    if failure.transport {
        return (DiagnosisStage::Network, SuggestedAction::CheckNetwork);
    }
    match failure.http_status {
        Some(401) | Some(403) => (DiagnosisStage::Catalog, SuggestedAction::RefillKey),
        _ => (DiagnosisStage::Catalog, SuggestedAction::CheckAccount),
    }
}

/// 推理阶段（chat/转写）失败分类。错误码优先于 HTTP 状态：
/// 不同厂商可能用 400/403/404 的任意组合表达 model_not_found，
/// 因此错误码明确时一律按模型问题归类；401/403 无可靠错误码时
/// 不得单独断言 Key 无效，落到「换模型 + 查权限」双动作。
pub fn classify_inference_failure(failure: &ProviderFailure) -> (DiagnosisStage, SuggestedAction) {
    if failure.transport {
        return (DiagnosisStage::Network, SuggestedAction::CheckNetwork);
    }
    if let Some(code) = failure.error_code.as_deref() {
        if is_model_unavailable_code(code) {
            return (DiagnosisStage::Model, SuggestedAction::SwitchModel);
        }
        if code.to_ascii_lowercase().contains("unsupported") {
            return (DiagnosisStage::Protocol, SuggestedAction::SwitchModel);
        }
    }
    match failure.http_status {
        Some(401) | Some(403) => (
            DiagnosisStage::Inference,
            SuggestedAction::ChangeModelOrPermission,
        ),
        Some(404) => (DiagnosisStage::Model, SuggestedAction::SwitchModel),
        Some(400) => (DiagnosisStage::Protocol, SuggestedAction::SwitchModel),
        Some(429) => (DiagnosisStage::Quota, SuggestedAction::Retry),
        _ => (DiagnosisStage::Inference, SuggestedAction::CheckAccount),
    }
}

/// 200 但内容异常时的分类：空内容 vs 非 JSON。
pub fn classify_success_anomaly(empty: bool) -> (DiagnosisStage, SuggestedAction) {
    if empty {
        (DiagnosisStage::Response, SuggestedAction::Retry)
    } else {
        (DiagnosisStage::Format, SuggestedAction::SwitchModel)
    }
}

/// 识别「模型不存在/无权使用/已下线」类错误码。
/// 覆盖常见写法：model_not_found / invalid_model / unknown model /
/// ModelNotExist 等，大小写去分隔符后匹配。
fn is_model_unavailable_code(code: &str) -> bool {
    let lower = code.to_ascii_lowercase();
    let normalized = lower.replace(['_', '-', ' '], "");
    normalized.contains("modelnotfound")
        || normalized.contains("invalidmodel")
        || normalized.contains("unknownmodel")
        || normalized.contains("modelnotexist")
        || normalized.contains("nomodel")
        || normalized.contains("modelnotopen")
}

// ---------------------------------------------------------------------------
// 草稿诊断探测：分析（chat）与转写（1 秒测试音）。全部不落盘、不写 Keychain。
// ---------------------------------------------------------------------------

/// 诊断请求输入。凭据为空表时回退当前槽位已保存凭据（要求服务商一致）。
pub struct DiagnoseInput {
    pub target: String,
    pub provider_id: String,
    pub base_url: String,
    pub model: String,
    pub credentials: BTreeMap<String, String>,
}

/// 诊断总超时：设置页探测不沿用正式任务的长重试与超时。
const PROBE_TIMEOUT_SECS: u64 = 30;
/// 响应体上限：诊断只需错误码与小响应，超长响应截断后参与分类。
const PROBE_MAX_BODY_BYTES: u64 = 64 * 1024;

/// 探测结果的结构化信号（从原始响应/错误中提取，已脱敏）。
pub struct ProbeOutcome {
    pub kind: ProbeKind,
    pub error_code: Option<String>,
    pub provider_message: Option<String>,
}

pub enum ProbeKind {
    /// 传输层失败或 30 秒探测超时。
    Transport,
    /// 收到 HTTP 响应但状态码非 2xx。
    Http(u16),
    /// 200 但响应信封缺少模型输出内容。
    EmptyContent,
    /// 200 有内容，但模型输出不是合法 JSON（分析探测）。
    NonJson,
    /// 通过。
    Ok,
}

/// 草稿诊断主入口（同步实现，测试直接调用；命令层用 spawn_blocking 包装）。
///
/// 凭据只在参数与内存中使用：不写数据库、不写 Keychain、不返回前端。
/// `library` 仅在凭据回退（空凭据表）时需要。
pub fn diagnose_external_provider_blocking(
    library: Option<&ManagedLibrary>,
    input: DiagnoseInput,
) -> Result<DiagnosisReport, String> {
    let capability_name = match input.target.as_str() {
        "analysis" => "analysis",
        "transcription" => "transcription",
        _ => return Err("诊断目标无效".to_owned()),
    };
    // 内置服务商锁定目录地址；custom 走用户填写的地址（长度校验含在 resolve 里）。
    let base_url = provider_catalog::resolve_base_url(
        capability_name,
        &input.provider_id,
        &input.base_url,
    )
    .map_err(|error| error.to_frontend())?;
    let capability = provider_catalog::capability(&input.provider_id, capability_name)
        .ok_or_else(|| "该服务商不支持所选能力".to_owned())?;
    let model = if input.model.trim().is_empty() {
        capability.recommended_model.clone()
    } else {
        input.model.trim().to_owned()
    };
    if model.is_empty() || model.len() > 160 {
        return Err("模型名称无效".to_owned());
    }

    let credentials = if input.credentials.is_empty() {
        let library = library.ok_or_else(|| "请先填写该服务商的凭据".to_owned())?;
        resolve_saved_credentials(library, capability_name, &input.provider_id)?
    } else {
        if !crate::external_ai_gate::credentials_are_complete(
            &input.provider_id,
            Some(&input.credentials),
        ) {
            return Err("请完整填写该服务商要求的凭据".to_owned());
        }
        input.credentials
    };

    let outcome = match capability_name {
        "analysis" => {
            let api_key = credentials
                .get("apiKey")
                .map(String::as_str)
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| "请先填写该服务商的凭据".to_owned())?;
            // custom 地址先过 SSRF 校验；内置地址是目录固定的 HTTPS 域名。
            crate::memory::validate_external_base_url(&base_url).map_err(|e| e.to_frontend())?;
            probe_analysis(&base_url, &model, api_key, &credentials)
        }
        "transcription" => probe_transcription(&input.provider_id, &base_url, &model, &credentials),
        _ => unreachable!("capability_name 已在上方收窄"),
    };

    let (stage, action, status, http_status, error_code, provider_message) = match outcome.kind {
        ProbeKind::Ok => (
            DiagnosisStage::Ok,
            SuggestedAction::Retry,
            "passed",
            None,
            None,
            None,
        ),
        ProbeKind::EmptyContent => {
            let (stage, action) = classify_success_anomaly(true);
            (stage, action, "failed", Some(200), None, outcome.provider_message)
        }
        ProbeKind::NonJson => {
            let (stage, action) = classify_success_anomaly(false);
            (stage, action, "failed", Some(200), None, outcome.provider_message)
        }
        ProbeKind::Http(status) => {
            let failure = ProviderFailure {
                http_status: Some(status),
                error_code: outcome.error_code.clone(),
                transport: false,
            };
            let (stage, action) = classify_inference_failure(&failure);
            (
                stage,
                action,
                "failed",
                Some(status),
                outcome.error_code,
                outcome.provider_message,
            )
        }
        ProbeKind::Transport => {
            let failure = ProviderFailure {
                http_status: None,
                error_code: None,
                transport: true,
            };
            let (stage, action) = classify_inference_failure(&failure);
            (stage, action, "failed", None, None, outcome.provider_message)
        }
    };

    Ok(DiagnosisReport {
        target: input.target,
        stage,
        status: status.to_owned(),
        model,
        http_status,
        error_code,
        provider_message,
        tested_at: chrono::Utc::now().to_rfc3339(),
        suggested_action: action,
        verification_token: None,
    })
}

/// 空凭据表时的回退：只允许回退到「已保存设置就是这家服务商」的槽位，
/// 绝不跨服务商取 Key。诊断与「保存并验证」共用同一解析语义，
/// 保证凭据摘要两次计算一致。
pub(crate) fn resolve_saved_credentials(
    library: &ManagedLibrary,
    capability_name: &str,
    provider_id: &str,
) -> Result<BTreeMap<String, String>, String> {
    let settings = library
        .repository()
        .external_ai_settings(false, false)
        .map_err(|error| error.to_frontend())?;
    let (saved_provider, credentials) = if capability_name == "analysis" {
        (
            settings.analysis_provider,
            crate::external_ai_gate::get_analysis_credentials()
                .map_err(|error| error.to_frontend())?,
        )
    } else {
        (
            settings.transcription_provider,
            crate::external_ai_gate::get_asr_credentials().map_err(|error| error.to_frontend())?,
        )
    };
    if saved_provider != provider_id {
        return Err("已保存配置属于其他服务商，请先填写新凭据".to_owned());
    }
    credentials
        .filter(|credentials| {
            crate::external_ai_gate::credentials_are_complete(provider_id, Some(credentials))
        })
        .ok_or_else(|| "请先填写该服务商的凭据".to_owned())
}

/// 分析探测：一次固定的小 chat 请求（提示词不含任何用户内容），无重试。
fn probe_analysis(
    base_url: &str,
    model: &str,
    api_key: &str,
    credentials: &BTreeMap<String, String>,
) -> ProbeOutcome {
    let endpoint = format!("{}/chat/completions", base_url.trim_end_matches('/'));
    // 请求前复校（防 DNS rebinding）——与正式 chat 路径同规。
    if let Err(error) = crate::memory::assert_endpoint_still_safe(&endpoint) {
        return ProbeOutcome {
            kind: ProbeKind::Transport,
            error_code: None,
            provider_message: Some(error.to_frontend()),
        };
    }
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout(Duration::from_secs(PROBE_TIMEOUT_SECS))
        .redirects(0)
        .build();
    let body = json!({
        "model": model,
        "messages": [
            {"role": "user", "content": PROBE_PROMPT}
        ],
        "stream": false,
    });
    let response = match agent
        .post(&endpoint)
        .set("Authorization", &format!("Bearer {}", api_key.trim()))
        .set("Content-Type", "application/json")
        .send_json(body)
    {
        Ok(response) => response,
        Err(ureq::Error::Status(code, response)) => {
            let body_text = read_limited_body(response, credentials);
            let (error_code, message) = parse_provider_error(&body_text, credentials);
            return ProbeOutcome {
                kind: ProbeKind::Http(code),
                error_code,
                provider_message: Some(message),
            };
        }
        Err(ureq::Error::Transport(_)) => {
            return ProbeOutcome {
                kind: ProbeKind::Transport,
                error_code: None,
                provider_message: Some("无法连接服务商，请检查地址与网络".to_owned()),
            };
        }
    };
    let body_text = read_limited_body(response, credentials);
    let value: serde_json::Value = match serde_json::from_str(&body_text) {
        Ok(value) => value,
        Err(_) => {
            return ProbeOutcome {
                kind: ProbeKind::EmptyContent,
                error_code: None,
                provider_message: Some("响应不是有效 JSON".to_owned()),
            };
        }
    };
    match extract_probe_content(&value).filter(|content| !content.trim().is_empty()) {
        None => ProbeOutcome {
            kind: ProbeKind::EmptyContent,
            error_code: None,
            provider_message: Some("服务商返回 200 但没有模型输出内容".to_owned()),
        },
        Some(content) => {
            // 固定提示词要求模型只返回 {"ok":true}：内容非空但结构不满足
            // 即「模型不满足结构化输出要求」。
            match crate::memory::parse_json_object(&content) {
                Ok(value) if value.get("ok").and_then(serde_json::Value::as_bool) == Some(true) => {
                    ProbeOutcome {
                        kind: ProbeKind::Ok,
                        error_code: None,
                        provider_message: None,
                    }
                }
                _ => ProbeOutcome {
                    kind: ProbeKind::NonJson,
                    error_code: None,
                    provider_message: Some("模型输出不是预期的 JSON 结构".to_owned()),
                },
            }
        }
    }
}

/// 转写探测：1 秒 440Hz 正弦测试音走真实协议上传。外层 30 秒超时，
/// 超时按网络类处理（内部协议实现的自有超时上限更长，探测线程有界自亡）。
///
/// 同意门禁豁免说明：正式转写必须先过 `require_external_asr_consent`
/// （音频上传同意门），本探测有意绕过它——探测载荷是合成的 1 秒正弦音，
/// 不含任何用户音频或文本，且只发生在用户显式点击「保存并验证/验证连通」
/// 时；若要求先勾同意才能测试，用户将无法在同意前确认配置可用，这与
/// 「草稿诊断不落盘、可先验证」的产品语义冲突。
fn probe_transcription(
    provider_id: &str,
    base_url: &str,
    model: &str,
    credentials: &BTreeMap<String, String>,
) -> ProbeOutcome {
    let probe_path =
        std::env::temp_dir().join(format!("echo-asr-diagnose-{}.wav", uuid::Uuid::new_v4()));
    let samples: Vec<f32> = (0..16_000)
        .map(|index| {
            let phase = index as f32 * 440.0 * std::f32::consts::TAU / 16_000.0;
            phase.sin() * 0.05
        })
        .collect();
    if let Err(error) = crate::external_asr::write_wav_chunk(&probe_path, &samples) {
        return ProbeOutcome {
            kind: ProbeKind::Transport,
            error_code: None,
            provider_message: Some(error.to_frontend()),
        };
    }
    let request = crate::external_asr::ExternalAsrRequest {
        provider_id: provider_id.to_owned(),
        base_url: base_url.to_owned(),
        model: model.to_owned(),
        credentials: credentials.clone(),
        audio_path: probe_path.clone(),
        language: "zh".to_owned(),
        duration_ms: 1_000,
    };
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let result = crate::external_asr::send_transcription_request(&request);
        let _ = std::fs::remove_file(&request.audio_path);
        let _ = sender.send(result);
    });
    let result = match receiver.recv_timeout(Duration::from_secs(PROBE_TIMEOUT_SECS)) {
        Ok(result) => result,
        Err(_) => {
            let _ = std::fs::remove_file(&probe_path);
            return ProbeOutcome {
                kind: ProbeKind::Transport,
                error_code: None,
                provider_message: Some("转写探测超时，请检查地址与网络".to_owned()),
            };
        }
    };
    match result {
        Ok(_) => ProbeOutcome {
            kind: ProbeKind::Ok,
            error_code: None,
            provider_message: None,
        },
        Err(error) => {
            let message = error.to_frontend();
            let (kind, provider_message) = parse_transcription_error(&message);
            ProbeOutcome {
                kind,
                error_code: None,
                provider_message: Some(provider_message),
            }
        }
    }
}

/// 转写协议实现的错误是字符串形态（HTTP 状态嵌在文案里，且已脱敏）。
/// 在同一仓库内按稳定格式解析；格式变化时由测试锁定。
fn parse_transcription_error(message: &str) -> (ProbeKind, String) {
    if message.contains("无法连接第三方转写服务") || message.contains("超时") {
        return (ProbeKind::Transport, message.to_owned());
    }
    if let Some(start) = message.find("（HTTP ") {
        let rest = &message[start + "（HTTP ".len()..];
        if let Some(end) = rest.find('）') {
            if let Ok(status) = rest[..end].parse::<u16>() {
                let detail = rest[end + "）".len()..].trim();
                let detail = detail.strip_prefix('：').unwrap_or(detail);
                return (ProbeKind::Http(status), detail.to_owned());
            }
        }
    }
    // 本地参数/协议错误（不应到达，兜底给一个非传输、非具体状态的信号）。
    (ProbeKind::Http(0), message.to_owned())
}

/// 从服务商错误体提取 error.code / error.type（腾讯云取 Response.Error.Code），
/// 全部经凭据脱敏。
fn parse_provider_error(
    body: &str,
    credentials: &BTreeMap<String, String>,
) -> (Option<String>, String) {
    let value: serde_json::Value = serde_json::from_str(body).unwrap_or(serde_json::Value::Null);
    let code = ["/error/code", "/error/type", "/Response/Error/Code"]
        .iter()
        .find_map(|pointer| {
            value
                .pointer(pointer)
                .and_then(serde_json::Value::as_str)
                .map(|code| sanitize_snippet(code, credentials))
        });
    let message = crate::external_asr::sanitize_provider_message(body, credentials);
    (code, message)
}

fn sanitize_snippet(text: &str, credentials: &BTreeMap<String, String>) -> String {
    let mut snippet: String = text.chars().take(120).collect();
    for secret in credentials.values() {
        if !secret.is_empty() {
            snippet = snippet.replace(secret, "***");
        }
    }
    snippet
}

/// 读取响应体并限制大小（诊断只关心错误码与小响应）。
fn read_limited_body(response: ureq::Response, credentials: &BTreeMap<String, String>) -> String {
    let mut reader = response.into_reader().take(PROBE_MAX_BODY_BYTES);
    let mut buffer = Vec::new();
    let _ = reader.read_to_end(&mut buffer);
    let raw = String::from_utf8_lossy(&buffer).to_string();
    sanitize_snippet(&raw, credentials)
}

/// 从 OpenAI 兼容 chat 响应信封里取模型输出文本（简化版，只为探测服务）。
fn extract_probe_content(value: &serde_json::Value) -> Option<String> {
    let choice = value
        .get("choices")
        .and_then(serde_json::Value::as_array)
        .and_then(|choices| choices.first())?;
    choice
        .pointer("/message/content")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .or_else(|| {
            choice
                .get("text")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        })
}

const PROBE_PROMPT: &str = "Return exactly this JSON object: {\"ok\":true}";

#[cfg(test)]
mod tests {
    use super::*;

    fn failure(status: Option<u16>, code: Option<&str>) -> ProviderFailure {
        ProviderFailure {
            http_status: status,
            error_code: code.map(str::to_owned),
            transport: false,
        }
    }

    // ---- 目录阶段（/models）----

    #[test]
    fn catalog_transport_failure_means_network() {
        let f = ProviderFailure {
            http_status: None,
            error_code: None,
            transport: true,
        };
        assert_eq!(
            classify_catalog_failure(&f),
            (DiagnosisStage::Network, SuggestedAction::CheckNetwork)
        );
    }

    #[test]
    fn catalog_unauthorized_means_refill_key() {
        assert_eq!(
            classify_catalog_failure(&failure(Some(401), None)),
            (DiagnosisStage::Catalog, SuggestedAction::RefillKey)
        );
        assert_eq!(
            classify_catalog_failure(&failure(Some(403), None)),
            (DiagnosisStage::Catalog, SuggestedAction::RefillKey)
        );
    }

    #[test]
    fn catalog_other_status_means_check_account() {
        assert_eq!(
            classify_catalog_failure(&failure(Some(500), None)),
            (DiagnosisStage::Catalog, SuggestedAction::CheckAccount)
        );
    }

    // ---- 推理阶段（chat/转写）：错误码优先 ----

    #[test]
    fn inference_model_not_found_code_wins_over_status() {
        // 服务商可能用 400/403/404 任意组合表达 model_not_found，
        // 错误码明确时一律归「模型不可用」。
        for status in [400u16, 403, 404] {
            assert_eq!(
                classify_inference_failure(&failure(Some(status), Some("model_not_found"))),
                (DiagnosisStage::Model, SuggestedAction::SwitchModel),
                "HTTP {status} + model_not_found 应归为模型问题"
            );
        }
    }

    #[test]
    fn inference_unknown_model_code_variants_are_recognized() {
        for code in ["invalid_model", "unknown model", "ModelNotExist"] {
            assert_eq!(
                classify_inference_failure(&failure(Some(400), Some(code))),
                (DiagnosisStage::Model, SuggestedAction::SwitchModel),
                "错误码 {code} 应识别为模型不可用"
            );
        }
    }

    #[test]
    fn inference_unauthorized_without_code_needs_dual_action() {
        // 401/403 无法可靠区分「Key 无效」还是「无推理权限」：
        // 不得单独断言 Key 无效，给双动作。
        assert_eq!(
            classify_inference_failure(&failure(Some(401), None)),
            (
                DiagnosisStage::Inference,
                SuggestedAction::ChangeModelOrPermission
            )
        );
        assert_eq!(
            classify_inference_failure(&failure(Some(403), None)),
            (
                DiagnosisStage::Inference,
                SuggestedAction::ChangeModelOrPermission
            )
        );
    }

    #[test]
    fn inference_404_without_code_means_model_unavailable() {
        assert_eq!(
            classify_inference_failure(&failure(Some(404), None)),
            (DiagnosisStage::Model, SuggestedAction::SwitchModel)
        );
    }

    #[test]
    fn inference_400_unsupported_means_protocol() {
        assert_eq!(
            classify_inference_failure(&failure(Some(400), Some("unsupported_model"))),
            (DiagnosisStage::Protocol, SuggestedAction::SwitchModel)
        );
        assert_eq!(
            classify_inference_failure(&failure(Some(400), None)),
            (DiagnosisStage::Protocol, SuggestedAction::SwitchModel)
        );
    }

    #[test]
    fn inference_429_means_quota_retry() {
        assert_eq!(
            classify_inference_failure(&failure(Some(429), None)),
            (DiagnosisStage::Quota, SuggestedAction::Retry)
        );
    }

    #[test]
    fn inference_transport_failure_means_network() {
        let f = ProviderFailure {
            http_status: None,
            error_code: None,
            transport: true,
        };
        assert_eq!(
            classify_inference_failure(&f),
            (DiagnosisStage::Network, SuggestedAction::CheckNetwork)
        );
    }

    #[test]
    fn inference_unexpected_status_means_check_account() {
        assert_eq!(
            classify_inference_failure(&failure(Some(500), None)),
            (DiagnosisStage::Inference, SuggestedAction::CheckAccount)
        );
    }

    // ---- 200 但内容异常 ----

    #[test]
    fn success_with_empty_content_means_response_anomaly() {
        assert_eq!(
            classify_success_anomaly(true),
            (DiagnosisStage::Response, SuggestedAction::Retry)
        );
    }

    #[test]
    fn success_with_non_json_content_means_format() {
        assert_eq!(
            classify_success_anomaly(false),
            (DiagnosisStage::Format, SuggestedAction::SwitchModel)
        );
    }

    // ---- 草稿诊断端到端（本地 fixture HTTP server，不访问真实服务商）----

    use std::io::Write as _;
    use std::net::TcpListener;

    /// 起一个只服务一次响应的本地 server（loopback HTTP，环回例外允许）。
    /// 必须把请求体读完再响应：转写探测的 multipart 体约 32KB，
    /// 提前 close 会让客户端偶发连接重置（并发下表现为不稳定失败）。
    fn serve_once(status: u16, body: &str) -> String {
        let body = body.to_owned();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                let mut received: Vec<u8> = Vec::new();
                let mut chunk = [0_u8; 8_192];
                loop {
                    match stream.read(&mut chunk) {
                        Ok(0) | Err(_) => break,
                        Ok(read) => {
                            received.extend_from_slice(&chunk[..read]);
                            let Some(header_end) =
                                received.windows(4).position(|w| w == b"\r\n\r\n")
                            else {
                                continue;
                            };
                            let content_length = String::from_utf8_lossy(&received[..header_end])
                                .to_ascii_lowercase()
                                .lines()
                                .find_map(|line| line.strip_prefix("content-length:"))
                                .and_then(|value| value.trim().parse::<usize>().ok())
                                .unwrap_or(0);
                            if received.len() >= header_end + 4 + content_length {
                                break;
                            }
                        }
                    }
                }
                let response = format!(
                    "HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });
        format!("http://{address}")
    }

    /// 起一个接受连接但立刻断开的端口，模拟传输失败。
    fn serve_dead_port() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                drop(stream);
            }
        });
        format!("http://{address}")
    }

    fn chat_body(content: &str) -> String {
        serde_json::json!({
            "choices": [{"message": {"content": content}}]
        })
        .to_string()
    }

    fn diagnose_analysis(base_url: &str, model: &str) -> Result<DiagnosisReport, String> {
        diagnose_external_provider_blocking(
            None,
            DiagnoseInput {
                target: "analysis".to_owned(),
                provider_id: "custom".to_owned(),
                base_url: base_url.to_owned(),
                model: model.to_owned(),
                credentials: BTreeMap::from([("apiKey".to_owned(), "fixture-diagnosis-key".to_owned())]),
            },
        )
    }

    #[test]
    fn analysis_probe_passes_on_ok_json() {
        let base = serve_once(200, &chat_body("{\"ok\":true}"));
        let report = diagnose_analysis(&base, "test-model").unwrap();
        assert_eq!(report.status, "passed");
        assert_eq!(report.stage, DiagnosisStage::Ok);
        assert!(report.verification_token.is_none(), "票据由命令层签发");
    }

    #[test]
    fn analysis_probe_classifies_model_not_found_despite_http() {
        // 服务商用 400 + model_not_found 表达模型不可用：错误码优先于状态码。
        let base = serve_once(
            400,
            r#"{"error":{"code":"model_not_found","message":"模型不存在或已下线"}}"#,
        );
        let report = diagnose_analysis(&base, "missing-model").unwrap();
        assert_eq!(report.status, "failed");
        assert_eq!(report.stage, DiagnosisStage::Model);
        assert_eq!(report.suggested_action, SuggestedAction::SwitchModel);
        assert_eq!(report.http_status, Some(400));
        assert_eq!(report.error_code.as_deref(), Some("model_not_found"));
    }

    #[test]
    fn analysis_probe_401_without_code_needs_dual_action() {
        let base = serve_once(401, r#"{"error":{"message":"拒绝访问"}}"#);
        let report = diagnose_analysis(&base, "test-model").unwrap();
        assert_eq!(report.status, "failed");
        assert_eq!(report.stage, DiagnosisStage::Inference);
        assert_eq!(
            report.suggested_action,
            SuggestedAction::ChangeModelOrPermission
        );
    }

    #[test]
    fn analysis_probe_429_means_quota() {
        let base = serve_once(429, r#"{"error":{"message":"请求过于频繁"}}"#);
        let report = diagnose_analysis(&base, "test-model").unwrap();
        assert_eq!(report.stage, DiagnosisStage::Quota);
        assert_eq!(report.suggested_action, SuggestedAction::Retry);
    }

    #[test]
    fn analysis_probe_flags_non_json_model_output_as_format() {
        let base = serve_once(200, &chat_body("抱歉，我不能返回 JSON。"));
        let report = diagnose_analysis(&base, "test-model").unwrap();
        assert_eq!(report.stage, DiagnosisStage::Format);
        assert_eq!(report.suggested_action, SuggestedAction::SwitchModel);
    }

    #[test]
    fn analysis_probe_flags_empty_content_as_response_anomaly() {
        let base = serve_once(200, &chat_body(""));
        let report = diagnose_analysis(&base, "test-model").unwrap();
        assert_eq!(report.stage, DiagnosisStage::Response);
        assert_eq!(report.suggested_action, SuggestedAction::Retry);
    }

    #[test]
    fn analysis_probe_transport_failure_means_network() {
        let base = serve_dead_port();
        let report = diagnose_analysis(&base, "test-model").unwrap();
        assert_eq!(report.stage, DiagnosisStage::Network);
        assert_eq!(report.suggested_action, SuggestedAction::CheckNetwork);
    }

    #[test]
    fn provider_error_body_is_sanitized_and_never_contains_the_key() {
        let base = serve_once(
            401,
            r#"{"error":{"message":"无效的 Key: fixture-diagnosis-key，请检查"}}"#,
        );
        let report = diagnose_analysis(&base, "test-model").unwrap();
        let message = report.provider_message.unwrap_or_default();
        assert!(!message.contains("fixture-diagnosis-key"), "错误正文泄漏了 Key: {message}");
        assert!(report.error_code.is_none());
    }

    #[test]
    fn built_in_provider_pins_catalog_base_url_even_if_request_says_otherwise() {
        // 内置服务商的地址由目录锁定：传入任何伪造地址都会被替换为目录
        // 固定 HTTPS 地址——诊断请求绝不可能被指到别处（SSRF 防线的第一道）。
        // 纯函数断言，不发任何网络请求。
        let pinned = provider_catalog::resolve_base_url(
            "analysis",
            "dashscope",
            "https://attacker.example/v1",
        )
        .unwrap();
        assert_eq!(
            pinned,
            "https://dashscope.aliyuncs.com/compatible-mode/v1",
            "伪造地址必须被目录锁定替换"
        );
        assert_ne!(pinned, "https://attacker.example/v1");
    }

    #[test]
    fn transcription_probe_passes_with_multipart_fixture() {
        let base = serve_once(200, r#"{"text":"测试"}"#);
        let report = diagnose_external_provider_blocking(
            None,
            DiagnoseInput {
                target: "transcription".to_owned(),
                provider_id: "custom".to_owned(),
                base_url: base.clone(),
                model: "whisper-1".to_owned(),
                credentials: BTreeMap::from([("apiKey".to_owned(), "fixture-diagnosis-key".to_owned())]),
            },
        )
        .unwrap();
        assert_eq!(report.status, "passed");
        assert_eq!(report.stage, DiagnosisStage::Ok);
        let _ = base;
    }

    #[test]
    fn transcription_probe_401_needs_dual_action_too() {
        let base = serve_once(401, r#"{"error":{"message":"拒绝访问"}}"#);
        let report = diagnose_external_provider_blocking(
            None,
            DiagnoseInput {
                target: "transcription".to_owned(),
                provider_id: "custom".to_owned(),
                base_url: base.clone(),
                model: "whisper-1".to_owned(),
                credentials: BTreeMap::from([("apiKey".to_owned(), "fixture-diagnosis-key".to_owned())]),
            },
        )
        .unwrap();
        assert_eq!(report.stage, DiagnosisStage::Inference);
        assert_eq!(
            report.suggested_action,
            SuggestedAction::ChangeModelOrPermission
        );
        let _ = base;
    }

    #[test]
    fn saved_credential_fallback_rejects_mismatched_provider() {
        let root = std::env::temp_dir().join(format!("echo-diagnosis-{}", uuid::Uuid::new_v4()));
        let library = crate::library::ManagedLibrary::open(root.clone()).unwrap();
        // 已保存设置停留在默认（无 provider）——回退必须拒绝而不是乱取 Key。
        let error = diagnose_external_provider_blocking(
            Some(&library),
            DiagnoseInput {
                target: "analysis".to_owned(),
                provider_id: "dashscope".to_owned(),
                base_url: String::new(),
                model: "qwen-turbo".to_owned(),
                credentials: BTreeMap::new(),
            },
        )
        .unwrap_err();
        assert!(error.contains("凭据"), "拒绝话术应引导填凭据：{error}");
        let _ = std::fs::remove_dir_all(root);
    }
}
