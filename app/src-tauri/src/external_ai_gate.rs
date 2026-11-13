//! External AI credentials, settings, and the privacy consent gate.

use crate::error::{AppError, AppResult};
use crate::library::ManagedLibrary;
use crate::provider_catalog;
use crate::types::ExternalAiSettings;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::process::Command;

const KEYCHAIN_SERVICE: &str = "com.soloplay.echo-memory.external-ai";
const KEYCHAIN_ACCOUNT: &str = "default";
const ASR_KEYCHAIN_ACCOUNT: &str = "asr";

/// env 来源的 Key 被用户在 UI 清除后，本进程内不再回退读取 env。
/// （重启后 env 仍然生效——那是运维层面的显式覆盖，UI 会在重启后如实显示已配置。）
static ENV_API_KEY_SUPPRESSED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
static ENV_ASR_API_KEY_SUPPRESSED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

pub fn get_api_key() -> AppResult<Option<String>> {
    get_credentials_with_account(
        "ECHO_EXTERNAL_AI_API_KEY",
        KEYCHAIN_ACCOUNT,
        &ENV_API_KEY_SUPPRESSED,
    )
    .map(|credentials| credentials.and_then(|fields| fields.get("apiKey").cloned()))
}

pub fn get_asr_api_key() -> AppResult<Option<String>> {
    get_asr_credentials().map(|credentials| credentials.and_then(|fields| fields.get("apiKey").cloned()))
}

pub fn get_asr_credentials() -> AppResult<Option<BTreeMap<String, String>>> {
    get_credentials_with_account(
        "ECHO_EXTERNAL_ASR_API_KEY",
        ASR_KEYCHAIN_ACCOUNT,
        &ENV_ASR_API_KEY_SUPPRESSED,
    )
}

pub fn get_analysis_credentials() -> AppResult<Option<BTreeMap<String, String>>> {
    get_credentials_with_account(
        "ECHO_EXTERNAL_AI_API_KEY",
        KEYCHAIN_ACCOUNT,
        &ENV_API_KEY_SUPPRESSED,
    )
}

/// 当前槽位凭据是否实际来自环境变量（env 优先语义）。
/// 回滚决策用：env 命中时回滚目标=删除 Keychain 条目而非回写 env 值。
pub(crate) fn analysis_credentials_from_env() -> bool {
    !ENV_API_KEY_SUPPRESSED.load(std::sync::atomic::Ordering::SeqCst)
        && std::env::var("ECHO_EXTERNAL_AI_API_KEY")
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false)
}

pub(crate) fn transcription_credentials_from_env() -> bool {
    !ENV_ASR_API_KEY_SUPPRESSED.load(std::sync::atomic::Ordering::SeqCst)
        && std::env::var("ECHO_EXTERNAL_ASR_API_KEY")
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false)
}

fn get_credentials_with_account(
    env_name: &str,
    account: &str,
    env_suppressed: &std::sync::atomic::AtomicBool,
) -> AppResult<Option<BTreeMap<String, String>>> {
    if !env_suppressed.load(std::sync::atomic::Ordering::SeqCst) {
        if let Ok(value) = std::env::var(env_name) {
            if !value.trim().is_empty() {
                return Ok(Some(parse_credentials(&value)));
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        let output = Command::new("security")
            .args([
                "find-generic-password",
                "-s",
                KEYCHAIN_SERVICE,
                "-a",
                account,
                "-w",
            ])
            .output()
            .map_err(|_| AppError::Io(std::io::Error::other("无法访问 macOS 钥匙串")))?;
        if !output.status.success() {
            return Ok(None);
        }
        let value = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        Ok((!value.is_empty()).then(|| parse_credentials(&value)))
    }
    #[cfg(not(target_os = "macos"))]
    Ok(None)
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(transparent)]
struct StoredCredentials(BTreeMap<String, String>);

pub fn parse_credentials(raw: &str) -> BTreeMap<String, String> {
    let trimmed = raw.trim();
    if let Ok(StoredCredentials(fields)) = serde_json::from_str::<StoredCredentials>(trimmed) {
        return fields;
    }
    let mut fields = BTreeMap::new();
    fields.insert("apiKey".to_owned(), trimmed.to_owned());
    fields
}

pub fn credentials_are_complete(
    provider_id: &str,
    credentials: Option<&BTreeMap<String, String>>,
) -> bool {
    let Some(credentials) = credentials else {
        return false;
    };
    let Ok(keys) = provider_catalog::expected_credential_keys(provider_id) else {
        return false;
    };
    keys.iter().all(|key| {
        credentials
            .get(key)
            .is_some_and(|value| !value.trim().is_empty())
    })
}

pub fn set_analysis_credentials(
    provider_id: &str,
    credentials: BTreeMap<String, String>,
) -> AppResult<()> {
    set_credentials_with_account(
        provider_id,
        credentials,
        KEYCHAIN_ACCOUNT,
        &ENV_API_KEY_SUPPRESSED,
    )
}

pub fn set_transcription_credentials(
    provider_id: &str,
    credentials: BTreeMap<String, String>,
) -> AppResult<()> {
    set_credentials_with_account(
        provider_id,
        credentials,
        ASR_KEYCHAIN_ACCOUNT,
        &ENV_ASR_API_KEY_SUPPRESSED,
    )
}

fn set_credentials_with_account(
    provider_id: &str,
    credentials: BTreeMap<String, String>,
    account: &str,
    env_suppressed: &std::sync::atomic::AtomicBool,
) -> AppResult<()> {
    let expected = provider_catalog::expected_credential_keys(provider_id)?;
    let mut normalized = BTreeMap::new();
    for key in expected {
        let value = credentials
            .get(&key)
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| AppError::Invalid("请完整填写该服务商要求的凭据".to_owned()))?;
        normalized.insert(key, value.to_owned());
    }
    let serialized = serde_json::to_string(&StoredCredentials(normalized))
        .map_err(|_| AppError::Invalid("凭据格式无效".to_owned()))?;
    #[cfg(target_os = "macos")]
    {
        // 写入走 Security 框架原生 API（见 native/keychain_bridge.m）：
        // `security add-generic-password` 经 stdin 传密码在 macOS 15 上会
        // 静默写空密码（exit 0），-w 参数又会把密钥暴露在进程参数里。
        // 桥接内部带写后读回校验，写不进去会如实报错，绝不假称保存成功。
        // CString 保证 NUL 结尾；serde_json 输出不含裸 NUL，转换不会失败。
        let service = std::ffi::CString::new(KEYCHAIN_SERVICE)
            .map_err(|_| AppError::Invalid("钥匙串服务名无效".to_owned()))?;
        let account = std::ffi::CString::new(account)
            .map_err(|_| AppError::Invalid("钥匙串账户名无效".to_owned()))?;
        let password = std::ffi::CString::new(serialized.as_str())
            .map_err(|_| AppError::Invalid("凭据格式无效".to_owned()))?;
        let status = unsafe { echo_keychain_set(service.as_ptr(), account.as_ptr(), password.as_ptr()) };
        if status != 0 {
            return Err(AppError::Invalid(format!(
                "API Key 保存到 macOS 钥匙串失败（错误码 {status}）"
            )));
        }
        // 重新保存即恢复 env 兜底的默认优先级语义。
        env_suppressed.store(false, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    Err(AppError::Invalid("当前平台不支持 macOS 钥匙串"))
}

// Security 框架钥匙串写入（native/keychain_bridge.m）。0=成功，非 0 见桥接注释。
#[cfg(target_os = "macos")]
extern "C" {
    fn echo_keychain_set(
        service: *const std::ffi::c_char,
        account: *const std::ffi::c_char,
        password: *const std::ffi::c_char,
    ) -> i32;
}

pub fn clear_analysis_credentials() -> AppResult<()> {
    clear_key_with_account(KEYCHAIN_ACCOUNT, &ENV_API_KEY_SUPPRESSED)
}

pub fn clear_transcription_credentials() -> AppResult<()> {
    clear_key_with_account(ASR_KEYCHAIN_ACCOUNT, &ENV_ASR_API_KEY_SUPPRESSED)
}

/// 回滚专用：只删除 Keychain 条目，**不**置 env suppressed 标志。
/// 提交失败的回滚不该改变运行时取 Key 语义（env 命中时 env 继续生效），
/// 这与用户主动「清除凭据」不同——后者必须权威（含 env）。
pub(crate) fn rollback_delete_keychain_entry(target: &str) -> AppResult<()> {
    let account = if target == "analysis" {
        KEYCHAIN_ACCOUNT
    } else if target == "transcription" {
        ASR_KEYCHAIN_ACCOUNT
    } else {
        return Err(AppError::Invalid("凭据目标无效".to_owned()));
    };
    #[cfg(target_os = "macos")]
    {
        let _ = Command::new("security")
            .args([
                "delete-generic-password",
                "-s",
                KEYCHAIN_SERVICE,
                "-a",
                account,
            ])
            .status();
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    Err(AppError::Invalid("当前平台不支持 macOS 钥匙串"))
}

fn clear_key_with_account(
    account: &str,
    env_suppressed: &std::sync::atomic::AtomicBool,
) -> AppResult<()> {
    #[cfg(target_os = "macos")]
    {
        let _ = Command::new("security")
            .args([
                "delete-generic-password",
                "-s",
                KEYCHAIN_SERVICE,
                "-a",
                account,
            ])
            .status();
    }
    // 清除必须权威：Key 来自环境变量时也要让「已清除」成为事实，
    // 否则界面显示已清除、实际仍在发送（UI 状态说谎）。
    env_suppressed.store(true, std::sync::atomic::Ordering::SeqCst);
    Ok(())
}

pub fn external_settings(library: &ManagedLibrary) -> AppResult<ExternalAiSettings> {
    let mut settings = library.repository().external_ai_settings(false, false)?;
    settings.has_api_key = credentials_are_complete(
        &settings.analysis_provider,
        get_analysis_credentials()?.as_ref(),
    );
    settings.transcription_has_api_key = credentials_are_complete(
        &settings.transcription_provider,
        get_asr_credentials()?.as_ref(),
    );
    Ok(settings)
}

/// 第三方转写通道是否已配齐。
///
/// 转录分流（走不走第三方转写）与导入流程共用这一份判定，保证两处
/// 永远给出同一个答案。条件与 `require_external_asr_consent` 的拒绝项
/// 严格镜像（模式、同意、地址、模型、提供方），外加 Key 存在性——
/// 缺任何一项都视为未配齐，转录自动回退内置 Whisper，导入不被拦死。
pub fn external_asr_channel_ready(library: &ManagedLibrary) -> AppResult<bool> {
    let settings = library.repository().external_ai_settings(false, false)?;
    if settings.transcription_mode != "external" {
        return Ok(false);
    }
    if settings.audio_upload_consent_at.is_none() {
        return Ok(false);
    }
    if settings.transcription_provider.trim().is_empty()
        || settings.transcription_provider == "none"
        || settings.transcription_base_url.trim().is_empty()
        || settings.transcription_model.trim().is_empty()
    {
        return Ok(false);
    }
    Ok(credentials_are_complete(
        &settings.transcription_provider,
        get_asr_credentials()?.as_ref(),
    ))
}

/// 第三方转写通道「选了但没配齐」：提供方已选定（非 none/非空），但 Key、
/// 上传同意或地址模型还缺。此时转录会自动回退内置 Whisper——语义上不再是
/// 「用户从未配置转写通道」的静默回退，导入时应给一次明确提示。
/// 从未配置（提供方为 none/未选）时返回 false：静默回退内置 Whisper 是正确语义。
pub fn external_asr_channel_half_configured(library: &ManagedLibrary) -> AppResult<bool> {
    let settings = library.repository().external_ai_settings(false, false)?;
    if settings.transcription_mode != "external" {
        return Ok(false);
    }
    if settings.transcription_provider.trim().is_empty() || settings.transcription_provider == "none"
    {
        return Ok(false);
    }
    Ok(!external_asr_channel_ready(library)?)
}

/// 所有外部 AI 外发入口的唯一隐私同意门禁。
///
/// 必须在读取 API Key、访问外发材料、构造客户端或发起网络请求之前调用。
pub fn require_external_ai_consent(library: &ManagedLibrary) -> AppResult<ExternalAiSettings> {
    let settings = library.repository().external_ai_settings(false, false)?;
    if settings.privacy_consent_at.is_none() {
        return Err(AppError::Invalid(
            "使用外部 AI 前必须确认文本发送说明".to_owned(),
        ));
    }
    Ok(ExternalAiSettings {
        has_api_key: credentials_are_complete(
            &settings.analysis_provider,
            get_analysis_credentials()?.as_ref(),
        ),
        transcription_has_api_key: credentials_are_complete(
            &settings.transcription_provider,
            get_asr_credentials()?.as_ref(),
        ),
        ..settings
    })
}

/// 第三方 ASR 的独立音频上传门禁。
///
/// 必须在读取 ASR Key、打开音频、预处理或构造请求之前调用。门禁本身不读取
/// Key，避免把“未同意”错误误报成“配置不完整”。
pub fn require_external_asr_consent(library: &ManagedLibrary) -> AppResult<ExternalAiSettings> {
    let settings = library.repository().external_ai_settings(false, false)?;
    if settings.transcription_mode != "external" {
        return Err(AppError::Invalid(
            "当前不是第三方处理模式，不能上传音频".to_owned(),
        ));
    }
    if settings.audio_upload_consent_at.is_none() {
        return Err(AppError::Invalid(
            "使用第三方转写前必须确认音频上传说明".to_owned(),
        ));
    }
    if settings.transcription_provider.trim().is_empty()
        || settings.transcription_provider == "none"
        || settings.transcription_base_url.trim().is_empty()
        || settings.transcription_model.trim().is_empty()
    {
        return Err(AppError::Invalid("请先完整配置第三方转写服务".to_owned()));
    }
    Ok(ExternalAiSettings {
        transcription_has_api_key: credentials_are_complete(
            &settings.transcription_provider,
            get_asr_credentials()?.as_ref(),
        ),
        ..settings
    })
}

#[cfg(test)]
pub(crate) struct ApiKeyEnvGuard {
    previous_value: Option<std::ffi::OsString>,
    previous_suppressed: bool,
    _lock: std::sync::MutexGuard<'static, ()>,
}

#[cfg(test)]
impl ApiKeyEnvGuard {
    pub(crate) fn set(value: &str) -> Self {
        use std::sync::atomic::Ordering;
        use std::sync::OnceLock;

        static LOCK: OnceLock<std::sync::Mutex<()>> = OnceLock::new();
        let lock = LOCK.get_or_init(|| std::sync::Mutex::new(()));
        let _lock = lock.lock().unwrap_or_else(|error| error.into_inner());
        let previous_value = std::env::var_os("ECHO_EXTERNAL_AI_API_KEY");
        let previous_suppressed = ENV_API_KEY_SUPPRESSED.load(Ordering::SeqCst);
        std::env::set_var("ECHO_EXTERNAL_AI_API_KEY", value);
        ENV_API_KEY_SUPPRESSED.store(false, Ordering::SeqCst);
        Self {
            previous_value,
            previous_suppressed,
            _lock,
        }
    }
}

#[cfg(test)]
impl Drop for ApiKeyEnvGuard {
    fn drop(&mut self) {
        use std::sync::atomic::Ordering;

        match &self.previous_value {
            Some(value) => std::env::set_var("ECHO_EXTERNAL_AI_API_KEY", value),
            None => std::env::remove_var("ECHO_EXTERNAL_AI_API_KEY"),
        }
        ENV_API_KEY_SUPPRESSED.store(self.previous_suppressed, Ordering::SeqCst);
    }
}

#[cfg(test)]
pub(crate) struct AsrApiKeyEnvGuard {
    previous_value: Option<std::ffi::OsString>,
    previous_suppressed: bool,
    _lock: std::sync::MutexGuard<'static, ()>,
}

#[cfg(test)]
impl AsrApiKeyEnvGuard {
    pub(crate) fn set(value: &str) -> Self {
        use std::sync::atomic::Ordering;
        use std::sync::OnceLock;

        static LOCK: OnceLock<std::sync::Mutex<()>> = OnceLock::new();
        let lock = LOCK.get_or_init(|| std::sync::Mutex::new(()));
        let _lock = lock.lock().unwrap_or_else(|error| error.into_inner());
        let previous_value = std::env::var_os("ECHO_EXTERNAL_ASR_API_KEY");
        let previous_suppressed = ENV_ASR_API_KEY_SUPPRESSED.load(Ordering::SeqCst);
        std::env::set_var("ECHO_EXTERNAL_ASR_API_KEY", value);
        ENV_ASR_API_KEY_SUPPRESSED.store(false, Ordering::SeqCst);
        Self {
            previous_value,
            previous_suppressed,
            _lock,
        }
    }
}

#[cfg(test)]
impl Drop for AsrApiKeyEnvGuard {
    fn drop(&mut self) {
        use std::sync::atomic::Ordering;

        match &self.previous_value {
            Some(value) => std::env::set_var("ECHO_EXTERNAL_ASR_API_KEY", value),
            None => std::env::remove_var("ECHO_EXTERNAL_ASR_API_KEY"),
        }
        ENV_ASR_API_KEY_SUPPRESSED.store(self.previous_suppressed, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_single_key_satisfies_single_key_providers_only() {
        let zhipu = parse_credentials("sk-legacy");
        assert!(credentials_are_complete("zhipu", Some(&zhipu)));
        assert!(!credentials_are_complete("tencent", Some(&zhipu)));

        let tencent = parse_credentials(
            r#"{"appId":"125","secretId":"secret-id","secretKey":"secret-key"}"#,
        );
        assert!(credentials_are_complete("tencent", Some(&tencent)));
        assert!(!credentials_are_complete("doubao", Some(&tencent)));
    }

    #[test]
    fn missing_or_blank_credentials_are_never_complete() {
        assert!(!credentials_are_complete("zhipu", None));
        let blank = BTreeMap::from([("apiKey".to_owned(), "  ".to_owned())]);
        assert!(!credentials_are_complete("zhipu", Some(&blank)));
        let unknown = parse_credentials("secret");
        assert!(!credentials_are_complete("not-a-provider", Some(&unknown)));
    }

    /// 2026-11-11 回归：`security add-generic-password` 经 stdin 传密码在
    /// macOS 15 上会静默写空密码（exit 0），之后读取永远当作「没有凭据」。
    /// 本测试走应用真实闭环：原生写入 → 原生读回 → 清理。
    /// 读回用 Security 框架（echo_keychain_get）而不是 security CLI——
    /// CLI 在无头并发测试里会因 ACL 授权弹窗卡死（2026-11-12 实测）。
    /// 会短暂写入一个专用测试条目（keychain-selftest），结束即删。
    #[cfg(target_os = "macos")]
    // #[ignore]：本测试跑真实的 Security 框架调用，在并发测试上下文里会
    // 与 securityd 发生会话死锁（2026-11-12 实测：全量并发必卡、单跑必过）。
    // 保留为手动回归项：cargo test --lib keychain -- --ignored
    #[ignore]
    #[test]
    fn keychain_write_round_trips_through_the_cli_read_path() {
        use std::ffi::CString;

        const TEST_ACCOUNT: &str = "keychain-selftest";
        let suppressed = std::sync::atomic::AtomicBool::new(false);
        let credentials = BTreeMap::from([("apiKey".to_owned(), "roundtrip-probe-key".to_owned())]);

        let written = set_credentials_with_account("zhipu", credentials.clone(), TEST_ACCOUNT, &suppressed);
        assert!(written.is_ok(), "原生写入必须成功：{written:?}");

        // 原生读回（与写入同一框架，等价于应用的真实读取结果）
        let service = CString::new(KEYCHAIN_SERVICE).unwrap();
        let account = CString::new(TEST_ACCOUNT).unwrap();
        let mut buffer = vec![0_u8; 4096];
        let status = unsafe {
            echo_keychain_get(service.as_ptr(), account.as_ptr(), buffer.as_mut_ptr() as *mut std::ffi::c_char, buffer.len())
        };
        assert_eq!(status, 0, "原生读回必须成功：{status}");
        let stored = CString::new(&buffer[..buffer.iter().position(|&b| b == 0).unwrap_or(0)])
            .unwrap()
            .into_string()
            .unwrap();
        assert!(
            stored.contains("roundtrip-probe-key"),
            "读回内容必须包含刚写入的 Key，实际得到：{stored}"
        );

        // 清理测试条目（delete 走 CLI，删除操作不返回密码、不会弹 ACL 框）。
        let _ = Command::new("security")
            .args([
                "delete-generic-password",
                "-s",
                KEYCHAIN_SERVICE,
                "-a",
                TEST_ACCOUNT,
            ])
            .status();
    }

    #[cfg(target_os = "macos")]
    extern "C" {
        fn echo_keychain_get(
            service: *const std::ffi::c_char,
            account: *const std::ffi::c_char,
            password_out: *mut std::ffi::c_char,
            buffer_len: usize,
        ) -> i32;
    }
}
