//! 本地麦克风录音采集层。
//!
//! 采集只负责把麦克风写入资料库的临时 WAV 文件；录音停止后的封存、
//! hash、正式记录和转写接入会在后续生命周期工作包中完成。当前模块刻意
//! 不做网络请求、实时转写或系统音频捕获。

use crate::db::repository::RecordingSessionRow;
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
    use crate::recording::RecordingInputDevice;
    use std::ffi::{CStr, CString};
    use std::os::raw::{c_char, c_float, c_int, c_void};
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

    type DeviceCallback =
        extern "C" fn(uid: *const c_char, name: *const c_char, is_default: c_int, context: *mut c_void);

    unsafe extern "C" {
        fn echo_recording_permission_status() -> c_int;
        fn echo_recording_request_permission() -> c_int;
        fn echo_recording_list_devices(callback: Option<DeviceCallback>, context: *mut c_void)
            -> c_int;
        fn echo_recording_start(path: *const c_char, device_uid: *const c_char) -> c_int;
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

    extern "C" fn collect_device(
        uid: *const c_char,
        name: *const c_char,
        is_default: c_int,
        context: *mut c_void,
    ) {
        if uid.is_null() || name.is_null() || context.is_null() {
            return;
        }
        let uid = unsafe { CStr::from_ptr(uid) }.to_string_lossy().into_owned();
        let name = unsafe { CStr::from_ptr(name) }
            .to_string_lossy()
            .into_owned();
        if uid.is_empty() || name.is_empty() {
            return;
        }
        let devices = unsafe { &mut *(context as *mut Vec<RecordingInputDevice>) };
        devices.push(RecordingInputDevice {
            id: uid,
            name,
            is_default: is_default == 1,
        });
    }

    pub(super) fn list_devices() -> AppResult<Vec<RecordingInputDevice>> {
        let mut devices = Vec::new();
        map_result(unsafe {
            echo_recording_list_devices(
                Some(collect_device),
                &mut devices as *mut Vec<RecordingInputDevice> as *mut c_void,
            )
        })?;
        Ok(devices)
    }

    pub(super) fn start(path: &Path, input_device_id: Option<&str>) -> AppResult<()> {
        let path = path
            .to_str()
            .ok_or_else(|| AppError::Invalid("录音路径不是有效的 UTF-8".to_owned()))?;
        let path =
            CString::new(path).map_err(|_| AppError::Invalid("录音路径包含无效字符".to_owned()))?;
        let device_uid = input_device_id
            .filter(|value| !value.trim().is_empty())
            .map(|value| {
                CString::new(value)
                    .map_err(|_| AppError::Invalid("麦克风设备标识包含无效字符".to_owned()))
            })
            .transpose()?;
        map_result(unsafe {
            echo_recording_start(
                path.as_ptr(),
                device_uid.as_ref().map_or(std::ptr::null(), |value| value.as_ptr()),
            )
        })
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

    pub(super) fn list_devices() -> AppResult<Vec<crate::recording::RecordingInputDevice>> {
        Ok(Vec::new())
    }

    pub(super) fn start(_path: &Path, _input_device_id: Option<&str>) -> AppResult<()> {
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
    pub(crate) fn idle() -> Self {
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
pub struct RecordingInputDevice {
    pub id: String,
    pub name: String,
    pub is_default: bool,
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
    /// 会话来源：microphone（麦克风）或 system（系统内放声）。
    pub source: String,
}

/// 麦克风会话的落盘文件名；系统内放声会话用 system-capture.wav（见 system_audio.rs）。
pub const MICROPHONE_CAPTURE_FILE_NAME: &str = "capture.wav";

/// 会话按临时文件名判断来源，恢复/放弃据此分发到对应控制器。
pub fn session_capture_source(session: &RecordingSessionRow) -> &'static str {
    if session
        .temp_path
        .ends_with(crate::system_audio::SYSTEM_CAPTURE_FILE_NAME)
    {
        "system"
    } else {
        "microphone"
    }
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

/// 封存完成、尚未入库的录音（块8 双路合并的中间产物）。
#[derive(Debug, Clone)]
pub struct CapturedRecording {
    pub session_id: String,
    pub project_id: Option<String>,
    pub file_path: PathBuf,
    pub title: String,
    pub duration_ms: i64,
    pub paused_ms: i64,
    pub bytes: u64,
}

#[derive(Clone, Default)]
pub struct RecordingController {
    active: Arc<Mutex<Option<ActiveRecording>>>,
    /// v1.4 包5：录音中打的标记（相对录音开始的毫秒、不含暂停时间、升序）。
    /// 录音停止并入库后由 ingest 转移到 records.marks_json；
    /// 会话失败/取消时随清理丢弃（标记跟录音走，录音丢了标记没有意义）。
    pending_marks: Arc<Mutex<std::collections::HashMap<String, Vec<i64>>>>,
}

impl RecordingController {
    /// 录中打点：记录当前录音位置（不含暂停时段）。没有进行中的录音时报错。
    pub fn mark(&self) -> AppResult<Vec<i64>> {
        let (session_id, relative_ms) = {
            let active = self
                .active
                .lock()
                .map_err(|_| AppError::Invalid("录音状态锁不可用".to_owned()))?;
            let current = active
                .as_ref()
                .ok_or_else(|| AppError::Invalid("当前没有正在进行的录音".to_owned()))?;
            // elapsed_for 返回的已是扣除暂停的净录音时长（原生引擎 pause 会
            // 停掉 tap 写盘，音频文件时长=净时长，记录 duration_ms 同口径）——
            // 这里**不能再减一次** paused_millis，否则标记时间轴整体前移暂停
            // 时长，seek 落点静默失准（2026-11-12 评审实锤的双重扣除 bug）。
            let relative_ms = self.elapsed_for(current).as_millis() as i64;
            (current.session_id.clone(), relative_ms.clamp(0, i64::MAX))
        };
        let mut marks = self
            .pending_marks
            .lock()
            .map_err(|_| AppError::Invalid("录音标记锁不可用".to_owned()))?;
        let list = marks.entry(session_id).or_default();
        list.push(relative_ms);
        list.sort_unstable();
        list.dedup();
        Ok(list.clone())
    }

    /// 取走某次录音会话的全部标记（取出即删除）；没有标记返回空。
    fn take_pending_marks(&self, session_id: &str) -> Vec<i64> {
        self.pending_marks
            .lock()
            .map(|mut marks| marks.remove(session_id).unwrap_or_default())
            .unwrap_or_default()
    }
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

    pub fn list_input_devices(&self) -> AppResult<Vec<RecordingInputDevice>> {
        native::list_devices()
    }

    pub fn start(
        &self,
        library: &ManagedLibrary,
        project_id: Option<&str>,
    ) -> AppResult<RecordingState> {
        self.start_with_device(library, project_id, None)
    }

    pub fn start_with_device(
        &self,
        library: &ManagedLibrary,
        project_id: Option<&str>,
        input_device_id: Option<&str>,
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
        if let Err(error) = native::start(&file_path, input_device_id) {
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
        let captured = self.stop_capture_only(library)?;
        // 单路录音：封存后立即入库（行为与拆分前完全一致）。
        self.ingest_captured(library, captured)
    }

    /// 封存阶段（不入库）：停流、落 WAV、bytes 校验、会话置 finalizing。
    /// 双路合并（块8）在两路都封存后统一走 ingest_captured。
    pub fn stop_capture_only(&self, library: &ManagedLibrary) -> AppResult<CapturedRecording> {
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
            // 录音没入库：标记随之丢弃。
            let _ = self.take_pending_marks(&finished.session_id);
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
        let paused_ms = self.paused_millis(&finished);
        Ok(CapturedRecording {
            session_id: finished.session_id,
            project_id: finished.project_id,
            file_path: finished.file_path,
            title: finished.title,
            duration_ms,
            paused_ms,
            bytes,
        })
    }

    /// 封存会话入库：导入音频、complete 会话、清理临时目录。
    /// 双路合并（dual_recording）以合并文件构造 CapturedRecording 调用这里。
    pub fn ingest_captured(
        &self,
        library: &ManagedLibrary,
        captured: CapturedRecording,
    ) -> AppResult<StoppedRecording> {
        let bytes = captured.bytes;
        let duration_ms = captured.duration_ms;
        let ingest = match library.import_audio_with_source_named(
            &captured.file_path,
            captured.project_id.as_deref(),
            true,
            "recording",
            Some(&captured.title),
        ) {
            Ok(result) => result,
            Err(error) => {
                // 录音没入库：标记随之丢弃（HashMap 防泄漏）。
                let _ = self.take_pending_marks(&captured.session_id);
                let _ = library.repository().update_recording_session_status(
                    &captured.session_id,
                    "failed",
                    duration_ms,
                    captured.paused_ms,
                    bytes.min(i64::MAX as u64) as i64,
                    Some(&error.to_string()),
                );
                return Err(error);
            }
        };
        library
            .repository()
            .complete_recording_session(&captured.session_id, &ingest.record_id)?;
        // v1.4 包5：录音中打的标记转移到正式记录（空标记不写库）。
        let marks = self.take_pending_marks(&captured.session_id);
        if !marks.is_empty() {
            if let Err(error) = library.repository().save_record_marks(&ingest.record_id, &marks) {
                // 标记是增强信息：落库失败不吞掉录音结果，如实记日志。
                eprintln!("录音标记保存失败（record={}）：{error}", ingest.record_id);
            }
        }
        if let Some(parent) = captured.file_path.parent() {
            let _ = fs::remove_dir_all(parent);
        }
        Ok(StoppedRecording {
            session_id: captured.session_id,
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
                    // 只列麦克风会话：系统内放声会话由 SystemAudioController 负责，
                    // 混在一起会让「恢复」用错路径校验（capture.wav vs system-capture.wav）。
                    // 注意必须按完整文件名比而不能用 ends_with：system-capture.wav
                    // 的字符串后缀同样是 capture.wav，用后缀会把系统会话误收进来。
                    .filter(|session| {
                        session.temp_path.ends_with(&format!("/{MICROPHONE_CAPTURE_FILE_NAME}"))
                            || session.temp_path == MICROPHONE_CAPTURE_FILE_NAME
                    })
                    .filter(|session| session.status == "recoverable" || session.status == "failed")
                    .map(|session| RecoverableRecording {
                        session_id: session.id,
                        status: session.status,
                        title: session.title,
                        started_at: session.started_at,
                        duration_ms: session.duration_ms,
                        written_bytes: session.written_bytes,
                        last_error: session.last_error,
                        source: "microphone".to_owned(),
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
pub fn list_microphone_devices(
    state: State<AppState>,
) -> Result<Vec<RecordingInputDevice>, String> {
    state
        .recording
        .list_input_devices()
        .map_err(|error| error.to_frontend())
}

#[tauri::command]
pub async fn start_recording(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    project_id: Option<String>,
    // 块8：录音来源 "microphone"（默认）| "both"（双路）。系统声单路走
    // start_system_recording 命令。
    source: Option<String>,
    // 双路的确认门：与系统声单路同款硬校验（隐私红线落 API 契约）。
    confirmed: Option<bool>,
    input_device_id: Option<String>,
    application_bundle_id: Option<String>,
) -> Result<RecordingState, String> {
    if source.as_deref() == Some("both") {
        if confirmed != Some(true) {
            return Err("双路录音会同时录到麦克风与系统声音（包括他人的声音），需要你显性同意后才能开始".to_owned());
        }
        return start_dual_recording(
            app,
            state,
            project_id,
            input_device_id,
            application_bundle_id,
        )
        .await;
    }
    // 跨源互斥（块5 挂账收口）：任一路在录或双路在录时拒绝新开始。
    crate::dual_recording::DualRecordingController::ensure_no_capture_active(state.inner())
        .map_err(|error| error.to_frontend())?;
    let recording = state.recording.clone();
    let library = state.library.clone();
    let started = tauri::async_runtime::spawn_blocking(move || {
        recording.start_with_device(
            &library,
            project_id.as_deref(),
            input_device_id.as_deref(),
        )
    })
    .await
    .map_err(|_| "开始录音后台任务异常".to_owned())?
    .map_err(|error| error.to_frontend())?;
    // 录音已成立：启动实时文字线程（模型不可用只影响预览，不影响录音）。
    let config = crate::live_transcript::config_from_library(&state.library);
    if let Some(session_id) = started.session_id.clone() {
        let wav_path = PathBuf::from(started.file_path.clone().unwrap_or_default());
        let handle = crate::live_transcript::spawn(
            app.clone(),
            state.live_transcripts.clone(),
            session_id.clone(),
            "microphone",
            wav_path,
            config,
        );
        // 块5：实时增量纪要引擎，共享实时文字线程的停止信号。
        crate::live_minutes::spawn_incremental(
            app,
            state.library.clone(),
            state.live_transcripts.clone(),
            session_id,
            "microphone",
            handle.shared_stop(),
        );
    }
    Ok(started)
}

/// 双路开始：mic + system 同时起。确认门槛在 start_recording 命令层
/// （source=="both" 时硬校验 confirmed）；启动后段任何失败都回滚两路。
async fn start_dual_recording(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    project_id: Option<String>,
    input_device_id: Option<String>,
    application_bundle_id: Option<String>,
) -> Result<RecordingState, String> {
    let state_clone = state.inner().clone();
    let project = project_id.clone();
    let started = tauri::async_runtime::spawn_blocking(move || {
        state_clone.dual.start_with_sources(
            &state_clone,
            project.as_deref(),
            input_device_id.as_deref(),
            application_bundle_id.as_deref(),
        )
    })
    .await
    .map_err(|_| "开始双路录音后台任务异常".to_owned())?
    .map_err(|error| error.to_frontend())?;
    // 双路已成立：实时文字只挂主路（麦克风）；系统声路的实时预览记为
    // 已知限制（合并前两路内容独立，预览合并属块9+ 优化项）。
    let config = crate::live_transcript::config_from_library(&state.library);
    if let Some(session_id) = started.session_id.clone() {
        let wav_path = PathBuf::from(started.file_path.clone().unwrap_or_default());
        let handle = crate::live_transcript::spawn(
            app.clone(),
            state.live_transcripts.clone(),
            session_id.clone(),
            "microphone",
            wav_path,
            config,
        );
        crate::live_minutes::spawn_incremental(
            app,
            state.library.clone(),
            state.live_transcripts.clone(),
            session_id,
            "microphone",
            handle.shared_stop(),
        );
    }
    Ok(started)
}

#[tauri::command]
pub async fn pause_recording(state: State<'_, AppState>) -> Result<RecordingState, String> {
    let dual_active = state.dual.state().is_some();
    let state_clone = state.inner().clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        if dual_active {
            state_clone.dual.pause(&state_clone)
        } else {
            state_clone.recording.pause(&state_clone.library)
        }
    })
    .await
    .map_err(|_| "暂停录音后台任务异常".to_owned())?;
    result.map_err(|error| error.to_frontend())
}

#[tauri::command]
pub async fn resume_recording(state: State<'_, AppState>) -> Result<RecordingState, String> {
    let dual_active = state.dual.state().is_some();
    let state_clone = state.inner().clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        if dual_active {
            state_clone.dual.resume(&state_clone)
        } else {
            state_clone.recording.resume(&state_clone.library)
        }
    })
    .await
    .map_err(|_| "继续录音后台任务异常".to_owned())?;
    result.map_err(|error| error.to_frontend())
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
    let dual_active = state.dual.state().is_some();
    // 无论停止成败都必须停掉实时线程：停止失败（比如没录到有效音频）时，
    // 泄漏的线程会带着 574MB 模型每秒空转到应用退出。
    let mic_session_id = state.recording.state().session_id.clone();
    let system_session_id = state.system_audio.state().session_id.clone();
    let state_clone = state.inner().clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        if dual_active {
            state_clone.dual.stop(&state_clone).map(|dual| dual.stopped)
        } else {
            state_clone.recording.stop(&state_clone.library)
        }
    })
    .await
    .map_err(|_| "停止录音后台任务异常".to_owned())?;
    for session_id in [
        mic_session_id.or_else(|| result.as_ref().ok().map(|s| s.session_id.clone())),
        system_session_id,
    ]
    .into_iter()
    .flatten()
    {
        state.live_transcripts.stop_session(&session_id);
    }
    result.map_err(|error| error.to_frontend())
}

#[tauri::command]
pub async fn cancel_recording(state: State<'_, AppState>) -> Result<(), String> {
    let dual_active = state.dual.state().is_some();
    let mic_session_id = state.recording.state().session_id.clone();
    let system_session_id = state.system_audio.state().session_id.clone();
    let state_clone = state.inner().clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        if dual_active {
            state_clone.dual.cancel(&state_clone)
        } else {
            state_clone.recording.cancel(&state_clone.library)
        }
    })
    .await
    .map_err(|_| "取消录音后台任务异常".to_owned())?;
    for session_id in [mic_session_id, system_session_id].into_iter().flatten() {
        state.live_transcripts.stop_session(&session_id);
    }
    result.map_err(|error| error.to_frontend())
}

#[tauri::command]
pub fn get_recording_state(state: State<AppState>) -> RecordingState {
    state.recording.state()
}

#[tauri::command]
pub fn list_recoverable_recordings(
    state: State<AppState>,
) -> Result<Vec<RecoverableRecording>, String> {
    let mut all = state
        .recording
        .recoverable_sessions(&state.library)
        .map_err(|error| error.to_frontend())?;
    crate::system_audio::system_recoverable_into(&state.library, &mut all)
        .map_err(|error| error.to_frontend())?;
    all.sort_by(|left, right| right.started_at.cmp(&left.started_at));
    Ok(all)
}

/// 查会话来源（麦克风 / 系统内放声），恢复与放弃据此分发到对应控制器。
fn session_capture_source_of(
    library: &ManagedLibrary,
    session_id: &str,
) -> AppResult<&'static str> {
    let session: RecordingSessionRow = library.repository().get_recording_session(session_id)?;
    Ok(session_capture_source(&session))
}

#[tauri::command]
pub async fn recover_recording(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<StoppedRecording, String> {
    let library = state.library.clone();
    let source = tauri::async_runtime::spawn_blocking({
        let library = library.clone();
        let session_id = session_id.clone();
        move || session_capture_source_of(&library, &session_id)
    })
    .await
    .map_err(|_| "恢复录音后台任务异常".to_owned())?
    .map_err(|error| error.to_frontend())?;
    match source {
        "system" => {
            let controller = state.system_audio.clone();
            let library = state.library.clone();
            tauri::async_runtime::spawn_blocking(move || {
                controller.recover_recording(&library, &session_id)
            })
            .await
            .map_err(|_| "恢复录音后台任务异常".to_owned())?
            .map_err(|error| error.to_frontend())
        }
        _ => {
            let controller = state.recording.clone();
            let library = state.library.clone();
            tauri::async_runtime::spawn_blocking(move || {
                controller.recover_recording(&library, &session_id)
            })
            .await
            .map_err(|_| "恢复录音后台任务异常".to_owned())?
            .map_err(|error| error.to_frontend())
        }
    }
}

#[tauri::command]
pub fn discard_recoverable_recording(
    state: State<AppState>,
    session_id: String,
) -> Result<(), String> {
    let result = match session_capture_source_of(&state.library, &session_id) {
        Ok("system") => {
            state
                .system_audio
                .discard_recoverable(&state.library, &session_id)
        }
        Ok(_) => state
            .recording
            .discard_recoverable(&state.library, &session_id),
        Err(error) => Err(error),
    };
    result.map_err(|error| error.to_frontend())
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
    fn system_sessions_are_not_mislisted_as_microphone() {
        // 回归：「system-capture.wav」字符串后缀包含「capture.wav」，
        // 过滤若用 ends_with 会把系统会话同时列进麦克风恢复列表（来源标错 + 重复行）。
        let system_path = format!(
            "recordings/inflight/{}/{}",
            Uuid::new_v4(),
            crate::system_audio::SYSTEM_CAPTURE_FILE_NAME
        );
        let mic_path = format!(
            "recordings/inflight/{}/{}",
            Uuid::new_v4(),
            MICROPHONE_CAPTURE_FILE_NAME
        );
        assert!(!system_path.ends_with(&format!("/{MICROPHONE_CAPTURE_FILE_NAME}")));
        assert!(mic_path.ends_with(&format!("/{MICROPHONE_CAPTURE_FILE_NAME}")));
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

    fn active_recording(started_ago: Duration, paused_total: Duration, paused_ago: Option<Duration>) -> ActiveRecording {
        ActiveRecording {
            session_id: "session-marks-1".to_owned(),
            project_id: None,
            file_path: PathBuf::from("capture.wav"),
            title: "测试录音".to_owned(),
            started_at: Instant::now() - started_ago,
            paused_at: paused_ago.map(|ago| Instant::now() - ago),
            paused_total,
            last_checkpoint: Instant::now(),
        }
    }

    /// 回归（2026-11-12 评审）：mark() 曾经在 elapsed_for（已扣暂停）之后
    /// 又减一次 paused_millis，暂停过的录音标记整体前移暂停时长。
    /// 正确口径：标记 = elapsed_for = 净录音时长（音频文件/记录 duration 同口径）。
    #[test]
    fn mark_uses_net_elapsed_without_double_deducting_pause() {
        // 录制 10 秒，其中暂停了 4 秒（已结束的暂停）→ 净时长约 6 秒
        let controller = RecordingController::default();
        let active = active_recording(
            Duration::from_secs(10),
            Duration::from_secs(4),
            None,
        );
        *controller.active.lock().unwrap() = Some(active);
        let marks = controller.mark().unwrap();
        assert_eq!(marks, vec![6_000], "标记必须是净时长，不能再扣一次暂停");

        // 再次打点：时间推进，标记保持升序
        let active2 = active_recording(
            Duration::from_secs(12),
            Duration::from_secs(4),
            None,
        );
        *controller.active.lock().unwrap() = Some(active2);
        let marks = controller.mark().unwrap();
        assert_eq!(marks, vec![6_000, 8_000]);
    }

    #[test]
    fn mark_rejects_without_active_recording() {
        let controller = RecordingController::default();
        assert!(controller.mark().is_err());
    }

    #[test]
    fn take_pending_marks_is_one_shot_and_empty_for_unknown_session() {
        let controller = RecordingController::default();
        // 未知会话：空
        assert!(controller.take_pending_marks("unknown").is_empty());
    }
}
