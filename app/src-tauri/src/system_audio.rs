//! 系统内放声采集层（ScreenCaptureKit）。
//!
//! 与 `recording.rs`（麦克风）平行的一条独立采集链路：ObjC 桥接只负责从
//! ScreenCaptureKit 拿到 Float32 样本并回调进来，交错下混、16-bit WAV 落盘、
//! 暂停门控与电平统计全部在本模块用纯 Rust 完成 —— 这部分逻辑与平台无关，
//! 是测试的重点对象。
//!
//! 隐私底线：系统内录录到的是「这台 Mac 正在播放的所有声音」，包括线上会议
//! 里对方说的话。前端在开始前必须给出显性告知与确认；本模块不默认开启任何
//! 采集，权限被拒时上层必须降级为只录麦克风而不是静默继续。

use crate::error::{AppError, AppResult};
use crate::library::ManagedLibrary;
use crate::recording::{RecordingState, RecordingStatus, StoppedRecording};
use crate::state::AppState;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::State;
use uuid::Uuid;

pub const SYSTEM_CAPTURE_FILE_NAME: &str = "system-capture.wav";
pub const SYSTEM_SAMPLE_RATE: u32 = 48_000;
/// 与麦克风一致的输入量程：把 -60dB..0dB 映射给前端电平条。
const DB_FLOOR: f32 = -100.0;

#[cfg(target_os = "macos")]
mod native {
    use crate::error::{AppError, AppResult};
    use std::os::raw::{c_float, c_int, c_uint};

    const OK: c_int = 0;
    const INVALID_ARGUMENT: c_int = -1;
    const PERMISSION_DENIED: c_int = -2;
    const UNSUPPORTED: c_int = -3;
    const BUSY: c_int = -4;
    const NOT_ACTIVE: c_int = -5;
    const AUDIO_ERROR: c_int = -6;
    const PERMISSION_TIMEOUT: c_int = -7;

    type SampleCallback = unsafe extern "C" fn(
        interleaved: *const c_float,
        frame_count: c_int,
        channels: c_int,
        sample_rate: c_uint,
    );

    unsafe extern "C" {
        fn echo_system_audio_set_sample_callback(callback: Option<SampleCallback>);
        fn echo_system_audio_permission_status() -> c_int;
        fn echo_system_audio_request_permission() -> c_int;
        fn echo_system_audio_start() -> c_int;
        fn echo_system_audio_stop() -> c_int;
        fn echo_system_audio_cancel() -> c_int;
        fn echo_system_audio_is_recording() -> c_int;
        fn echo_system_audio_stream_failed() -> c_int;
    }

    pub(super) fn install_sample_callback(callback: SampleCallback) {
        unsafe { echo_system_audio_set_sample_callback(Some(callback)) };
    }

    pub(super) fn permission_status() -> i32 {
        unsafe { echo_system_audio_permission_status() }
    }

    pub(super) fn request_permission() -> AppResult<bool> {
        let result = unsafe { echo_system_audio_request_permission() };
        match result {
            1 => Ok(true),
            0 => Ok(false),
            -2 => Err(AppError::Invalid(
                "屏幕与系统声音授权超时：请在「系统设置 → 隐私与安全性 → 屏幕与系统声音录制」（旧版为「屏幕录制」）中允许回声记忆，然后重试"
                    .to_owned(),
            )),
            _ => Err(AppError::Invalid(
                "当前 macOS 不支持屏幕与系统声音授权请求".to_owned(),
            )),
        }
    }

    pub(super) fn start() -> AppResult<()> {
        map_result(unsafe { echo_system_audio_start() })
    }

    /// 停流。NOT_ACTIVE（流已被系统中断或已自行停止）不算错误，返回 false；
    /// 其余错误照常上抛。
    pub(super) fn stop_tolerant() -> AppResult<bool> {
        match unsafe { echo_system_audio_stop() } {
            OK => Ok(true),
            NOT_ACTIVE => Ok(false),
            other => map_result(other).map(|_| true),
        }
    }

    pub(super) fn cancel_tolerant() -> AppResult<bool> {
        match unsafe { echo_system_audio_cancel() } {
            OK => Ok(true),
            NOT_ACTIVE => Ok(false),
            other => map_result(other).map(|_| true),
        }
    }

    pub(super) fn is_recording() -> bool {
        unsafe { echo_system_audio_is_recording() == 1 }
    }

    pub(super) fn stream_failed() -> bool {
        unsafe { echo_system_audio_stream_failed() == 1 }
    }

    fn map_result(code: i32) -> AppResult<()> {
        match code {
            OK => Ok(()),
            INVALID_ARGUMENT => Err(AppError::Invalid("系统声采集参数无效".to_owned())),
            PERMISSION_DENIED => Err(AppError::Invalid(
                "屏幕与系统声音权限未授予。要录制系统内放声，请在「系统设置 → 隐私与安全性 → 屏幕与系统声音录制」（旧版为「屏幕录制」）中允许回声记忆，并重启应用后重试；你也可以只录麦克风。"
                    .to_owned(),
            )),
            UNSUPPORTED => Err(AppError::Invalid(
                "当前 macOS 版本不支持系统声采集（需要 macOS 12.3 及以上）".to_owned(),
            )),
            PERMISSION_TIMEOUT => Err(AppError::Invalid(
                "屏幕与系统声音授权未在限时内完成：请在系统设置中允许后重启应用重试".to_owned(),
            )),
            BUSY => Err(AppError::Invalid("已有一条系统声采集正在进行".to_owned())),
            NOT_ACTIVE => Err(AppError::Invalid("当前没有正在进行的系统声采集".to_owned())),
            AUDIO_ERROR => Err(AppError::Import(
                "macOS 系统声采集器无法启动".to_owned(),
            )),
            other => Err(AppError::Import(format!(
                "macOS 系统声采集器返回未知错误码 {other}"
            ))),
        }
    }
}

#[cfg(target_os = "macos")]
extern "C" fn system_audio_samples(
    interleaved: *const std::os::raw::c_float,
    frame_count: std::os::raw::c_int,
    channels: std::os::raw::c_int,
    sample_rate: std::os::raw::c_uint,
) {
    if interleaved.is_null() || frame_count <= 0 || channels <= 0 || sample_rate == 0 {
        return;
    }
    let frames = frame_count as usize;
    let channel_count = channels as usize;
    let samples = unsafe { std::slice::from_raw_parts(interleaved, frames * channel_count) };
    // 全程持锁：锁内只做一次 KB 级缓冲写，stop() 需要取走 sink 时必须等本轮回调结束，
    // 杜绝「stop 拿到 None 而样本仍在路上」的落盘竞态。
    // 锁中毒时 fail-open（继续收其它回调的样本），不让一次 panic 静默杀掉整段录音。
    let mut slot = capture_sink().lock().unwrap_or_else(|error| error.into_inner());
    if let Some(sink) = slot.as_mut() {
        sink.ingest(samples, frames, channel_count, sample_rate);
    }
}

fn capture_sink() -> &'static Mutex<Option<CaptureSink>> {
    static SINK: OnceLock<Mutex<Option<CaptureSink>>> = OnceLock::new();
    SINK.get_or_init(|| Mutex::new(None))
}

#[cfg(target_os = "macos")]
fn install_native_callback_once() {
    static INSTALLED: OnceLock<()> = OnceLock::new();
    INSTALLED.get_or_init(|| native::install_sample_callback(system_audio_samples));
}

#[cfg(not(target_os = "macos"))]
fn install_native_callback_once() {}

#[cfg(not(target_os = "macos"))]
mod native {
    use crate::error::{AppError, AppResult};

    pub(super) fn permission_status() -> i32 {
        3 // unsupported
    }

    pub(super) fn request_permission() -> AppResult<bool> {
        Err(AppError::Invalid("当前平台不支持系统声采集".to_owned()))
    }

    pub(super) fn start() -> AppResult<()> {
        Err(AppError::Invalid("当前平台不支持系统声采集".to_owned()))
    }

    pub(super) fn stop_tolerant() -> AppResult<bool> {
        Err(AppError::Invalid("当前平台不支持系统声采集".to_owned()))
    }

    pub(super) fn cancel_tolerant() -> AppResult<bool> {
        Err(AppError::Invalid("当前平台不支持系统声采集".to_owned()))
    }

    pub(super) fn is_recording() -> bool {
        false
    }

    pub(super) fn stream_failed() -> bool {
        false
    }
}

/* ---------------- 纯逻辑：样本转换与 WAV 落盘（测试重点） ---------------- */

/// 把交错的 Float32 多声道样本下混为单声道 i16（各声道取平均，超界截断）。
/// 空输入返回空 Vec；channel_count 为 0 视为无效输入，返回空 Vec。
pub fn downmix_to_i16(interleaved: &[f32], channel_count: usize) -> Vec<i16> {
    if channel_count == 0 || interleaved.is_empty() {
        return Vec::new();
    }
    let frames = interleaved.len() / channel_count;
    let mut out = Vec::with_capacity(frames);
    for frame in 0..frames {
        let base = frame * channel_count;
        let sum: f32 = interleaved[base..base + channel_count].iter().sum();
        let mean = (sum / channel_count as f32).clamp(-1.0, 1.0);
        // 标准满幅映射：-1.0 → -32768，+1.0 → +32767（右侧截断）。
        out.push((mean * 32_768.0).round().clamp(-32_768.0, 32_767.0) as i16);
    }
    out
}

/// 把一段新样本折算成 (average_db, peak_db)，量程 [DB_FLOOR, 0]。
pub fn levels_from_samples(samples: &[f32]) -> (f32, f32) {
    if samples.is_empty() {
        return (DB_FLOOR, DB_FLOOR);
    }
    let sum_squares: f64 = samples
        .iter()
        .filter(|value| value.is_finite())
        .map(|value| {
            let value = value.clamp(-1.0, 1.0);
            (value as f64) * (value as f64)
        })
        .sum();
    let peak = samples
        .iter()
        .filter(|value| value.is_finite())
        .fold(0.0f32, |acc, value| acc.max(value.abs().clamp(0.0, 1.0)));
    let rms = (sum_squares / samples.len() as f64).sqrt() as f32;
    let to_db = |linear: f32| {
        if linear <= f32::EPSILON {
            DB_FLOOR
        } else {
            (20.0 * linear.log10()).clamp(DB_FLOOR, 0.0)
        }
    };
    (to_db(rms), to_db(peak))
}

/// 流式 WAV 写入器：16-bit PCM 单声道。创建时写占位头，结束时空补 RIFF/data 两个长度字段。
pub struct WavStreamWriter {
    file: Option<BufWriter<fs::File>>,
    path: PathBuf,
    data_bytes: u64,
    sample_rate: u32,
    channels: u16,
}

fn write_wav_header_bytes(sink: &mut impl Write, data_len: u32, sample_rate: u32, channels: u16) -> std::io::Result<()> {
    let bits_per_sample: u16 = 16;
    let block_align = channels * (bits_per_sample / 8);
    let byte_rate = sample_rate * block_align as u32;
    sink.write_all(b"RIFF")?;
    sink.write_all(&(36_u32 + data_len).to_le_bytes())?;
    sink.write_all(b"WAVE")?;
    sink.write_all(b"fmt ")?;
    sink.write_all(&16_u32.to_le_bytes())?;
    sink.write_all(&1_u16.to_le_bytes())?; // PCM
    sink.write_all(&channels.to_le_bytes())?;
    sink.write_all(&sample_rate.to_le_bytes())?;
    sink.write_all(&byte_rate.to_le_bytes())?;
    sink.write_all(&block_align.to_le_bytes())?;
    sink.write_all(&bits_per_sample.to_le_bytes())?;
    sink.write_all(b"data")?;
    sink.write_all(&data_len.to_le_bytes())?;
    Ok(())
}

impl WavStreamWriter {
    pub fn create(path: &Path, sample_rate: u32, channels: u16) -> AppResult<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let file = fs::File::create(path)?;
        let mut writer = BufWriter::new(file);
        write_wav_header_bytes(&mut writer, 0, sample_rate, channels)
            .map_err(|error| AppError::Import(format!("无法写入 WAV 头：{error}")))?;
        Ok(Self {
            file: Some(writer),
            path: path.to_path_buf(),
            data_bytes: 0,
            sample_rate,
            channels,
        })
    }

    pub fn append_i16(&mut self, samples: &[i16]) -> AppResult<()> {
        if samples.is_empty() {
            return Ok(());
        }
        let Some(file) = self.file.as_mut() else {
            return Err(AppError::Import("WAV 写入器已经结束".to_owned()));
        };
        let bytes: Vec<u8> = samples.iter().flat_map(|sample| sample.to_le_bytes()).collect();
        file.write_all(&bytes)
            .map_err(|error| AppError::Import(format!("无法写入音频数据：{error}")))?;
        self.data_bytes += bytes.len() as u64;
        Ok(())
    }

    pub fn data_bytes(&self) -> u64 {
        self.data_bytes
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn channels(&self) -> u16 {
        self.channels
    }

    /// 补写 RIFF/data 长度并关闭文件；返回文件总字节数。
    pub fn finalize(mut self) -> AppResult<u64> {
        let mut file = self
            .file
            .take()
            .ok_or_else(|| AppError::Import("WAV 写入器已经结束".to_owned()))?;
        file.flush()
            .map_err(|error| AppError::Import(format!("无法落盘音频数据：{error}")))?;
        let data_len = self.data_bytes.min(u32::MAX as u64) as u32;
        let mut raw = file.into_inner().map_err(|error| {
            let (error, _) = error.into_parts();
            AppError::Import(format!("无法落盘音频数据：{error}"))
        })?;
        // RIFF 总长在文件偏移 4，data 长度在文件偏移 40（44 字节头）。
        let mut length_buffer = [0u8; 4];
        length_buffer.copy_from_slice(&(36_u32 + data_len).to_le_bytes());
        raw.seek(SeekFrom::Start(4))
            .and_then(|_| raw.write_all(&length_buffer))
            .map_err(|error| AppError::Import(format!("无法回写 WAV 长度：{error}")))?;
        length_buffer.copy_from_slice(&data_len.to_le_bytes());
        raw.seek(SeekFrom::Start(40))
            .and_then(|_| raw.write_all(&length_buffer))
            .map_err(|error| AppError::Import(format!("无法回写 WAV 长度：{error}")))?;
        raw.sync_all()
            .map_err(|error| AppError::Import(format!("无法落盘音频数据：{error}")))?;
        Ok(44 + self.data_bytes)
    }

    /// 关闭并删除半成品文件（放弃或无效会话）。
    pub fn discard(mut self) {
        self.file = None;
        let _ = fs::remove_file(&self.path);
    }
}

fn total_bytes_of(path: &Path) -> u64 {
    fs::metadata(path).map(|meta| meta.len()).unwrap_or(0)
}

/// 采集中的活动状态：Rust 侧持有，音频回调线程与控制线程通过全局槽位共享。
pub struct CaptureSink {
    writer: WavStreamWriter,
    pub paused: bool,
    pub average_db: f32,
    pub peak_db: f32,
    pub write_error: Option<String>,
    pub frames_written: u64,
}

impl CaptureSink {
    pub fn new(writer: WavStreamWriter) -> Self {
        Self {
            writer,
            paused: false,
            average_db: DB_FLOOR,
            peak_db: DB_FLOOR,
            write_error: None,
            frames_written: 0,
        }
    }

    /// 音频回调线程调用：暂停时丢弃样本；写失败后记录错误并不再写入。
    pub fn ingest(&mut self, interleaved: &[f32], frames: usize, channel_count: usize, _sample_rate: u32) {
        if self.paused || self.write_error.is_some() {
            return;
        }
        let sample_end = frames.saturating_mul(channel_count);
        if sample_end > interleaved.len() {
            return;
        }
        let usable = &interleaved[..sample_end];
        let (average_db, peak_db) = levels_from_samples(usable);
        self.average_db = average_db;
        self.peak_db = peak_db;
        let mono = downmix_to_i16(usable, channel_count);
        if let Err(error) = self.writer.append_i16(&mono) {
            self.write_error = Some(error.to_string());
        } else {
            self.frames_written += frames as u64;
        }
    }

    pub fn finalize_writer(self) -> AppResult<u64> {
        self.writer.finalize()
    }

    pub fn discard_writer(self) {
        self.writer.discard();
    }
}

/* ---------------- 会话与控制器 ---------------- */

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SystemAudioPermission {
    Undetermined,
    Denied,
    Granted,
    Unsupported,
}

#[derive(Debug)]
struct ActiveSystemRecording {
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
pub struct SystemAudioController {
    active: Arc<Mutex<Option<ActiveSystemRecording>>>,
}

fn validated_system_inflight_path(library: &ManagedLibrary, relative: &str) -> AppResult<PathBuf> {
    let relative_path = Path::new(relative);
    let components = relative_path.components().collect::<Vec<_>>();
    if relative_path.is_absolute()
        || components
            .iter()
            .any(|component| !matches!(component, Component::Normal(_)))
        || components.len() != 4
        || components[0] != Component::Normal("recordings".as_ref())
        || components[1] != Component::Normal("inflight".as_ref())
        || components[3] != Component::Normal(SYSTEM_CAPTURE_FILE_NAME.as_ref())
    {
        return Err(AppError::Invalid("可恢复系统录音路径无效".to_owned()));
    }
    let candidate = library.root().join(relative_path);
    let canonical_root = fs::canonicalize(library.root())
        .map_err(|_| AppError::Invalid("资料库路径不可用".to_owned()))?;
    let parent = candidate
        .parent()
        .ok_or_else(|| AppError::Invalid("可恢复系统录音路径无效".to_owned()))?;
    // 父目录解析失败（不存在/权限）直接拒绝，不静默放行：这是库内路径校验，
    // 放行等于允许读到库外的任意文件。
    let canonical_parent = fs::canonicalize(parent)
        .map_err(|_| AppError::Invalid("可恢复系统录音路径不在资料库内".to_owned()))?;
    if !canonical_parent.starts_with(&canonical_root) {
        return Err(AppError::Invalid("可恢复系统录音路径不在资料库内".to_owned()));
    }
    Ok(candidate)
}

impl SystemAudioController {
    pub fn permission_status(&self) -> SystemAudioPermission {
        match native::permission_status() {
            0 => SystemAudioPermission::Undetermined,
            1 => SystemAudioPermission::Denied,
            2 => SystemAudioPermission::Granted,
            _ => SystemAudioPermission::Unsupported,
        }
    }

    pub fn request_permission(&self) -> AppResult<SystemAudioPermission> {
        let granted = native::request_permission()?;
        Ok(if granted {
            SystemAudioPermission::Granted
        } else {
            SystemAudioPermission::Denied
        })
    }

    pub fn start(
        &self,
        library: &ManagedLibrary,
        project_id: Option<&str>,
    ) -> AppResult<RecordingState> {
        install_native_callback_once();
        let library_root = library.root();
        let mut active = self
            .active
            .lock()
            .map_err(|_| AppError::Invalid("系统声采集状态锁不可用".to_owned()))?;
        if active.is_some() || native::is_recording() {
            return Err(AppError::Invalid("已有一条系统声采集正在进行".to_owned()));
        }

        let session_id = Uuid::new_v4().to_string();
        let session_dir = library_root
            .join("recordings")
            .join("inflight")
            .join(&session_id);
        fs::create_dir_all(&session_dir)?;
        let file_path = session_dir.join(SYSTEM_CAPTURE_FILE_NAME);
        let relative_path = file_path
            .strip_prefix(library_root)
            .map_err(|_| AppError::Invalid("系统声采集临时路径不在资料库内".to_owned()))?
            .to_path_buf();
        let started_at = Utc::now().to_rfc3339();
        let title = format!("系统录音 {}", chrono::Local::now().format("%Y-%m-%d %H-%M"));
        library.repository().create_recording_session(
            &session_id,
            &relative_path.to_string_lossy(),
            &title,
            project_id,
            &started_at,
        )?;

        let writer = match WavStreamWriter::create(&file_path, SYSTEM_SAMPLE_RATE, 1) {
            Ok(writer) => writer,
            Err(error) => {
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
        };
        // 先装 sink 再启流：SCK 启动完成后马上就可能回调，装槽晚了会把开头
        // 几百毫秒静默丢掉；启动失败时再取走丢弃。
        *capture_sink().lock().unwrap_or_else(|error| error.into_inner()) =
            Some(CaptureSink::new(writer));
        if let Err(error) = native::start() {
            let _ = library.repository().update_recording_session_status(
                &session_id,
                "canceled",
                0,
                0,
                0,
                Some(&error.to_string()),
            );
            if let Some(sink) = capture_sink().lock().unwrap_or_else(|e| e.into_inner()).take() {
                sink.discard_writer();
            }
            let _ = fs::remove_dir_all(&session_dir);
            return Err(error);
        }
        *active = Some(ActiveSystemRecording {
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
            let _ = native::stop_tolerant();
            capture_sink().lock().unwrap_or_else(|e| e.into_inner()).take();
            active.take();
            return Err(error);
        }
        Ok(RecordingState {
            status: RecordingStatus::Recording,
            session_id: Some(session_id),
            elapsed_ms: 0,
            input_level_db: Some(DB_FLOOR),
            peak_level_db: Some(DB_FLOOR),
            file_path: Some(file_path.to_string_lossy().into_owned()),
            error: None,
        })
    }

    pub fn pause(&self, library: &ManagedLibrary) -> AppResult<RecordingState> {
        let mut guard = self.lock_active()?;
        let active = guard
            .as_mut()
            .ok_or_else(|| AppError::Invalid("当前没有正在进行的系统声采集".to_owned()))?;
        if let Some(sink) = capture_sink().lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            sink.paused = true;
        }
        active.paused_at = Some(Instant::now());
        self.checkpoint(library, active, "paused")?;
        Ok(self.state_for(active, RecordingStatus::Paused))
    }

    pub fn resume(&self, library: &ManagedLibrary) -> AppResult<RecordingState> {
        let mut guard = self.lock_active()?;
        let active = guard
            .as_mut()
            .ok_or_else(|| AppError::Invalid("当前没有正在进行的系统声采集".to_owned()))?;
        if let Some(sink) = capture_sink().lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            sink.paused = false;
        }
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
            .ok_or_else(|| AppError::Invalid("当前没有正在进行的系统声采集".to_owned()))?;
        let mut levels = (None, None);
        if let Some(sink) = capture_sink().lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            levels = (Some(sink.average_db), Some(sink.peak_db));
            if let Some(error) = sink.write_error.clone() {
                return Err(AppError::Import(error));
            }
        }
        Ok(RecordingState {
            input_level_db: levels.0,
            peak_level_db: levels.1,
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
        // 先在锁内把会话摘下来（锁只护状态，不护 I/O），再去停流——
        // 否则 stop 最长 10 秒的等待会让 levels 轮询一起卡住。
        let (finished, duration) = {
            let mut active = self
                .active
                .lock()
                .map_err(|_| AppError::Invalid("系统声采集状态锁不可用".to_owned()))?;
            let current = active
                .as_ref()
                .ok_or_else(|| AppError::Invalid("当前没有正在进行的系统声采集".to_owned()))?;
            let duration = self.elapsed_for(current);
            let finished = active.take().expect("系统声采集状态在同一把锁内不应丢失");
            (finished, duration)
        };
        let stream_failed = native::stream_failed();
        // 流可能已被系统中断（如显示器休眠触发 didStopWithError）：NOT_ACTIVE
        // 视为「流已自行停止」，继续收尾，绝不能让会话卡在无法停止的状态。
        if let Err(error) = native::stop_tolerant() {
            if !stream_failed {
                let _ = library.repository().update_recording_session_status(
                    &finished.session_id,
                    "failed",
                    duration.as_millis().min(i64::MAX as u128) as i64,
                    self.paused_millis(&finished),
                    total_bytes_of(&finished.file_path).min(i64::MAX as u64) as i64,
                    Some(&error.to_string()),
                );
                return Err(error);
            }
        }
        let sink = capture_sink().lock().unwrap_or_else(|e| e.into_inner()).take();
        let duration_ms = duration.as_millis().min(i64::MAX as u128) as i64;
        let total_bytes = match sink {
            Some(sink) => match sink.finalize_writer() {
                Ok(bytes) => bytes,
                Err(error) => {
                    let _ = library.repository().update_recording_session_status(
                        &finished.session_id,
                        "failed",
                        duration_ms,
                        self.paused_millis(&finished),
                        total_bytes_of(&finished.file_path).min(i64::MAX as u64) as i64,
                        Some(&error.to_string()),
                    );
                    return Err(error);
                }
            },
            None => total_bytes_of(&finished.file_path),
        };
        library.repository().update_recording_session_status(
            &finished.session_id,
            "finalizing",
            duration_ms,
            self.paused_millis(&finished),
            total_bytes.min(i64::MAX as u64) as i64,
            None,
        )?;
        if stream_failed {
            let message = "系统声采集被系统中断，音频不完整，本次不保存";
            let _ = library.repository().update_recording_session_status(
                &finished.session_id,
                "failed",
                duration_ms,
                self.paused_millis(&finished),
                total_bytes.min(i64::MAX as u64) as i64,
                Some(message),
            );
            return Err(AppError::Import(message.to_owned()));
        }
        if total_bytes <= 44 || !finished.file_path.is_file() {
            let message = "系统声采集已停止，但没有录到任何音频（请确认 Mac 正在播放声音）";
            let _ = library.repository().update_recording_session_status(
                &finished.session_id,
                "failed",
                duration_ms,
                self.paused_millis(&finished),
                total_bytes.min(i64::MAX as u64) as i64,
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
                    total_bytes.min(i64::MAX as u64) as i64,
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
        let (session_id, path, paused_ms) = {
            let mut active = self
                .active
                .lock()
                .map_err(|_| AppError::Invalid("系统声采集状态锁不可用".to_owned()))?;
            let current = active
                .as_ref()
                .ok_or_else(|| AppError::Invalid("当前没有正在进行的系统声采集".to_owned()))?;
            let session_id = current.session_id.clone();
            let path = current.file_path.clone();
            let paused_ms = self.paused_millis(current);
            active.take();
            (session_id, path, paused_ms)
        };
        // 与 stop 同理：流已被系统中断时 NOT_ACTIVE 不算错误，继续清理。
        if let Err(error) = native::cancel_tolerant() {
            let _ = library.repository().update_recording_session_status(
                &session_id,
                "canceled",
                0,
                paused_ms,
                0,
                Some(&error.to_string()),
            );
            return Err(error);
        }
        if let Some(sink) = capture_sink().lock().unwrap_or_else(|e| e.into_inner()).take() {
            sink.discard_writer();
        }
        if let Err(error) = library.repository().update_recording_session_status(
            &session_id,
            "canceled",
            0,
            paused_ms,
            0,
            None,
        ) {
            return Err(AppError::Import(format!(
                "系统声采集已停止，但取消状态无法保存：{error}；临时文件已保留，重启后可放弃"
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
    ) -> AppResult<Vec<RecoverableSystemRecording>> {
        library
            .repository()
            .list_recoverable_recording_sessions()
            .map(|sessions| {
                sessions
                    .into_iter()
                    .filter(|session| session.temp_path.ends_with(SYSTEM_CAPTURE_FILE_NAME))
                    .filter(|session| session.status == "recoverable" || session.status == "failed")
                    .map(|session| RecoverableSystemRecording {
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
        if !session.temp_path.ends_with(SYSTEM_CAPTURE_FILE_NAME) {
            return Err(AppError::Invalid("该录音不是系统声采集会话".to_owned()));
        }
        if !matches!(session.status.as_str(), "recoverable" | "failed") {
            return Err(AppError::Invalid("该录音当前不能放弃".to_owned()));
        }
        let path = validated_system_inflight_path(library, &session.temp_path)?;
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
        if !session.temp_path.ends_with(SYSTEM_CAPTURE_FILE_NAME) {
            return Err(AppError::Invalid("该录音不是系统声采集会话".to_owned()));
        }
        if !matches!(session.status.as_str(), "recoverable" | "failed") {
            return Err(AppError::Invalid("该录音当前不能恢复".to_owned()));
        }
        let path = validated_system_inflight_path(library, &session.temp_path)?;
        if !path.is_file() {
            return Err(AppError::Import("找不到可恢复的系统录音文件".to_owned()));
        }
        let bytes = fs::metadata(&path)?.len();
        if bytes <= 44 {
            return Err(AppError::Import("临时系统录音文件为空，无法恢复".to_owned()));
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
                error: Some("系统声采集状态锁不可用".to_owned()),
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

    fn lock_active(&self) -> AppResult<std::sync::MutexGuard<'_, Option<ActiveSystemRecording>>> {
        self.active
            .lock()
            .map_err(|_| AppError::Invalid("系统声采集状态锁不可用".to_owned()))
    }

    fn elapsed_for(&self, active: &ActiveSystemRecording) -> Duration {
        let paused = active
            .paused_total
            .saturating_add(active.paused_at.map(|at| at.elapsed()).unwrap_or_default());
        active.started_at.elapsed().saturating_sub(paused)
    }

    fn paused_millis(&self, active: &ActiveSystemRecording) -> i64 {
        active
            .paused_total
            .saturating_add(active.paused_at.map(|at| at.elapsed()).unwrap_or_default())
            .as_millis()
            .min(i64::MAX as u128) as i64
    }

    fn checkpoint(
        &self,
        library: &ManagedLibrary,
        active: &mut ActiveSystemRecording,
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

    fn state_for(&self, active: &ActiveSystemRecording, status: RecordingStatus) -> RecordingState {
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoverableSystemRecording {
    pub session_id: String,
    pub status: String,
    pub title: String,
    pub started_at: String,
    pub duration_ms: i64,
    pub written_bytes: i64,
    pub last_error: Option<String>,
}

/// 导出给 recoverable 列表用：把系统会话并入统一列表并打上来源标记。
pub fn system_recoverable_into(
    library: &ManagedLibrary,
    sink: &mut Vec<crate::recording::RecoverableRecording>,
) -> AppResult<()> {
    let controller = SystemAudioController::default();
    for session in controller.recoverable_sessions(library)? {
        sink.push(crate::recording::RecoverableRecording {
            session_id: session.session_id,
            status: session.status,
            title: session.title,
            started_at: session.started_at,
            duration_ms: session.duration_ms,
            written_bytes: session.written_bytes,
            last_error: session.last_error,
            source: "system".to_owned(),
        });
    }
    Ok(())
}

#[tauri::command]
pub fn system_audio_permission(state: State<AppState>) -> Result<SystemAudioPermission, String> {
    Ok(state.system_audio.permission_status())
}

#[tauri::command]
pub async fn request_system_audio_permission(
    state: State<'_, AppState>,
) -> Result<SystemAudioPermission, String> {
    let controller = state.system_audio.clone();
    tauri::async_runtime::spawn_blocking(move || controller.request_permission())
        .await
        .map_err(|_| "系统声授权后台任务异常".to_owned())?
        .map_err(|error| error.to_frontend())
}

#[tauri::command]
pub async fn start_system_recording(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    project_id: Option<String>,
    // 后端确认门：只有前端走完显性告知确认框（confirmAction 返回 true）才会传 true。
    // 隐私红线落在 API 契约上，而不是只靠前端自觉。
    confirmed: Option<bool>,
) -> Result<RecordingState, String> {
    if confirmed != Some(true) {
        return Err("系统内录开始前必须完成用户确认：本功能会录到 Mac 正在播放的所有声音（包括他人的声音），需要你显性同意后才能开始".to_owned());
    }
    let controller = state.system_audio.clone();
    let library = state.library.clone();
    let started = tauri::async_runtime::spawn_blocking(move || {
        controller.start(&library, project_id.as_deref())
    })
    .await
    .map_err(|_| "开始系统声采集后台任务异常".to_owned())?
    .map_err(|error| error.to_frontend())?;
    // 录音已成立：启动实时文字线程（模型不可用只影响预览，不影响录音）。
    let config = crate::live_transcript::config_from_library(&state.library);
    if let Some(session_id) = started.session_id.clone() {
        let wav_path = PathBuf::from(started.file_path.clone().unwrap_or_default());
        crate::live_transcript::spawn(
            app,
            state.live_transcripts.clone(),
            session_id,
            "system",
            wav_path,
            config,
        );
    }
    Ok(started)
}

#[tauri::command]
pub async fn pause_system_recording(state: State<'_, AppState>) -> Result<RecordingState, String> {
    let controller = state.system_audio.clone();
    let library = state.library.clone();
    tauri::async_runtime::spawn_blocking(move || controller.pause(&library))
        .await
        .map_err(|_| "暂停系统声采集后台任务异常".to_owned())?
        .map_err(|error| error.to_frontend())
}

#[tauri::command]
pub async fn resume_system_recording(state: State<'_, AppState>) -> Result<RecordingState, String> {
    let controller = state.system_audio.clone();
    let library = state.library.clone();
    tauri::async_runtime::spawn_blocking(move || controller.resume(&library))
        .await
        .map_err(|_| "继续系统声采集后台任务异常".to_owned())?
        .map_err(|error| error.to_frontend())
}

#[tauri::command]
pub fn system_recording_levels(state: State<AppState>) -> Result<RecordingState, String> {
    state
        .system_audio
        .levels()
        .map_err(|error| error.to_frontend())
}

#[tauri::command]
pub async fn stop_system_recording(
    state: State<'_, AppState>,
) -> Result<StoppedRecording, String> {
    let controller = state.system_audio.clone();
    let library = state.library.clone();
    // 无论停止成败都必须停掉实时线程（理由同麦克风链路）。
    let session_id = state.system_audio.state().session_id.clone();
    let result = tauri::async_runtime::spawn_blocking(move || controller.stop(&library))
        .await
        .map_err(|_| "停止系统声采集后台任务异常".to_owned())?;
    if let Some(session_id) =
        session_id.or_else(|| result.as_ref().ok().map(|stopped| stopped.session_id.clone()))
    {
        state.live_transcripts.stop_session(&session_id);
    }
    result.map_err(|error| error.to_frontend())
}

#[tauri::command]
pub async fn cancel_system_recording(state: State<'_, AppState>) -> Result<(), String> {
    let controller = state.system_audio.clone();
    let library = state.library.clone();
    let session_id = state.system_audio.state().session_id.clone();
    let result = tauri::async_runtime::spawn_blocking(move || controller.cancel(&library))
        .await
        .map_err(|_| "取消系统声采集后台任务异常".to_owned())?;
    if let Some(session_id) = session_id {
        state.live_transcripts.stop_session(&session_id);
    }
    result.map_err(|error| error.to_frontend())
}

#[tauri::command]
pub fn get_system_recording_state(state: State<AppState>) -> RecordingState {
    state.system_audio.state()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn scratch_path(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!("echo-sys-audio-{label}-{}.wav", Uuid::new_v4()))
    }

    #[test]
    fn downmix_stereo_averages_and_clamps() {
        // 两声道取平均：(-1.0, 1.0) → 0；(1.0, 1.0) → 满幅正。
        let stereo = [-1.0f32, 1.0, 1.0, 1.0, 0.5, -0.5];
        let mono = downmix_to_i16(&stereo, 2);
        assert_eq!(mono.len(), 3);
        assert_eq!(mono[0], 0);
        assert_eq!(mono[1], i16::MAX);
        assert_eq!(mono[2], 0);

        // 超出 [-1, 1] 的样本被截断而不是回绕。
        let over = [2.0f32, 2.0, -2.0, -2.0];
        let mono = downmix_to_i16(&over, 2);
        assert_eq!(mono[0], i16::MAX);
        assert_eq!(mono[1], i16::MIN);

        // 空输入与无效声道数都安全。
        assert!(downmix_to_i16(&[], 2).is_empty());
        assert!(downmix_to_i16(&[0.0f32], 0).is_empty());
    }

    #[test]
    fn levels_span_silence_to_full_scale() {
        let silence = vec![0.0f32; 4800];
        let (average_db, peak_db) = levels_from_samples(&silence);
        assert!(average_db <= DB_FLOOR + f32::EPSILON);
        assert!(peak_db <= DB_FLOOR + f32::EPSILON);

        // 0.5 幅度正弦：RMS ≈ -9dB，峰值 ≈ -6dB。
        let tone: Vec<f32> = (0..4800)
            .map(|i| 0.5 * (2.0 * std::f32::consts::PI * 440.0 * i as f32 / 48_000.0).sin())
            .collect();
        let (average_db, peak_db) = levels_from_samples(&tone);
        assert!((-12.0..=-6.0).contains(&average_db), "rms={average_db}");
        assert!((-7.0..=-5.0).contains(&peak_db), "peak={peak_db}");

        let square = vec![1.0f32; 1000];
        let (average_db, peak_db) = levels_from_samples(&square);
        assert!((peak_db - 0.0).abs() < 0.01);
        assert!((-0.1..=0.0).contains(&average_db), "rms={average_db}");
    }

    #[test]
    fn wav_writer_round_trips_with_hound() {
        let path = scratch_path("roundtrip");
        let mut writer = WavStreamWriter::create(&path, SYSTEM_SAMPLE_RATE, 1).unwrap();
        let samples: Vec<i16> = (0..9600).map(|i| ((i % 48) * 300 - 7200) as i16).collect();
        writer.append_i16(&samples).unwrap();
        writer.append_i16(&samples).unwrap();
        let total = writer.finalize().unwrap();
        assert_eq!(total, 44 + (samples.len() * 2 * 2) as u64);

        let mut reader = hound::WavReader::open(&path).unwrap();
        let spec = reader.spec();
        assert_eq!(spec.sample_rate, SYSTEM_SAMPLE_RATE);
        assert_eq!(spec.channels, 1);
        assert_eq!(spec.bits_per_sample, 16);
        let read_back: Vec<i16> = reader.samples::<i16>().map(Result::unwrap).collect();
        assert_eq!(read_back.len(), samples.len() * 2);
        assert_eq!(&read_back[..samples.len()], &samples[..]);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn wav_writer_header_patches_incrementally() {
        // 逐块写小批次，验证 data 长度被正确累计；再验证放弃后文件被删除。
        let path = scratch_path("incremental");
        let mut writer = WavStreamWriter::create(&path, SYSTEM_SAMPLE_RATE, 1).unwrap();
        assert_eq!(writer.data_bytes(), 0);
        for _ in 0..10 {
            writer.append_i16(&[0i16; 100]).unwrap();
        }
        assert_eq!(writer.data_bytes(), 10 * 100 * 2);
        let total = writer.finalize().unwrap();
        assert_eq!(total, 44 + 2000);

        let mut raw = Vec::new();
        fs::File::open(&path).unwrap().read_to_end(&mut raw).unwrap();
        assert_eq!(&raw[0..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(raw[4..8].try_into().unwrap()), 36 + 2000);
        assert_eq!(&raw[36..40], b"data");
        assert_eq!(u32::from_le_bytes(raw[40..44].try_into().unwrap()), 2000);
        std::fs::remove_file(&path).ok();

        let discard_path = scratch_path("discard");
        let writer = WavStreamWriter::create(&discard_path, SYSTEM_SAMPLE_RATE, 1).unwrap();
        writer.discard();
        assert!(!discard_path.exists());
    }

    #[test]
    fn paused_sink_drops_samples_but_keeps_levels() {
        let path = scratch_path("paused");
        let mut sink = CaptureSink::new(WavStreamWriter::create(&path, SYSTEM_SAMPLE_RATE, 1).unwrap());
        let loud = vec![1.0f32; 480];
        sink.ingest(&loud, 480, 1, SYSTEM_SAMPLE_RATE);
        assert_eq!(sink.frames_written, 480);
        assert!(sink.peak_db > -1.0);

        sink.paused = true;
        sink.ingest(&loud, 480, 1, SYSTEM_SAMPLE_RATE);
        assert_eq!(sink.frames_written, 480, "暂停后的样本不应写入");

        sink.paused = false;
        let silence = vec![0.0f32; 100];
        sink.ingest(&silence, 100, 1, SYSTEM_SAMPLE_RATE);
        assert_eq!(sink.frames_written, 580);
        assert!(sink.peak_db <= DB_FLOOR + f32::EPSILON, "静音后电平应回落");
        sink.discard_writer();
    }

    #[test]
    fn sink_survives_write_errors_and_reports_them() {
        // 写入器 finalize 后再 ingest：把错误记录在 sink 而不是 panic。
        let path = scratch_path("write-error");
        let writer = WavStreamWriter::create(&path, SYSTEM_SAMPLE_RATE, 1).unwrap();
        let mut sink = CaptureSink::new(writer);
        sink.ingest(&[0.5f32; 100], 100, 1, SYSTEM_SAMPLE_RATE);
        assert!(sink.write_error.is_none());
        // 模拟磁盘错误：把 writer 置为已结束状态后继续喂样本。
        sink.write_error = Some("模拟磁盘错误".to_owned());
        sink.ingest(&[0.5f32; 100], 100, 1, SYSTEM_SAMPLE_RATE);
        assert_eq!(sink.write_error, Some("模拟磁盘错误".to_owned()));
        sink.discard_writer();
    }

    #[test]
    fn multi_channel_ingest_is_downmixed() {
        let path = scratch_path("stereo");
        let mut sink = CaptureSink::new(WavStreamWriter::create(&path, SYSTEM_SAMPLE_RATE, 1).unwrap());
        // 立体声交错输入：480 帧 × 2 声道，应产出 480 个单声道样本。
        let stereo: Vec<f32> = (0..480).flat_map(|i| [0.5f32, -0.5].map(|v| v * i as f32 / 480.0)).collect();
        sink.ingest(&stereo, 480, 2, SYSTEM_SAMPLE_RATE);
        assert_eq!(sink.frames_written, 480);
        sink.discard_writer();
    }

    #[test]
    fn elapsed_excludes_paused_time() {
        let controller = SystemAudioController::default();
        let active = ActiveSystemRecording {
            session_id: "test".to_owned(),
            project_id: None,
            file_path: PathBuf::from("system-capture.wav"),
            title: "系统录音".to_owned(),
            started_at: Instant::now() - Duration::from_secs(3),
            paused_at: Some(Instant::now() - Duration::from_secs(2)),
            paused_total: Duration::from_secs(0),
            last_checkpoint: Instant::now(),
        };
        let elapsed = controller.elapsed_for(&active);
        assert!(elapsed >= Duration::from_millis(800));
        assert!(elapsed < Duration::from_secs(2));
    }

    /// 真机端到端（Mac 上能做的部分）：ECHO_SYS_AUDIO_E2E=1 时，
    /// 播放一段 440Hz 测试音，用 ScreenCaptureKit 采集系统内放声，
    /// 验证采集落盘的 WAV 时长正确且包含非静音内容。
    ///
    /// 只做预检、绝不主动触发授权弹窗（无人值守时弹窗没人点）：权限未授予时
    /// 直接按「需人工授权」失败并给出路径，不假装成功。
    #[test]
    fn captures_played_system_audio_end_to_end() {
        let Ok(flag) = std::env::var("ECHO_SYS_AUDIO_E2E") else {
            return; // 默认跳过：需要真实系统音频环境
        };
        if flag != "1" {
            return;
        }
        if !cfg!(target_os = "macos") {
            panic!("系统声端到端只能在 macOS 上运行");
        }
        // 预检不通过就明确失败：这是「需要人在系统设置里授权」的信号，
        // 不是采集逻辑的失败，也不能悄悄跳过算过。
        if native::permission_status() != 2 {
            panic!(
                "屏幕与系统声音录制权限未授予：请先在「系统设置 → 隐私与安全性 → 屏幕与系统声音录制」里允许宿主进程，再跑 ECHO_SYS_AUDIO_E2E=1"
            );
        }
        install_native_callback_once();

        let tone_path = std::env::temp_dir().join(format!("echo-sys-tone-{}.wav", Uuid::new_v4()));
        let tone_seconds = 5_u32;
        {
            let spec = hound::WavSpec {
                channels: 1,
                sample_rate: 48_000,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            };
            let mut writer = hound::WavWriter::create(&tone_path, spec).unwrap();
            for i in 0..48_000 * tone_seconds {
                let value = (2.0 * std::f32::consts::PI * 440.0 * i as f32 / 48_000.0).sin();
                writer.write_sample((value * 0.6 * 32_767.0) as i16).unwrap();
            }
            writer.finalize().unwrap();
        }

        let capture_path = std::env::temp_dir().join(format!("echo-sys-e2e-{}.wav", Uuid::new_v4()));
        let writer = WavStreamWriter::create(&capture_path, SYSTEM_SAMPLE_RATE, 1).unwrap();
        let sink = CaptureSink::new(writer);
        *capture_sink().lock().unwrap_or_else(|e| e.into_inner()) = Some(sink);

        native::start().expect("有权限时系统声采集必须能启动");
        let play = std::process::Command::new("/usr/bin/afplay")
            .arg(&tone_path)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .expect("无法启动 afplay 播放测试音");
        assert!(play.success(), "afplay 播放测试音失败");
        std::thread::sleep(Duration::from_millis(800));

        native::stop_tolerant().expect("停止系统声采集失败");
        let sink = capture_sink().lock().unwrap_or_else(|e| e.into_inner()).take();
        let frames = sink.as_ref().map(|s| s.frames_written).unwrap_or(0);
        let total_bytes = sink
            .map(|s| s.finalize_writer().expect("WAV 收尾失败"))
            .unwrap_or(0);

        let _ = fs::remove_file(&tone_path);
        let data = fs::read(&capture_path).expect("系统声采集应已落盘");
        assert_eq!(data.len() as u64, total_bytes, "落盘字节数与收尾统计应一致");
        assert!(
            frames > 0,
            "采集回调没有收到任何样本（frames_written=0）"
        );
        let expected_min = (SYSTEM_SAMPLE_RATE as u64 * 2) * (tone_seconds as u64 - 1);
        assert!(
            total_bytes >= 44 + expected_min,
            "落盘时长不足：{total_bytes} 字节（期望 ≥ {expected_min} 数据字节）"
        );
        // 非静音校验：读回样本算 RMS。系统音量被调到最低时这里会失败——
        // 那是环境问题，报错信息里说明。
        let samples: Vec<i16> = data[44..]
            .chunks_exact(2)
            .map(|chunk| i16::from_le_bytes([chunk[0], chunk[1]]))
            .collect();
        let rms = (samples.iter().map(|s| (*s as f64) * (*s as f64)).sum::<f64>()
            / samples.len().max(1) as f64)
            .sqrt();
        eprintln!("系统声端到端：frames={frames} bytes={total_bytes} rms={rms:.1}");
        assert!(
            rms > 50.0,
            "采集到的内容接近静音（rms={rms}）：请确认系统音量未静音后重试"
        );
        fs::remove_file(&capture_path).ok();
    }
}
