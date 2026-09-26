//! External AI credentials, settings, and the privacy consent gate.

use crate::error::{AppError, AppResult};
use crate::library::ManagedLibrary;
use crate::types::ExternalAiSettings;
use std::io::Write as _;
use std::process::{Command, Stdio};

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
    get_key_with_account(
        "ECHO_EXTERNAL_AI_API_KEY",
        KEYCHAIN_ACCOUNT,
        &ENV_API_KEY_SUPPRESSED,
    )
}

pub fn get_asr_api_key() -> AppResult<Option<String>> {
    get_key_with_account(
        "ECHO_EXTERNAL_ASR_API_KEY",
        ASR_KEYCHAIN_ACCOUNT,
        &ENV_ASR_API_KEY_SUPPRESSED,
    )
}

fn get_key_with_account(
    env_name: &str,
    account: &str,
    env_suppressed: &std::sync::atomic::AtomicBool,
) -> AppResult<Option<String>> {
    if !env_suppressed.load(std::sync::atomic::Ordering::SeqCst) {
        if let Ok(value) = std::env::var(env_name) {
            if !value.trim().is_empty() {
                return Ok(Some(value));
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
        Ok((!value.is_empty()).then_some(value))
    }
    #[cfg(not(target_os = "macos"))]
    Ok(None)
}

pub fn set_api_key(api_key: &str) -> AppResult<()> {
    set_key_with_account(api_key, KEYCHAIN_ACCOUNT, &ENV_API_KEY_SUPPRESSED)
}

pub fn set_asr_api_key(api_key: &str) -> AppResult<()> {
    set_key_with_account(api_key, ASR_KEYCHAIN_ACCOUNT, &ENV_ASR_API_KEY_SUPPRESSED)
}

fn set_key_with_account(
    api_key: &str,
    account: &str,
    env_suppressed: &std::sync::atomic::AtomicBool,
) -> AppResult<()> {
    if api_key.trim().is_empty() {
        return Err(AppError::Invalid("API Key 不能为空".to_owned()));
    }
    #[cfg(target_os = "macos")]
    {
        // 密钥走 stdin，不进 argv：同用户其他进程 `ps` 看不到。
        let mut child = Command::new("security")
            .args([
                "add-generic-password",
                "-U",
                "-s",
                KEYCHAIN_SERVICE,
                "-a",
                account,
            ])
            .stdin(Stdio::piped())
            .spawn()
            .map_err(|_| AppError::Io(std::io::Error::other("无法访问 macOS 钥匙串")))?;
        if let Some(stdin) = child.stdin.as_mut() {
            stdin
                .write_all(api_key.trim().as_bytes())
                .map_err(|_| AppError::Io(std::io::Error::other("无法写入 macOS 钥匙串")))?;
        }
        let status = child
            .wait()
            .map_err(|_| AppError::Io(std::io::Error::other("无法写入 macOS 钥匙串")))?;
        if !status.success() {
            return Err(AppError::Invalid(
                "API Key 保存到 macOS 钥匙串失败".to_owned(),
            ));
        }
        // 重新保存即恢复 env 兜底的默认优先级语义。
        env_suppressed.store(false, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    Err(AppError::Invalid("当前平台不支持 macOS 钥匙串"))
}

pub fn clear_api_key() -> AppResult<()> {
    clear_key_with_account(KEYCHAIN_ACCOUNT, &ENV_API_KEY_SUPPRESSED)
}

pub fn clear_asr_api_key() -> AppResult<()> {
    clear_key_with_account(ASR_KEYCHAIN_ACCOUNT, &ENV_ASR_API_KEY_SUPPRESSED)
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
    library
        .repository()
        .external_ai_settings(get_api_key()?.is_some(), get_asr_api_key()?.is_some())
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
        has_api_key: get_api_key()?.is_some(),
        transcription_has_api_key: get_asr_api_key()?.is_some(),
        ..settings
    })
}

/// 第三方 ASR 的独立音频上传门禁。
///
/// 必须在读取 ASR Key、打开音频、预处理或构造请求之前调用。门禁本身不读取
/// Key，避免把“未同意”错误误报成“配置不完整”。
pub fn require_external_asr_consent(library: &ManagedLibrary) -> AppResult<ExternalAiSettings> {
    let settings = library.repository().external_ai_settings(false, false)?;
    if settings.processing_mode != "external" {
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
        transcription_has_api_key: get_asr_api_key()?.is_some(),
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
