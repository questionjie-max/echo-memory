//! External AI credentials, settings, and the privacy consent gate.

use crate::error::{AppError, AppResult};
use crate::library::ManagedLibrary;
use crate::types::ExternalAiSettings;
use std::io::Write as _;
use std::process::{Command, Stdio};

const KEYCHAIN_SERVICE: &str = "com.soloplay.echo-memory.external-ai";
const KEYCHAIN_ACCOUNT: &str = "default";

/// env 来源的 Key 被用户在 UI 清除后，本进程内不再回退读取 env。
/// （重启后 env 仍然生效——那是运维层面的显式覆盖，UI 会在重启后如实显示已配置。）
static ENV_API_KEY_SUPPRESSED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

pub fn get_api_key() -> AppResult<Option<String>> {
    // 用户点过「清除」后 env 兜底失效，直到下次保存。
    let env_suppressed = ENV_API_KEY_SUPPRESSED.load(std::sync::atomic::Ordering::SeqCst);
    if !env_suppressed {
        if let Ok(value) = std::env::var("ECHO_EXTERNAL_AI_API_KEY") {
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
                KEYCHAIN_ACCOUNT,
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
                KEYCHAIN_ACCOUNT,
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
        ENV_API_KEY_SUPPRESSED.store(false, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    Err(AppError::Invalid("当前平台不支持 macOS 钥匙串"))
}

pub fn clear_api_key() -> AppResult<()> {
    #[cfg(target_os = "macos")]
    {
        let _ = Command::new("security")
            .args([
                "delete-generic-password",
                "-s",
                KEYCHAIN_SERVICE,
                "-a",
                KEYCHAIN_ACCOUNT,
            ])
            .status();
    }
    // 清除必须权威：Key 来自环境变量时也要让「已清除」成为事实，
    // 否则界面显示已清除、实际仍在发送（UI 状态说谎）。
    ENV_API_KEY_SUPPRESSED.store(true, std::sync::atomic::Ordering::SeqCst);
    Ok(())
}

pub fn external_settings(library: &ManagedLibrary) -> AppResult<ExternalAiSettings> {
    library
        .repository()
        .external_ai_settings(get_api_key()?.is_some())
}

/// 所有外部 AI 外发入口的唯一隐私同意门禁。
///
/// 必须在读取 API Key、访问外发材料、构造客户端或发起网络请求之前调用。
pub fn require_external_ai_consent(library: &ManagedLibrary) -> AppResult<ExternalAiSettings> {
    let settings = library.repository().external_ai_settings(false)?;
    if settings.privacy_consent_at.is_none() {
        return Err(AppError::Invalid(
            "使用外部 AI 前必须确认文本发送说明".to_owned(),
        ));
    }
    Ok(ExternalAiSettings {
        has_api_key: get_api_key()?.is_some(),
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
        let previous_value = std::env::var_os("ECHO_EXTERNAL_AI_API_KEY");
        let previous_suppressed = ENV_API_KEY_SUPPRESSED.load(Ordering::SeqCst);
        std::env::set_var("ECHO_EXTERNAL_AI_API_KEY", value);
        ENV_API_KEY_SUPPRESSED.store(false, Ordering::SeqCst);
        Self {
            previous_value,
            previous_suppressed,
            _lock: lock.lock().unwrap_or_else(|error| error.into_inner()),
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
