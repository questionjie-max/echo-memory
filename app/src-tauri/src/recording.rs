//! 本地麦克风录音采集层。
//!
//! 采集只负责把麦克风写入资料库的临时 WAV 文件；录音停止后的封存、
//! hash、正式记录和转写接入会在后续生命周期工作包中完成。当前模块刻意
//! 不做网络请求、实时转写或系统音频捕获。

use crate::error::{AppError, AppResult};
use crate::library::ManagedLibrary;
use crate::state::AppState;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::State;
use uuid::Uuid;

#[cfg(target_os = "macos")]
mod native {
    use crate::error::{AppError, AppResult};
    use std::ffi::CString;
    use std::os::raw::{c_char, c_float, c_int};
    use std::path::Path;

    const OK: c_int = 0;
    const INVALID_ARGUMENT: c_int = -1;
    const PERMISSION_DENIED: c_int = -2;
    const UNSUPPORTED: c_int = -3;
    const BUSY: c_int = -4;
    const NOT_ACTIVE: c_int = -5;
    const AUDIO_ERROR: c_int = -6;

    const PERMISSION_TIMEOUT: c_int = -7;

    const PERMISSION_UNDETERMINED: c_int = 0;
    const PERMISSION_DENIED_STATUS: c_int = 1;
    const PERMISSION_GRANTED: c_int = 2;
    const PERMISSION_UNSUPPORTED_STATUS: c_int = 3;

    unsafe extern "C" {
        fn echo_recording_permission_status() -> c_int;
        fn echo_recording_request_permission() -> c_int;
        fn echo_recording_start(path: *const c_char) -> c_int;
        fn echo_recording_pause() -> c_int;
        fn echo_recording_resume() -> c_int;
        fn echo_recording_stop() -> c_int;
        fn echo_recording_cancel() -> c_int;
        fn echo_recording_is_recording() -> c_int;
        fn echo_recording_levels(average_db: *mut c_float, peak_db: *mut c_float) -> c_int;
    }

    pub(super) fn permission_status() -> PermissionStatus {
        let status = unsafe { echo_recording_permission_status() };
        match status {
            PERMISSION_UNDETERMINED => PermissionStatus::Undetermined,
            PERMISSION_DENIED_STATUS => PermissionStatus::Denied,
            PERMISSION_GRANTED => PermissionStatus::Granted,
            PERMISSION_UNSUPPORTED_STATUS => PermissionStatus::Unsupported,
            _ => PermissionStatus::Unsupported,
        }
    }

    pub(super) fn request_permission() -> AppResult<bool> {
        let result = unsafe { echo_recording_request_permission() };
        match result {
            1 => Ok(true),
            0 => Ok(false),
            -2 => Err(AppError::Invalid(
                "麦克风权限请求超时：请在「系统设置 → 隐私与安全性 → 麦克风」中允许回声记忆，然后重试"
                    .to_owned(),
            )),
            _ => Err(AppError::Invalid(
                "当前 macOS 不支持麦克风权限请求".to_owned(),
            )),
        }
    }

    pub(super) fn start(path: &Path) -> AppResult<()> {
        let path = path
            .to_str()
            .ok_or_else(|| AppError::Invalid("录音路径不是有效的 UTF-8".to_owned()))?;
        let path =
            CString::new(path).map_err(|_| AppError::Invalid("录音路径包含无效字符".to_owned()))?;
        map_result(unsafe { echo_recording_start(path.as_ptr()) })
    }

    pub(super) fn pause() -> AppResult<()> {
        map_result(unsafe { echo_recording_pause() })
    }

    pub(super) fn resume() -> AppResult<()> {
        map_result(unsafe { echo_recording_resume() })
    }

    pub(super) fn stop() -> AppResult<()> {
        map_result(unsafe { echo_recording_stop() })
    }

    pub(super) fn cancel() -> AppResult<()> {
        map_result(unsafe { echo_recording_cancel() })
    }

    pub(super) fn is_recording() -> bool {
        unsafe { echo_recording_is_recording() == 1 }
    }

    pub(super) fn levels() -> AppResult<AudioLevels> {
        let mut average_db = 0.0_f32;
        let mut peak_db = 0.0_f32;
        map_result(unsafe {
            echo_recording_levels(&mut average_db as *mut f32, &mut peak_db as *mut f32)
        })?;
        Ok(AudioLevels {
            average_db,
            peak_db,
        })
    }

    fn map_result(code: i32) -> AppResult<()> {
        match code {
            OK => Ok(()),
            INVALID_ARGUMENT => Err(AppError::Invalid("录音参数无效".to_owned())),
            PERMISSION_DENIED => Err(AppError::Invalid(
                "麦克风权限未授予，请在系统设置中允许回声记忆访问麦克风".to_owned(),
            )),
            UNSUPPORTED => Err(AppError::Invalid("当前平台不支持本地麦克风录音".to_owned())),
            PERMISSION_TIMEOUT => Err(AppError::Invalid(
                "麦克风授权未在限时内完成：请在「系统设置 → 隐私与安全性 → 麦克风」中允许回声记忆，然后重试"
                    .to_owned(),
            )),
            BUSY => Err(AppError::Invalid("已有一条录音正在进行".to_owned())),
            NOT_ACTIVE => Err(AppError::Invalid("当前没有正在进行的录音".to_owned())),
            AUDIO_ERROR => Err(AppError::Import(
                "macOS 音频录音器无法打开或写入文件".to_owned(),
            )),
            other => Err(AppError::Import(format!(
                "macOS 录音器返回未知错误码 {other}"
            ))),
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(super) enum PermissionStatus {
        Undetermined,
        Denied,
        Granted,
        Unsupported,
    }

    #[derive(Debug, Clone, Copy)]
    pub(super) struct AudioLevels {
        pub average_db: f32,
        pub peak_db: f32,
    }
}

#[cfg(not(target_os = "macos"))]
mod native {
    use crate::error::{AppError, AppResult};
    use std::path::Path;
    use std::time::Duration;

    pub(super) fn permission_status() -> PermissionStatus {
        PermissionStatus::Unsupported
    }

    pub(super) fn request_permission() -> AppResult<bool> {
        Err(AppError::Invalid("当前平台不支持本地麦克风录音".to_owned()))
    }

    pub(super) fn start(_path: &Path) -> AppResult<()> {
        Err(AppError::Invalid("当前平台不支持本地麦克风录音".to_owned()))
    }

    pub(super) fn pause() -> AppResult<()> {
        Err(AppError::Invalid("当前平台不支持本地麦克风录音".to_owned()))
    }

    pub(super) fn resume() -> AppResult<()> {
        Err(AppError::Invalid("当前平台不支持本地麦克风录音".to_owned()))
    }

    pub(super) fn stop() -> AppResult<()> {
        Err(AppError::Invalid("当前平台不支持本地麦克风录音".to_owned()))
    }

    pub(super) fn cancel() -> AppResult<()> {
        Err(AppError::Invalid("当前平台不支持本地麦克风录音".to_owned()))
    }

    pub(super) fn is_recording() -> bool {
        false
    }

    pub(super) fn current_time() -> Duration {
        Duration::ZERO
    }

    pub(super) fn levels() -> AppResult<AudioLevels> {
        Err(AppError::Invalid("当前平台不支持本地麦克风录音".to_owned()))
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(super) enum PermissionStatus {
        Undetermined,
        Denied,
        Granted,
        Unsupported,
    }

    #[derive(Debug, Clone, Copy)]
    pub(super) struct AudioLevels {
        pub average_db: f32,
        pub peak_db: f32,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RecordingStatus {
    Idle,
    Starting,
    Recording,
    Paused,
    Finalizing,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingState {
    pub status: RecordingStatus,
    pub session_id: Option<String>,
    pub elapsed_ms: i64,
    pub input_level_db: Option<f32>,
    pub peak_level_db: Option<f32>,
    pub file_path: Option<String>,
    pub error: Option<String>,
}

impl RecordingState {
    fn idle() -> Self {
        Self {
            status: RecordingStatus::Idle,
            session_id: None,
            elapsed_ms: 0,
            input_level_db: None,
            peak_level_db: None,
            file_path: None,
            error: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoppedRecording {
    pub session_id: String,
    pub record_id: String,
    pub duration_ms: i64,
    pub title: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MicrophonePermission {
    Undetermined,
    Denied,
    Granted,
    Unsupported,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoverableRecording {
    pub session_id: String,
    pub status: String,
    pub title: String,
    pub started_at: String,
    pub duration_ms: i64,
    pub written_bytes: i64,
    pub last_error: Option<String>,
}

#[derive(Debug)]
struct ActiveRecording {
    session_id: String,
    project_id: Option<String>,
    file_path: PathBuf,
    title: String,
    started_at: Instant,
    paused_at: Option<Instant>,
    paused_total: Duration,
    last_checkpoint: Instant,
}

#[derive(Clone, Default)]
pub struct RecordingController {
    active: Arc<Mutex<Option<ActiveRecording>>>,
}

fn validated_inflight_path(library: &ManagedLibrary, relative: &str) -> AppResult<PathBuf> {
    let relative_path = Path::new(relative);
    let components = relative_path.components().collect::<Vec<_>>();
    if relative_path.is_absolute()
        || components
            .iter()
            .any(|component| !matches!(component, Component::Normal(_)))
        || components.len() != 4
        || components[0] != Component::Normal("recordings".as_ref())
        || components[1] != Component::Normal("inflight".as_ref())
        || components[3] != Component::Normal("capture.wav".as_ref())
    {
        return Err(AppError::Invalid("可恢复录音路径无效".to_owned()));
    }
    let candidate = library.root().join(relative_path);
    let canonical_root = fs::canonicalize(library.root())
        .map_err(|_| AppError::Invalid("资料库路径不可用".to_owned()))?;
    let parent = candidate
        .parent()
        .ok_or_else(|| AppError::Invalid("可恢复录音路径无效".to_owned()))?;
    if let Ok(canonical_parent) = fs::canonicalize(parent) {
        if !canonical_parent.starts_with(&canonical_root) {
            return Err(AppError::Invalid("可恢复录音路径不在资料库内".to_owned()));
        }
    }
    Ok(candidate)
}

impl RecordingController {
    pub fn permission_status(&self) -> MicrophonePermission {
        match native::permission_status() {
            native::PermissionStatus::Undetermined => MicrophonePermission::Undetermined,
            native::PermissionStatus::Denied => MicrophonePermission::Denied,
            native::PermissionStatus::Granted => MicrophonePermission::Granted,
            native::PermissionStatus::Unsupported => MicrophonePermission::Unsupported,
        }
    }

    pub fn request_permission(&self) -> AppResult<MicrophonePermission> {
        let granted = native::request_permission()?;
        Ok(if granted {
            MicrophonePermission::Granted
        } else {
            MicrophonePermission::Denied
        })
    }

    pub fn start(
        &self,
        library: &ManagedLibrary,
        project_id: Option<&str>,
    ) -> AppResult<RecordingState> {
        let library_root = library.root();
        let mut active = self
            .active
            .lock()
            .map_err(|_| AppError::Invalid("录音状态锁不可用".to_owned()))?;
        if active.is_some() || native::is_recording() {
            return Err(AppError::Invalid("已有一条录音正在进行".to_owned()));
        }

        let session_id = Uuid::new_v4().to_string();
        let session_dir = library_root
            .join("recordings")
            .join("inflight")
            .join(&session_id);
        fs::create_dir_all(&session_dir)?;
        let file_path = session_dir.join("capture.wav");
        let relative_path = file_path
            .strip_prefix(library_root)
            .map_err(|_| AppError::Invalid("录音临时路径不在资料库内".to_owned()))?
            .to_path_buf();
        let started_at = Utc::now().to_rfc3339();
        // 标题是给用户看的文本，必须用本地时间；库里其它时间戳存 UTC、由界面转本地显示，
        // 这里若用 UTC 会出现「标题 11-08、列表 19:08」这种自相矛盾的记录。
        let title = format!("本地录音 {}", chrono::Local::now().format("%Y-%m-%d %H-%M"));
        library.repository().create_recording_session(
            &session_id,
            &relative_path.to_string_lossy(),
            &title,
            project_id,
            &started_at,
        )?;
        if let Err(error) = native::start(&file_path) {
            let _ = library.repository().update_recording_session_status(
                &session_id,
                "canceled",
                0,
                0,
                0,
                Some(&error.to_string()),
            );
            let _ = fs::remove_dir_all(&session_dir);
            return Err(error);
        }
        *active = Some(ActiveRecording {
            session_id: session_id.clone(),
            project_id: project_id.map(str::to_owned),
            file_path: file_path.clone(),
            title,
            started_at: Instant::now(),
            paused_at: None,
            paused_total: Duration::ZERO,
            last_checkpoint: Instant::now(),
        });
        if let Err(error) = library.repository().update_recording_session_status(
            &session_id,
            "recording",
            0,
            0,
            0,
            None,
        ) {
            if let Err(stop_error) = native::stop() {
                return Err(AppError::Import(format!(
                    "录音已启动但状态无法保存：{error}；停止录音也失败：{stop_error}"
                )));
            }
            active.take();
            return Err(error);
        }
        Ok(RecordingState {
            status: RecordingStatus::Recording,
            session_id: Some(session_id),
            elapsed_ms: 0,
            input_level_db: None,
            peak_level_db: None,
            file_path: Some(file_path.to_string_lossy().into_owned()),
            error: None,
        })
    }

    pub fn pause(&self, library: &ManagedLibrary) -> AppResult<RecordingState> {
        let mut guard = self.lock_active()?;
        let active = guard
            .as_mut()
            .ok_or_else(|| AppError::Invalid("当前没有正在进行的录音".to_owned()))?;
        native::pause()?;
        active.paused_at = Some(Instant::now());
        self.checkpoint(library, active, "paused")?;
        Ok(self.state_for(active, RecordingStatus::Paused))
    }

    pub fn resume(&self, library: &ManagedLibrary) -> AppResult<RecordingState> {
        let mut guard = self.lock_active()?;
        let active = guard
            .as_mut()
            .ok_or_else(|| AppError::Invalid("当前没有正在进行的录音".to_owned()))?;
        native::resume()?;
        if let Some(paused_at) = active.paused_at.take() {
            active.paused_total += paused_at.elapsed();
        }
        self.checkpoint(library, active, "recording")?;
        Ok(self.state_for(active, RecordingStatus::Recording))
    }

    pub fn levels(&self) -> AppResult<RecordingState> {
        let mut guard = self.lock_active()?;
        let active = guard
            .as_mut()
            .ok_or_else(|| AppError::Invalid("当前没有正在进行的录音".to_owned()))?;
        let levels = native::levels()?;
        Ok(RecordingState {
            input_level_db: Some(levels.average_db),
            peak_level_db: Some(levels.peak_db),
            ..self.state_for(
                active,
                if active.paused_at.is_some() {
                    RecordingStatus::Paused
                } else {
                    RecordingStatus::Recording
                },
            )
        })
    }

    pub fn stop(&self, library: &ManagedLibrary) -> AppResult<StoppedRecording> {
        let finished = {
            let mut active = self
                .active
                .lock()
                .map_err(|_| AppError::Invalid("录音状态锁不可用".to_owned()))?;
            let current = active
                .as_ref()
                .ok_or_else(|| AppError::Invalid("当前没有正在进行的录音".to_owned()))?;
            let duration = self.elapsed_for(current);
            native::stop()?;
            let finished = active.take().expect("录音状态在同一把锁内不应丢失");
            (finished, duration)
        };
        let (finished, duration) = finished;
        let bytes = fs::metadata(&finished.file_path)
            .map(|meta| meta.len())
            .unwrap_or(0);
        let duration_ms = duration.as_millis().min(i64::MAX as u128) as i64;
        library.repository().update_recording_session_status(
            &finished.session_id,
            "finalizing",
            duration_ms,
            self.paused_millis(&finished),
            bytes.min(i64::MAX as u64) as i64,
            None,
        )?;
        if bytes <= 44 || !finished.file_path.is_file() {
            let message = "录音已停止，但没有保存有效音频";
            let _ = library.repository().update_recording_session_status(
                &finished.session_id,
                "failed",
                duration_ms,
                self.paused_millis(&finished),
                bytes.min(i64::MAX as u64) as i64,
                Some(message),
            );
            return Err(AppError::Import(message.to_owned()));
        }
        let ingest = match library.import_audio_with_source_named(
            &finished.file_path,
            finished.project_id.as_deref(),
            true,
            "recording",
            Some(&finished.title),
        ) {
            Ok(result) => result,
            Err(error) => {
                let _ = library.repository().update_recording_session_status(
                    &finished.session_id,
                    "failed",
                    duration_ms,
                    self.paused_millis(&finished),
                    bytes.min(i64::MAX as u64) as i64,
                    Some(&error.to_string()),
                );
                return Err(error);
            }
        };
        library
            .repository()
            .complete_recording_session(&finished.session_id, &ingest.record_id)?;
        if let Some(parent) = finished.file_path.parent() {
            let _ = fs::remove_dir_all(parent);
        }
        Ok(StoppedRecording {
            session_id: finished.session_id,
            record_id: ingest.record_id,
            duration_ms: ingest.duration_ms,
            title: ingest.title,
        })
    }

    pub fn cancel(&self, library: &ManagedLibrary) -> AppResult<()> {
        let mut active = self
            .active
            .lock()
            .map_err(|_| AppError::Invalid("录音状态锁不可用".to_owned()))?;
        let current = active
            .as_ref()
            .ok_or_else(|| AppError::Invalid("当前没有正在进行的录音".to_owned()))?;
        let session_id = current.session_id.clone();
        let path = current.file_path.clone();
        let paused_ms = self.paused_millis(current);
        native::cancel()?;
        active.take();
        if let Err(error) = library.repository().update_recording_session_status(
            &session_id,
            "canceled",
            0,
            paused_ms,
            0,
            None,
        ) {
            return Err(AppError::Import(format!(
                "录音已停止，但取消状态无法保存：{error}；临时文件已保留，请重启后恢复或放弃"
            )));
        }
        if let Some(parent) = path.parent() {
            let _ = fs::remove_dir_all(parent);
        }
        Ok(())
    }

    pub fn recoverable_sessions(
        &self,
        library: &ManagedLibrary,
    ) -> AppResult<Vec<RecoverableRecording>> {
        library
            .repository()
            .list_recoverable_recording_sessions()
            .map(|sessions| {
                sessions
                    .into_iter()
                    .filter(|session| session.status == "recoverable" || session.status == "failed")
                    .map(|session| RecoverableRecording {
                        session_id: session.id,
                        status: session.status,
                        title: session.title,
                        started_at: session.started_at,
                        duration_ms: session.duration_ms,
                        written_bytes: session.written_bytes,
                        last_error: session.last_error,
                    })
                    .collect()
            })
    }

    pub fn discard_recoverable(&self, library: &ManagedLibrary, session_id: &str) -> AppResult<()> {
        let session = library.repository().get_recording_session(session_id)?;
        if !matches!(session.status.as_str(), "recoverable" | "failed") {
            return Err(AppError::Invalid("该录音当前不能放弃".to_owned()));
        }
        let path = validated_inflight_path(library, &session.temp_path)?;
        if let Some(parent) = path.parent() {
            let _ = fs::remove_dir_all(parent);
        }
        library.repository().update_recording_session_status(
            session_id,
            "canceled",
            session.duration_ms,
            session.paused_ms,
            session.written_bytes,
            None,
        )
    }

    pub fn recover_recording(
        &self,
        library: &ManagedLibrary,
        session_id: &str,
    ) -> AppResult<StoppedRecording> {
        let session = library.repository().get_recording_session(session_id)?;
        if !matches!(session.status.as_str(), "recoverable" | "failed") {
            return Err(AppError::Invalid("该录音当前不能恢复".to_owned()));
        }
        let path = validated_inflight_path(library, &session.temp_path)?;
        if !path.is_file() {
            return Err(AppError::Import("找不到可恢复的临时录音文件".to_owned()));
        }
        let bytes = fs::metadata(&path)?.len();
        if bytes <= 44 {
            return Err(AppError::Import("临时录音文件为空，无法恢复".to_owned()));
        }
        library.repository().update_recording_session_status(
            session_id,
            "finalizing",
            session.duration_ms,
            session.paused_ms,
            bytes.min(i64::MAX as u64) as i64,
            None,
        )?;
        let ingest = match library.import_audio_with_source_named(
            &path,
            session.project_id.as_deref(),
            true,
            "recording",
            Some(&session.title),
        ) {
            Ok(result) => result,
            Err(error) => {
                let _ = library.repository().update_recording_session_status(
                    session_id,
                    "failed",
                    session.duration_ms,
                    session.paused_ms,
                    bytes.min(i64::MAX as u64) as i64,
                    Some(&error.to_string()),
                );
                return Err(error);
            }
        };
        library
            .repository()
            .complete_recording_session(session_id, &ingest.record_id)?;
        if let Some(parent) = path.parent() {
            let _ = fs::remove_dir_all(parent);
        }
        Ok(StoppedRecording {
            session_id: session_id.to_owned(),
            record_id: ingest.record_id,
            duration_ms: ingest.duration_ms,
            title: ingest.title,
        })
    }

    pub fn state(&self) -> RecordingState {
        let Ok(active) = self.active.lock() else {
            return RecordingState {
                status: RecordingStatus::Failed,
                error: Some("录音状态锁不可用".to_owned()),
                ..RecordingState::idle()
            };
        };
        let Some(active) = active.as_ref() else {
            return RecordingState::idle();
        };
        self.state_for(
            active,
            if active.paused_at.is_some() {
                RecordingStatus::Paused
            } else {
                RecordingStatus::Recording
            },
        )
    }

    fn lock_active(&self) -> AppResult<std::sync::MutexGuard<'_, Option<ActiveRecording>>> {
        self.active
            .lock()
            .map_err(|_| AppError::Invalid("录音状态锁不可用".to_owned()))
    }

    fn elapsed_for(&self, active: &ActiveRecording) -> Duration {
        let paused = active
            .paused_total
            .saturating_add(active.paused_at.map(|at| at.elapsed()).unwrap_or_default());
        active.started_at.elapsed().saturating_sub(paused)
    }

    fn paused_millis(&self, active: &ActiveRecording) -> i64 {
        active
            .paused_total
            .saturating_add(active.paused_at.map(|at| at.elapsed()).unwrap_or_default())
            .as_millis()
            .min(i64::MAX as u128) as i64
    }

    fn checkpoint(
        &self,
        library: &ManagedLibrary,
        active: &mut ActiveRecording,
        status: &str,
    ) -> AppResult<()> {
        let now = Instant::now();
        if status == "recording"
            && now.duration_since(active.last_checkpoint) < Duration::from_secs(5)
        {
            return Ok(());
        }
        let bytes = fs::metadata(&active.file_path)
            .map(|metadata| metadata.len().min(i64::MAX as u64) as i64)
            .unwrap_or(0);
        let duration_ms = self.elapsed_for(active).as_millis().min(i64::MAX as u128) as i64;
        library.repository().update_recording_session_status(
            &active.session_id,
            status,
            duration_ms,
            self.paused_millis(active),
            bytes,
            None,
        )?;
        active.last_checkpoint = now;
        Ok(())
    }

    fn state_for(&self, active: &ActiveRecording, status: RecordingStatus) -> RecordingState {
        let elapsed = self.elapsed_for(active);
        RecordingState {
            status,
            session_id: Some(active.session_id.clone()),
            elapsed_ms: elapsed.as_millis().min(i64::MAX as u128) as i64,
            input_level_db: None,
            peak_level_db: None,
            file_path: Some(active.file_path.to_string_lossy().into_owned()),
            error: None,
        }
    }
}

#[tauri::command]
pub fn microphone_permission(state: State<AppState>) -> Result<MicrophonePermission, String> {
    Ok(state.recording.permission_status())
}

#[tauri::command]
pub async fn request_microphone_permission(
    state: State<'_, AppState>,
) -> Result<MicrophonePermission, String> {
    let recording = state.recording.clone();
    tauri::async_runtime::spawn_blocking(move || recording.request_permission())
        .await
        .map_err(|_| "麦克风权限请求后台任务异常".to_owned())?
        .map_err(|error| error.to_frontend())
}

#[tauri::command]
pub async fn start_recording(
    state: State<'_, AppState>,
    project_id: Option<String>,
) -> Result<RecordingState, String> {
    let recording = state.recording.clone();
    let library = state.library.clone();
    tauri::async_runtime::spawn_blocking(move || recording.start(&library, project_id.as_deref()))
        .await
        .map_err(|_| "开始录音后台任务异常".to_owned())?
        .map_err(|error| error.to_frontend())
}

#[tauri::command]
pub async fn pause_recording(state: State<'_, AppState>) -> Result<RecordingState, String> {
    let recording = state.recording.clone();
    let library = state.library.clone();
    tauri::async_runtime::spawn_blocking(move || recording.pause(&library))
        .await
        .map_err(|_| "暂停录音后台任务异常".to_owned())?
        .map_err(|error| error.to_frontend())
}

#[tauri::command]
pub async fn resume_recording(state: State<'_, AppState>) -> Result<RecordingState, String> {
    let recording = state.recording.clone();
    let library = state.library.clone();
    tauri::async_runtime::spawn_blocking(move || recording.resume(&library))
        .await
        .map_err(|_| "继续录音后台任务异常".to_owned())?
        .map_err(|error| error.to_frontend())
}

#[tauri::command]
pub fn recording_levels(state: State<AppState>) -> Result<RecordingState, String> {
    state
        .recording
        .levels()
        .map_err(|error| error.to_frontend())
}

#[tauri::command]
pub async fn stop_recording(state: State<'_, AppState>) -> Result<StoppedRecording, String> {
    let recording = state.recording.clone();
    let library = state.library.clone();
    tauri::async_runtime::spawn_blocking(move || recording.stop(&library))
        .await
        .map_err(|_| "停止录音后台任务异常".to_owned())?
        .map_err(|error| error.to_frontend())
}

#[tauri::command]
pub async fn cancel_recording(state: State<'_, AppState>) -> Result<(), String> {
    let recording = state.recording.clone();
    let library = state.library.clone();
    tauri::async_runtime::spawn_blocking(move || recording.cancel(&library))
        .await
        .map_err(|_| "取消录音后台任务异常".to_owned())?
        .map_err(|error| error.to_frontend())
}

#[tauri::command]
pub fn get_recording_state(state: State<AppState>) -> RecordingState {
    state.recording.state()
}

#[tauri::command]
pub fn list_recoverable_recordings(
    state: State<AppState>,
) -> Result<Vec<RecoverableRecording>, String> {
    state
        .recording
        .recoverable_sessions(&state.library)
        .map_err(|error| error.to_frontend())
}

#[tauri::command]
pub async fn recover_recording(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<StoppedRecording, String> {
    let recording = state.recording.clone();
    let library = state.library.clone();
    tauri::async_runtime::spawn_blocking(move || recording.recover_recording(&library, &session_id))
        .await
        .map_err(|_| "恢复录音后台任务异常".to_owned())?
        .map_err(|error| error.to_frontend())
}

#[tauri::command]
pub fn discard_recoverable_recording(
    state: State<AppState>,
    session_id: String,
) -> Result<(), String> {
    state
        .recording
        .discard_recoverable(&state.library, &session_id)
        .map_err(|error| error.to_frontend())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_state_has_no_active_file() {
        let state = RecordingState::idle();
        assert_eq!(state.status, RecordingStatus::Idle);
        assert!(state.session_id.is_none());
        assert!(state.file_path.is_none());
    }

    #[test]
    fn elapsed_excludes_paused_time() {
        let controller = RecordingController::default();
        let active = ActiveRecording {
            session_id: "test".to_owned(),
            project_id: None,
            file_path: PathBuf::from("capture.wav"),
            title: "测试录音".to_owned(),
            started_at: Instant::now() - Duration::from_secs(3),
            paused_at: Some(Instant::now() - Duration::from_secs(2)),
            paused_total: Duration::from_secs(0),
            last_checkpoint: Instant::now(),
        };
        let elapsed = controller.elapsed_for(&active);
        assert!(elapsed >= Duration::from_millis(800));
        assert!(elapsed < Duration::from_secs(2));
    }
}
