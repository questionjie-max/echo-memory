//! Third-party provider catalog.
//!
//! This is the single source of truth for endpoint, credentials, protocol,
//! recommended model and input limits. The frontend renders these records
//! instead of carrying a second provider list.

use crate::error::{AppError, AppResult};
use serde::Serialize;

pub const CUSTOM_PROVIDER_ID: &str = "custom";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderCatalog {
    pub version: u32,
    pub providers: Vec<ProviderDefinition>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderDefinition {
    pub id: String,
    pub label: String,
    pub description: String,
    pub capabilities: Vec<String>,
    pub credential_fields: Vec<CredentialFieldDefinition>,
    pub key_url: Option<String>,
    pub analysis: Option<ProviderCapability>,
    pub transcription: Option<ProviderCapability>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CredentialFieldDefinition {
    pub key: String,
    pub label: String,
    pub placeholder: String,
    pub secret: bool,
    pub help: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderCapability {
    pub base_url: String,
    pub recommended_model: String,
    pub models: Vec<String>,
    pub supports_model_list: bool,
    pub protocol: String,
    pub protocol_hint: String,
    pub max_duration_ms: Option<i64>,
    pub max_file_bytes: Option<u64>,
    pub returns_timestamps: bool,
    pub returns_speakers: bool,
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedProviderConfig {
    pub provider_id: String,
    pub base_url: String,
    pub model: String,
}

pub fn provider_catalog() -> ProviderCatalog {
    ProviderCatalog {
        version: 1,
        providers: vec![
            provider(
                "zhipu",
                "智谱 AI",
                "一个 API Key 可同时用于转写与分析。",
                &["transcription", "analysis"],
                single_key_fields("智谱 API Key", "填写智谱开放平台的 API Key"),
                Some("https://open.bigmodel.cn/usercenter/apikeys"),
                Some(analysis(
                    "https://open.bigmodel.cn/api/paas/v4",
                    "glm-4-flash",
                    &["glm-4-flash", "glm-4-plus"],
                    "OpenAI 兼容",
                )),
                Some(transcription(
                    "https://open.bigmodel.cn/api/paas/v4",
                    "glm-asr-2512",
                    &["glm-asr-2512"],
                    "zhipu-multipart",
                    "multipart 上传，通常只返回全文。",
                    30_000,
                    25 * 1024 * 1024,
                    false,
                    false,
                    Some("单次上限约 30 秒，长录音会自动切块，请求数量会明显增加。"),
                ).with_model_list()),
            ),
            provider(
                "dashscope",
                "阿里云百炼",
                "通义千问转写与文本分析共用百炼 API Key。",
                &["transcription", "analysis"],
                single_key_fields("百炼 API Key", "填写阿里云百炼 API Key"),
                Some("https://bailian.console.aliyun.com/?tab=model#/api-key"),
                Some(analysis(
                    "https://dashscope.aliyuncs.com/compatible-mode/v1",
                    "qwen-turbo",
                    &["qwen-turbo", "qwen-plus", "qwen-max"],
                    "OpenAI 兼容",
                )),
                Some(transcription(
                    "https://dashscope.aliyuncs.com/compatible-mode/v1",
                    "qwen3-asr-flash",
                    &["qwen3-asr-flash"],
                    "dashscope-chat-audio",
                    "通过 chat/completions 的 input_audio 发送本地音频。",
                    300_000,
                    7 * 1024 * 1024,
                    false,
                    false,
                    Some("单段音频编码后上限约 10MB，长录音会自动切块。"),
                ).with_model_list()),
            ),
            provider(
                "openai",
                "OpenAI",
                "官方转写与文本分析接口。",
                &["transcription", "analysis"],
                single_key_fields("OpenAI API Key", "sk-…（仅保存，不回显）"),
                Some("https://platform.openai.com/api-keys"),
                Some(analysis(
                    "https://api.openai.com/v1",
                    "gpt-4o-mini",
                    &["gpt-4o-mini", "gpt-4.1-mini"],
                    "OpenAI 兼容",
                )),
                Some(transcription(
                    "https://api.openai.com/v1",
                    "whisper-1",
                    &["whisper-1", "gpt-4o-mini-transcribe"],
                    "openai-multipart",
                    "OpenAI 音频转写 multipart。",
                    1_200_000,
                    24 * 1024 * 1024,
                    true,
                    false,
                    Some("长录音会按大小与时长自动切块。"),
                ).with_model_list()),
            ),
            provider(
                "tencent",
                "腾讯云语音识别",
                "录音文件识别极速版，需要 AppID 与一对云 API 密钥。",
                &["transcription"],
                vec![
                    credential(
                        "appId",
                        "AppID",
                        "腾讯云账号 AppID",
                        false,
                        Some("数字 AppID，用于请求路径。"),
                    ),
                    credential(
                        "secretId",
                        "SecretId",
                        "云 API SecretId",
                        true,
                        None,
                    ),
                    credential(
                        "secretKey",
                        "SecretKey",
                        "云 API SecretKey",
                        true,
                        None,
                    ),
                ],
                Some("https://console.cloud.tencent.com/cam/capi"),
                None,
                Some(transcription(
                    "https://asr.cloud.tencent.com/asr/flash/v1",
                    "16k_zh_en",
                    &["16k_zh_en", "16k_zh", "16k_multi_lang"],
                    "tencent-flash",
                    "录音文件识别极速版，HMAC-SHA1 签名后上传原始音频。",
                    7_200_000,
                    100 * 1024 * 1024,
                    true,
                    true,
                    Some("单段上限约 2 小时 / 100MB；返回时间戳与说话人时直接采用。"),
                )),
            ),
            provider(
                "doubao",
                "豆包语音",
                "BigModel 录音文件识别极速版，只填一个 API Key。",
                &["transcription"],
                single_key_fields("豆包语音 API Key", "填写火山引擎语音控制台 API Key"),
                Some("https://console.volcengine.com/speech/new/setting/apikeys"),
                None,
                Some(transcription(
                    "https://openspeech.bytedance.com/api/v3/auc/bigmodel/recognize/flash",
                    "volc.bigasr.auc_turbo",
                    &["volc.bigasr.auc_turbo"],
                    "doubao-flash",
                    "本地音频以 base64 发送，使用 X-Api-Key 鉴权。",
                    7_200_000,
                    100 * 1024 * 1024,
                    true,
                    true,
                    Some("单段上限约 2 小时 / 100MB；返回时间戳与说话人时直接采用。"),
                )),
            ),
            provider(
                "deepseek",
                "DeepSeek",
                "OpenAI 兼容文本分析。",
                &["analysis"],
                single_key_fields("DeepSeek API Key", "填写 DeepSeek API Key"),
                Some("https://platform.deepseek.com/api_keys"),
                Some(analysis(
                    "https://api.deepseek.com/v1",
                    "deepseek-chat",
                    &["deepseek-chat", "deepseek-reasoner"],
                    "OpenAI 兼容",
                )),
                None,
            ),
            provider(
                "moonshot",
                "月之暗面 Kimi",
                "OpenAI 兼容文本分析。",
                &["analysis"],
                single_key_fields("Kimi API Key", "填写 Moonshot API Key"),
                Some("https://platform.moonshot.cn/console/api-keys"),
                Some(analysis(
                    "https://api.moonshot.cn/v1",
                    "moonshot-v1-8k",
                    &["moonshot-v1-8k", "moonshot-v1-32k"],
                    "OpenAI 兼容",
                )),
                None,
            ),
            provider(
                "minimax",
                "MiniMax",
                "OpenAI 兼容文本分析。",
                &["analysis"],
                single_key_fields("MiniMax API Key", "填写 MiniMax API Key"),
                Some("https://platform.minimaxi.com/user-center/basic-information/interface-key"),
                Some(analysis(
                    "https://api.minimax.chat/v1",
                    "abab6.5s-chat",
                    &["abab6.5s-chat"],
                    "OpenAI 兼容",
                )),
                None,
            ),
            provider(
                "siliconflow",
                "硅基流动",
                "OpenAI 兼容文本分析。",
                &["analysis"],
                single_key_fields("硅基流动 API Key", "填写 SiliconFlow API Key"),
                Some("https://cloud.siliconflow.cn/account/ak"),
                Some(analysis(
                    "https://api.siliconflow.cn/v1",
                    "Qwen/Qwen2.5-7B-Instruct",
                    &["Qwen/Qwen2.5-7B-Instruct"],
                    "OpenAI 兼容",
                )),
                None,
            ),
            provider(
                CUSTOM_PROVIDER_ID,
                "自定义 OpenAI 兼容接口",
                "自建服务、中转站或目录外的服务商。",
                &["transcription", "analysis"],
                single_key_fields("API Key", "填写该服务商的 API Key"),
                None,
                Some(custom_analysis()),
                Some(custom_transcription()),
            ),
        ],
    }
}

#[allow(clippy::too_many_arguments)]
fn provider(
    id: &str,
    label: &str,
    description: &str,
    capabilities: &[&str],
    credential_fields: Vec<CredentialFieldDefinition>,
    key_url: Option<&str>,
    analysis: Option<ProviderCapability>,
    transcription: Option<ProviderCapability>,
) -> ProviderDefinition {
    ProviderDefinition {
        id: id.to_owned(),
        label: label.to_owned(),
        description: description.to_owned(),
        capabilities: capabilities.iter().map(|value| (*value).to_owned()).collect(),
        credential_fields,
        key_url: key_url.map(str::to_owned),
        analysis,
        transcription,
    }
}

fn analysis(
    base_url: &str,
    recommended_model: &str,
    models: &[&str],
    protocol: &str,
) -> ProviderCapability {
    ProviderCapability {
        base_url: base_url.to_owned(),
        recommended_model: recommended_model.to_owned(),
        models: models.iter().map(|value| (*value).to_owned()).collect(),
        supports_model_list: true,
        protocol: protocol.to_owned(),
        protocol_hint: "OpenAI 兼容的 /v1 聊天接口。".to_owned(),
        max_duration_ms: None,
        max_file_bytes: None,
        returns_timestamps: false,
        returns_speakers: false,
        note: None,
    }
}

#[allow(clippy::too_many_arguments)]
fn transcription(
    base_url: &str,
    recommended_model: &str,
    models: &[&str],
    protocol: &str,
    protocol_hint: &str,
    max_duration_ms: i64,
    max_file_bytes: u64,
    returns_timestamps: bool,
    returns_speakers: bool,
    note: Option<&str>,
) -> ProviderCapability {
    ProviderCapability {
        base_url: base_url.to_owned(),
        recommended_model: recommended_model.to_owned(),
        models: models.iter().map(|value| (*value).to_owned()).collect(),
        supports_model_list: false,
        protocol: protocol.to_owned(),
        protocol_hint: protocol_hint.to_owned(),
        max_duration_ms: Some(max_duration_ms),
        max_file_bytes: Some(max_file_bytes),
        returns_timestamps,
        returns_speakers,
        note: note.map(str::to_owned),
    }
}

fn custom_analysis() -> ProviderCapability {
    ProviderCapability {
        base_url: String::new(),
        recommended_model: String::new(),
        models: Vec::new(),
        supports_model_list: true,
        protocol: "openai-compatible".to_owned(),
        protocol_hint: "填写 OpenAI 兼容的 /v1 地址与模型 ID。".to_owned(),
        max_duration_ms: None,
        max_file_bytes: None,
        returns_timestamps: false,
        returns_speakers: false,
        note: None,
    }
}

fn custom_transcription() -> ProviderCapability {
    ProviderCapability {
        base_url: String::new(),
        recommended_model: String::new(),
        models: Vec::new(),
        supports_model_list: true,
        protocol: "openai-multipart".to_owned(),
        protocol_hint: "填写 OpenAI 兼容的 /v1 地址与音频转写模型 ID。".to_owned(),
        max_duration_ms: Some(1_200_000),
        max_file_bytes: Some(24 * 1024 * 1024),
        returns_timestamps: true,
        returns_speakers: false,
        note: Some("自定义接口按 OpenAI multipart 协议调用。".to_owned()),
    }
}

impl ProviderCapability {
    fn with_model_list(mut self) -> Self {
        self.supports_model_list = true;
        self
    }
}

fn single_key_fields(label: &str, placeholder: &str) -> Vec<CredentialFieldDefinition> {
    vec![credential("apiKey", label, placeholder, true, None)]
}

fn credential(
    key: &str,
    label: &str,
    placeholder: &str,
    secret: bool,
    help: Option<&str>,
) -> CredentialFieldDefinition {
    CredentialFieldDefinition {
        key: key.to_owned(),
        label: label.to_owned(),
        placeholder: placeholder.to_owned(),
        secret,
        help: help.map(str::to_owned),
    }
}

pub fn provider_by_id(id: &str) -> Option<ProviderDefinition> {
    provider_catalog()
        .providers
        .into_iter()
        .find(|provider| provider.id == id)
}

pub fn capability(provider_id: &str, capability: &str) -> Option<ProviderCapability> {
    let provider = provider_by_id(provider_id)?;
    match capability {
        "analysis" => provider.analysis,
        "transcription" => provider.transcription,
        _ => None,
    }
}

pub fn provider_id_for_base_url(capability: &str, base_url: &str) -> Option<String> {
    let wanted = base_url.trim().trim_end_matches('/');
    provider_catalog()
        .providers
        .into_iter()
        .filter(|provider| provider.id != CUSTOM_PROVIDER_ID)
        .find(|provider| {
            let Some(config) = (match capability {
                "analysis" => provider.analysis.as_ref(),
                "transcription" => provider.transcription.as_ref(),
                _ => None,
            }) else {
                return false;
            };
            config.base_url.trim_end_matches('/') == wanted
        })
        .map(|provider| provider.id)
}

/// Maps legacy provider IDs and URL-only settings onto the current catalog.
pub fn normalize_provider_id(
    capability: &str,
    provider_id: &str,
    base_url: &str,
) -> String {
    let value = provider_id.trim();
    if value == "openai-compatible" || value.is_empty() || value == "none" {
        return provider_id_for_base_url(capability, base_url)
            .unwrap_or_else(|| CUSTOM_PROVIDER_ID.to_owned());
    }
    if provider_by_id(value)
        .as_ref()
        .and_then(|provider| match capability {
            "analysis" => provider.analysis.as_ref(),
            "transcription" => provider.transcription.as_ref(),
            _ => None,
        })
        .is_some()
    {
        value.to_owned()
    } else {
        CUSTOM_PROVIDER_ID.to_owned()
    }
}

pub fn resolve_config(
    capability_name: &str,
    provider_id: &str,
    requested_base_url: &str,
    requested_model: &str,
) -> AppResult<ResolvedProviderConfig> {
    let base_url = resolve_base_url(capability_name, provider_id, requested_base_url)?;
    let config = capability(provider_id, capability_name)
        .ok_or_else(|| AppError::Invalid("该服务商不支持所选能力".to_owned()))?;
    let model = if requested_model.trim().is_empty() {
        config.recommended_model.clone()
    } else {
        requested_model.trim().to_owned()
    };
    if model.is_empty() || model.len() > 160 {
        return Err(AppError::Invalid("模型名称无效".to_owned()));
    }
    Ok(ResolvedProviderConfig {
        provider_id: provider_id.trim().to_owned(),
        base_url,
        model,
    })
}

/// Resolves only the endpoint for operations such as model listing that do not
/// have a model selected yet.
pub fn resolve_base_url(
    capability_name: &str,
    provider_id: &str,
    requested_base_url: &str,
) -> AppResult<String> {
    let provider_id = provider_id.trim();
    if provider_id.is_empty() || provider_id == "none" {
        return Err(AppError::Invalid("请选择第三方服务商".to_owned()));
    }
    let definition = provider_by_id(provider_id)
        .ok_or_else(|| AppError::Invalid("第三方服务商无效".to_owned()))?;
    let config = match capability_name {
        "analysis" => definition.analysis,
        "transcription" => definition.transcription,
        _ => None,
    }
    .ok_or_else(|| AppError::Invalid("该服务商不支持所选能力".to_owned()))?;
    let base_url = if provider_id == CUSTOM_PROVIDER_ID {
        requested_base_url.trim().trim_end_matches('/').to_owned()
    } else {
        config.base_url.trim_end_matches('/').to_owned()
    };
    if base_url.is_empty() || base_url.len() > 500 {
        return Err(AppError::Invalid("接口地址无效".to_owned()));
    }
    Ok(base_url)
}

pub fn expected_credential_keys(provider_id: &str) -> AppResult<Vec<String>> {
    let provider = provider_by_id(provider_id)
        .ok_or_else(|| AppError::Invalid("第三方服务商无效".to_owned()))?;
    Ok(provider
        .credential_fields
        .into_iter()
        .map(|field| field.key)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn catalog_ids_capabilities_and_credentials_are_complete() {
        let catalog = provider_catalog();
        let mut ids = HashSet::new();
        for provider in catalog.providers {
            assert!(ids.insert(provider.id.clone()), "duplicate {}", provider.id);
            assert!(!provider.label.trim().is_empty());
            assert!(!provider.credential_fields.is_empty());
            assert!(provider.analysis.is_some() || provider.transcription.is_some());
            for capability in &provider.capabilities {
                assert!(matches!(capability.as_str(), "analysis" | "transcription"));
                let supported = match capability.as_str() {
                    "analysis" => provider.analysis.is_some(),
                    "transcription" => provider.transcription.is_some(),
                    _ => false,
                };
                assert!(supported, "{} declares {capability} without config", provider.id);
            }
            let mut keys = HashSet::new();
            for field in &provider.credential_fields {
                assert!(keys.insert(field.key.clone()));
                assert!(!field.label.trim().is_empty());
            }
        }
    }

    #[test]
    fn built_in_endpoints_are_fixed_https_and_custom_requires_its_own_url() {
        let catalog = provider_catalog();
        for provider in catalog.providers {
            for config in [provider.analysis, provider.transcription].into_iter().flatten() {
                if provider.id == CUSTOM_PROVIDER_ID {
                    assert!(config.base_url.is_empty());
                } else {
                    assert!(config.base_url.starts_with("https://"), "{}", provider.id);
                }
            }
        }
        let custom = resolve_config(
            "transcription",
            CUSTOM_PROVIDER_ID,
            "https://example.com/v1",
            "whisper-1",
        )
        .unwrap();
        assert_eq!(custom.base_url, "https://example.com/v1");
        let fixed = resolve_config(
            "transcription",
            "zhipu",
            "https://attacker.example/v1",
            "",
        )
        .unwrap();
        assert_eq!(fixed.base_url, "https://open.bigmodel.cn/api/paas/v4");
        assert_eq!(fixed.model, "glm-asr-2512");
    }

    #[test]
    fn legacy_urls_are_mapped_to_catalog_providers() {
        assert_eq!(
            normalize_provider_id(
                "analysis",
                "openai-compatible",
                "https://open.bigmodel.cn/api/paas/v4"
            ),
            "zhipu"
        );
        assert_eq!(
            normalize_provider_id(
                "transcription",
                "openai-compatible",
                "https://dashscope.aliyuncs.com/compatible-mode/v1/"
            ),
            "dashscope"
        );
        assert_eq!(
            normalize_provider_id(
                "analysis",
                "openai-compatible",
                "https://relay.example/v1"
            ),
            CUSTOM_PROVIDER_ID
        );
    }

    #[test]
    fn tencent_requires_three_fields() {
        assert_eq!(
            expected_credential_keys("tencent").unwrap(),
            vec!["appId", "secretId", "secretKey"]
        );
        assert_eq!(
            expected_credential_keys("doubao").unwrap(),
            vec!["apiKey"]
        );
    }

    #[test]
    fn model_listing_is_declared_only_for_compatible_providers() {
        for provider in ["zhipu", "dashscope", "openai", CUSTOM_PROVIDER_ID] {
            assert!(
                capability(provider, "transcription")
                    .unwrap()
                    .supports_model_list,
                "{provider}"
            );
        }
        for provider in ["tencent", "doubao"] {
            assert!(
                !capability(provider, "transcription")
                    .unwrap()
                    .supports_model_list,
                "{provider}"
            );
        }
        for provider in ["zhipu", "dashscope", "openai", "deepseek", CUSTOM_PROVIDER_ID] {
            assert!(
                capability(provider, "analysis")
                    .unwrap()
                    .supports_model_list,
                "{provider}"
            );
        }
    }
}
