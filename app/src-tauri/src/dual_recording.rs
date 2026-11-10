//! 双路录音（v1.1 块8）：麦克风 + 系统内放声按同一时间轴合并。
//!
//! 两路各自独立采集落盘（各自的会话行、各自的崩溃恢复语义不变），停止时
//! 封存两路 WAV，按「系统声启动 − 麦克风启动」的实测墙钟偏移对齐，合成
//! 立体声 WAV（左=麦克风、右=系统声，**不预混音**），走主会话（麦克风）入库。
//! 偏移毫秒写进 recording_sessions.alignment_offset_ms——对齐误差可见、可测。
//!
//! 已知限制（如实记录）：暂停/恢复由编排器同时驱动两路，两路各自的暂停
//! 时间差为亚秒级操作延迟，计入对齐误差；转写管线的预处理会把立体声混成
//! 单声道（既有行为），两路内容在转写层重叠——文件层面始终保留分声道。

use crate::error::{AppError, AppResult};
use crate::recording::{CapturedRecording, RecordingController, StoppedRecording};
use crate::state::AppState;
use crate::system_audio::SystemAudioController;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DualStoppedRecording {
    #[serde(flatten)]
    pub stopped: StoppedRecording,
    /// 两路启动的实测偏移（毫秒，系统声相对麦克风；负值=系统声先启动）。
    pub alignment_offset_ms: i64,
    /// 合并后副会话（系统声）的会话 id，供对账。
    pub system_session_id: String,
}

/// 双路对齐偏移的 sanity 上限（毫秒）：会话创建墙钟差超过它视为时钟
/// 异常（跳变/NTP），拒绝合并而不是按偏移补出巨型静音。
pub const MAX_ALIGNMENT_OFFSET_MS: i64 = 10_000;

#[derive(Debug, Clone, Default)]
pub struct DualRecordingController {
    active: Arc<Mutex<Option<DualActive>>>,
}

/// 双路会话的活跃态（对命令层可见：is_some() 即双路在录）。
#[derive(Debug, Clone)]
pub struct DualActive {
    pub mic_session_id: String,
    pub system_session_id: String,
}

/// 纯函数：按偏移对齐两路单声道样本为立体声帧（左=麦克风，右=系统声）。
/// offset_ms = 系统声启动 − 麦克风启动（墙钟毫秒，48kHz 下每毫秒 48 样本）：
/// 正值=系统声晚启（右声道先补静音），负值=系统声先启（左声道补静音）。
/// 总长取两路在统一时间轴上覆盖的并集。
pub fn merge_dual_samples(mic: &[i16], system: &[i16], offset_ms: i64) -> Vec<(i16, i16)> {
    let offset_samples = offset_ms * 48;
    if offset_samples >= 0 {
        let system_start = offset_samples as usize;
        let total = mic.len().max(system_start.saturating_add(system.len()));
        (0..total)
            .map(|index| {
                let left = mic.get(index).copied().unwrap_or(0);
                let right = if index >= system_start {
                    system.get(index - system_start).copied().unwrap_or(0)
                } else {
                    0
                };
                (left, right)
            })
            .collect()
    } else {
        let mic_start = (-offset_samples) as usize;
        let total = system.len().max(mic_start.saturating_add(mic.len()));
        (0..total)
            .map(|index| {
                let left = if index >= mic_start {
                    mic.get(index - mic_start).copied().unwrap_or(0)
                } else {
                    0
                };
                let right = system.get(index).copied().unwrap_or(0);
                (left, right)
            })
            .collect()
    }
}

/// 合并两路 WAV 为立体声文件（流式逐帧写出，不整段驻留内存——1 小时双路
/// 全量驻留约 1.4GB，8GB 机器有风险）；返回输出文件总字节数。
pub fn merge_dual_wav_files(
    mic_path: &std::path::Path,
    system_path: &std::path::Path,
    offset_ms: i64,
    output_path: &std::path::Path,
) -> AppResult<u64> {
    let mut mic = hound::WavReader::open(mic_path)
        .map_err(|error| AppError::Import(format!("无法读取麦克风录音：{error}")))?;
    {
        let spec = mic.spec();
        if spec.sample_rate != 48_000 || spec.channels != 1 || spec.bits_per_sample != 16 {
            return Err(AppError::Import(format!(
                "麦克风录音格式异常（{rate}Hz/{channels}ch/{bits}bit）",
                rate = spec.sample_rate,
                channels = spec.channels,
                bits = spec.bits_per_sample
            )));
        }
    }
    let mut system = hound::WavReader::open(system_path)
        .map_err(|error| AppError::Import(format!("无法读取系统声录音：{error}")))?;
    {
        let spec = system.spec();
        if spec.sample_rate != 48_000 || spec.channels != 1 || spec.bits_per_sample != 16 {
            return Err(AppError::Import(format!(
                "系统声录音格式异常（{rate}Hz/{channels}ch/{bits}bit）",
                rate = spec.sample_rate,
                channels = spec.channels,
                bits = spec.bits_per_sample
            )));
        }
    }
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: 48_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(output_path, spec)
        .map_err(|error| AppError::Import(format!("无法创建合并音频：{error}")))?;

    // 48kHz：每毫秒 48 样本。offset 正=系统声晚启，负=系统声先启。
    let offset_samples = offset_ms * 48;
    let system_start = offset_samples.max(0) as u64;
    let mic_start = (-offset_samples).max(0) as u64;
    let mic_frames = mic.duration() as u64;
    let system_frames = system.duration() as u64;
    let total_frames = if offset_samples >= 0 {
        mic_frames.max(system_start.saturating_add(system_frames))
    } else {
        system_frames.max(mic_start.saturating_add(mic_frames))
    };

    let mut mic_samples = mic.samples::<i16>();
    let mut system_samples = system.samples::<i16>();
    let mut next_mic: Option<i16> = read_next_sample(&mut mic_samples)?;
    let mut next_system: Option<i16> = read_next_sample(&mut system_samples)?;
    for frame in 0..total_frames {
        let left = if frame >= mic_start {
            let value = next_mic.take().unwrap_or(0);
            next_mic = read_next_sample(&mut mic_samples)?;
            value
        } else {
            0
        };
        let right = if frame >= system_start {
            let value = next_system.take().unwrap_or(0);
            next_system = read_next_sample(&mut system_samples)?;
            value
        } else {
            0
        };
        writer
            .write_sample(left)
            .and_then(|_| writer.write_sample(right))
            .map_err(|error| AppError::Import(format!("合并写入失败：{error}")))?;
    }
    writer
        .finalize()
        .map_err(|error| AppError::Import(format!("合并收尾失败：{error}")))?;
    std::fs::metadata(output_path)
        .map(|meta| meta.len())
        .map_err(|error| AppError::Import(format!("合并文件不可读：{error}")))
}

fn session_started_ms(session_started_at: &str) -> AppResult<i64> {
    DateTime::parse_from_rfc3339(session_started_at)
        .map(|time| time.with_timezone(&Utc).timestamp_millis())
        .map_err(|error| AppError::Invalid(format!("会话开始时间无法解析：{error}")))
}

/// 流式合并的取下一个样本（读错误如实上抛，不静默补零）。
fn read_next_sample<R: std::io::Read>(
    iter: &mut hound::WavSamples<'_, R, i16>,
) -> AppResult<Option<i16>> {
    match iter.next() {
        Some(Ok(value)) => Ok(Some(value)),
        Some(Err(error)) => Err(AppError::Import(format!("合并读取失败：{error}"))),
        None => Ok(None),
    }
}

impl DualRecordingController {
    /// 是否有双路会话在进行（命令层据此分发 stop/pause/cancel）。
    pub fn state(&self) -> Option<DualActive> {
        self.active
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    /// 跨源互斥检查（块5 挂账收口）：任一路在录、或双路在录时，拒绝新的开始。
    pub fn ensure_no_capture_active(state: &AppState) -> AppResult<()> {
        if self_active(&state.recording) || self_active_system(&state.system_audio) {
            return Err(AppError::Invalid("已有一条录音正在进行".to_owned()));
        }
        if state.dual.state().is_some() {
            return Err(AppError::Invalid("已有一条双路录音正在进行".to_owned()));
        }
        Ok(())
    }

    pub fn start(
        &self,
        state: &AppState,
        project_id: Option<&str>,
    ) -> AppResult<crate::recording::RecordingState> {
        Self::ensure_no_capture_active(state)?;
        // 主路（麦克风）先起：副路失败时主路整体回滚，绝不静默降级成单路。
        // 后段任何失败同样回滚两路——否则两路在录而 UI 认为启动失败，
        // 用户以为没在录音（隐私红线不允许静默采集）。
        let mic = state.recording.start(&state.library, project_id)?;
        let mic_session_id = match mic.session_id.clone() {
            Some(id) => id,
            None => {
                let _ = state.recording.cancel(&state.library);
                return Err(AppError::Invalid("麦克风会话启动异常".to_owned()));
            }
        };
        let system = match state.system_audio.start(&state.library, project_id) {
            Ok(system) => system,
            Err(error) => {
                let _ = state.recording.cancel(&state.library);
                return Err(AppError::Invalid(format!(
                    "双路录音需要两路都就绪：系统声采集启动失败（{error}），麦克风已一并停止"
                )));
            }
        };
        let system_session_id = match system.session_id.clone() {
            Some(id) => id,
            None => {
                let _ = state.recording.cancel(&state.library);
                let _ = state.system_audio.cancel(&state.library);
                return Err(AppError::Invalid("系统声会话启动异常".to_owned()));
            }
        };
        if let Err(error) = state
            .library
            .repository()
            .set_paired_session(&mic_session_id, &system_session_id)
        {
            let _ = state.recording.cancel(&state.library);
            let _ = state.system_audio.cancel(&state.library);
            return Err(AppError::Invalid(format!(
                "双路会话登记失败，两路已全部停止：{error}"
            )));
        }
        *self.active.lock().unwrap_or_else(|error| error.into_inner()) = Some(DualActive {
            mic_session_id: mic_session_id.clone(),
            system_session_id: system_session_id.clone(),
        });
        Ok(mic)
    }

    pub fn pause(&self, state: &AppState) -> AppResult<crate::recording::RecordingState> {
        self.require_active()?;
        // 系统声先暂停；麦克风暂停失败时把系统声恢复回去——两路必须同拍，
        // 半暂停状态会让合并时间轴从此错位。返回麦克风状态（前端以它为准）。
        let _ = state.system_audio.pause(&state.library)?;
        let mic = state.recording.pause(&state.library);
        if let Err(error) = &mic {
            let _ = state.system_audio.resume(&state.library);
            return Err(AppError::Invalid(format!(
                "双路暂停失败（系统声已恢复）：{error}"
            )));
        }
        Ok(mic.unwrap())
    }

    pub fn resume(&self, state: &AppState) -> AppResult<crate::recording::RecordingState> {
        self.require_active()?;
        // 麦克风先恢复；系统声恢复失败时把麦克风暂停回去。
        let mic = state.recording.resume(&state.library)?;
        if let Err(error) = state.system_audio.resume(&state.library) {
            let _ = state.recording.pause(&state.library);
            return Err(AppError::Invalid(format!(
                "双路恢复失败（麦克风已退回暂停）：{error}"
            )));
        }
        Ok(mic)
    }

    pub fn stop(&self, state: &AppState) -> AppResult<DualStoppedRecording> {
        let _active_gate = self.require_active()?;
        let stop_outcome = self.stop_inner(state);
        // 任何中途失败：双路活跃态必须清掉（否则重试 stop 直接被门禁拒绝，
        // 会话卡死到重启）；已封存但未入库的会话标 failed 留给恢复列表。
        if stop_outcome.is_err() {
            *self.active.lock().unwrap_or_else(|error| error.into_inner()) = None;
        }
        stop_outcome
    }

    fn stop_inner(&self, state: &AppState) -> AppResult<DualStoppedRecording> {
        let repository = state.library.repository();
        // 1) 两路分别封存（不入库）。会话 id 以封存产物为准（require_active
        //    只做门禁，两路状态由各自控制器锁保证一致）。
        let mic = match state.recording.stop_capture_only(&state.library) {
            Ok(mic) => mic,
            Err(error) => return Err(error),
        };
        let system = match state.system_audio.stop_capture_only(&state.library) {
            Ok(system) => system,
            Err(error) => {
                // mic 已封存未入库：标 failed 让恢复列表接手。
                let _ = repository.update_recording_session_status(
                    &mic.session_id,
                    "failed",
                    mic.duration_ms,
                    mic.paused_ms,
                    mic.bytes.min(i64::MAX as u64) as i64,
                    Some("双路停止中断：系统声封存失败，麦克风音频可恢复"),
                );
                return Err(error);
            }
        };
        // 2) 对齐偏移 = 系统声启动 − 麦克风启动（会话创建墙钟差；含两路
        //    建流延迟差，为真实对齐误差的近似下界）。加 sanity 上限：
        //    异常巨值（时钟跳变）会让按偏移补静音的合并直接吃爆内存。
        let mic_started_at = repository
            .get_recording_session(&mic.session_id)?
            .started_at;
        let system_started_at = repository
            .get_recording_session(&system.session_id)?
            .started_at;
        let offset_ms = session_started_ms(&system_started_at)?
            - session_started_ms(&mic_started_at)?;
        if offset_ms.abs() > MAX_ALIGNMENT_OFFSET_MS {
            let _ = repository.update_recording_session_status(
                &mic.session_id,
                "failed",
                mic.duration_ms,
                mic.paused_ms,
                mic.bytes.min(i64::MAX as u64) as i64,
                Some(&format!(
                    "双路对齐偏移异常（{offset_ms}ms），合并已中止；两路音频可分别恢复"
                )),
            );
            let _ = repository.update_recording_session_status(
                &system.session_id,
                "failed",
                system.duration_ms,
                system.paused_ms,
                system.bytes.min(i64::MAX as u64) as i64,
                Some("双路对齐偏移异常，合并已中止；系统声音频可恢复"),
            );
            return Err(AppError::Invalid(format!(
                "双路对齐偏移异常（{offset_ms}ms），本次不合并；两路音频已转入恢复列表"
            )));
        }
        // 3) 合并立体声（左=麦克风，右=系统声；流式写出，不整段驻留内存）。
        let merged_path = mic
            .file_path
            .parent()
            .map(|parent| parent.join("merged.wav"))
            .ok_or_else(|| AppError::Invalid("录音临时目录异常".to_owned()))?;
        let merged_bytes = match merge_dual_wav_files(
            &mic.file_path,
            &system.file_path,
            offset_ms,
            &merged_path,
        ) {
            Ok(bytes) => bytes,
            Err(error) => {
                let _ = repository.update_recording_session_status(
                    &mic.session_id,
                    "failed",
                    mic.duration_ms,
                    mic.paused_ms,
                    mic.bytes.min(i64::MAX as u64) as i64,
                    Some(&format!("双路合并失败：{error}；两路音频可分别恢复")),
                );
                let _ = repository.update_recording_session_status(
                    &system.session_id,
                    "failed",
                    system.duration_ms,
                    system.paused_ms,
                    system.bytes.min(i64::MAX as u64) as i64,
                    Some("双路合并失败；系统声音频可恢复"),
                );
                return Err(error);
            }
        };
        // 4) 主会话（麦克风）以合并文件入库；偏移入库可查。
        repository.set_alignment_offset(&mic.session_id, offset_ms)?;
        let mut merged_captured = CapturedRecording {
            file_path: merged_path.clone(),
            bytes: merged_bytes,
            ..mic.clone()
        };
        merged_captured.title = format!("双路录音 {}", chrono::Local::now().format("%Y-%m-%d %H-%M"));
        let stopped = match state.recording.ingest_captured(&state.library, merged_captured) {
            Ok(stopped) => stopped,
            Err(error) => {
                let _ = repository.update_recording_session_status(
                    &system.session_id,
                    "failed",
                    system.duration_ms,
                    system.paused_ms,
                    system.bytes.min(i64::MAX as u64) as i64,
                    Some("双路合并文件入库失败；系统声音频可恢复"),
                );
                return Err(error);
            }
        };
        // 5) 副会话（系统声）记 canceled + 指向合并记录，留对账。
        repository.finish_paired_session(
            &system.session_id,
            &stopped.record_id,
            system.duration_ms,
            system.bytes.min(i64::MAX as u64) as i64,
        )?;
        // 6) 清理两路临时目录（合并文件在主目录里，随 ingest_captured 一并删除）。
        for path in [mic.file_path.clone(), system.file_path.clone()] {
            if let Some(parent) = path.parent() {
                let _ = std::fs::remove_dir_all(parent);
            }
        }
        Ok(DualStoppedRecording {
            stopped,
            alignment_offset_ms: offset_ms,
            system_session_id: system.session_id.clone(),
        })
    }

    pub fn cancel(&self, state: &AppState) -> AppResult<()> {
        let _active_gate = self.require_active()?;
        let mic_result = state.recording.cancel(&state.library);
        let system_result = state.system_audio.cancel(&state.library);
        *self.active.lock().unwrap_or_else(|error| error.into_inner()) = None;
        mic_result?;
        system_result
    }

    fn require_active(&self) -> AppResult<DualActive> {
        self.active
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
            .ok_or_else(|| AppError::Invalid("当前没有正在进行的双路录音".to_owned()))
    }
}

fn self_active(recording: &RecordingController) -> bool {
    matches!(
        recording.state().status,
        crate::recording::RecordingStatus::Recording | crate::recording::RecordingStatus::Paused
    )
}

fn self_active_system(system_audio: &SystemAudioController) -> bool {
    matches!(
        system_audio.state().status,
        crate::recording::RecordingStatus::Recording | crate::recording::RecordingStatus::Paused
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn merge_aligns_by_offset_with_leading_silence() {
        // 系统声晚启 1000ms（=48000 样本）：左侧立即有麦克风信号，
        // 右侧前 1000ms 是静音，之后对齐系统声内容。
        let mic: Vec<i16> = (0..48_000).map(|i| (i % 100) as i16).collect();
        let system: Vec<i16> = (0..48_000).map(|i| (i % 77 + 1) as i16).collect();
        let frames = merge_dual_samples(&mic, &system, 1_000);
        assert_eq!(frames.len(), 48_000 + 48_000);
        assert_ne!(frames[10].0, 0, "麦克风从 0 开始有声");
        assert_eq!(frames[10].1, 0, "系统声未启动前右侧静音");
        assert_eq!(frames[48_100].1, system[100], "偏移后右侧对齐系统声内容");
        // 负偏移（系统声先启）：镜像对称。
        let frames = merge_dual_samples(&mic, &system, -1_000);
        assert_eq!(frames.len(), 96_000);
        assert_eq!(frames[10].0, 0, "麦克风未启动前左侧静音");
        assert_eq!(frames[10].1, system[10], "系统声从 0 开始有声");
        assert_eq!(frames[48_100].0, mic[100], "偏移后左侧对齐麦克风内容");
    }

    #[test]
    fn merge_preserves_stereo_without_downmix() {
        // 不预混音：同一段信号进两路，输出两声道各自保留原值（而非平均）。
        let signal: Vec<i16> = vec![10_000, -20_000, 30_000];
        let frames = merge_dual_samples(&signal, &signal, 0);
        assert_eq!(frames[0], (10_000, 10_000));
        assert_eq!(frames[1], (-20_000, -20_000));
        assert_eq!(frames[2], (30_000, 30_000));
    }

    #[test]
    fn dual_wav_files_round_trip() {
        let dir = std::env::temp_dir().join(format!("echo-dual-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let mic_path = dir.join("mic.wav");
        let sys_path = dir.join("sys.wav");
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        {
            let mut writer = hound::WavWriter::create(&mic_path, spec).unwrap();
            for i in 0..4_800 {
                writer.write_sample((i % 50) as i16).unwrap();
            }
            writer.finalize().unwrap();
        }
        {
            let mut writer = hound::WavWriter::create(&sys_path, spec).unwrap();
            for i in 0..4_800 {
                writer.write_sample((i % 33 + 1) as i16).unwrap();
            }
            writer.finalize().unwrap();
        }
        let out = dir.join("merged.wav");
        let bytes = merge_dual_wav_files(&mic_path, &sys_path, 500, &out).unwrap();
        assert!(bytes > 44);
        let mut reader = hound::WavReader::open(&out).unwrap();
        assert_eq!(reader.spec().channels, 2);
        assert_eq!(reader.spec().sample_rate, 48_000);
        // 偏移 500ms = 24000 样本 → 总帧数 = max(4800, 24000+4800) = 28800 帧
        // = 57600 个交错样本。
        assert_eq!(reader.samples::<i16>().count(), 57_600);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn rejects_non_mono_or_wrong_rate_inputs() {
        let dir = std::env::temp_dir().join(format!("echo-dual-bad-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let stereo = dir.join("stereo.wav");
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        {
            let mut writer = hound::WavWriter::create(&stereo, spec).unwrap();
            writer.write_sample(0).unwrap();
            writer.write_sample(0).unwrap();
            writer.finalize().unwrap();
        }
        let out = dir.join("merged.wav");
        assert!(merge_dual_wav_files(&stereo, &stereo, 0, &out).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    /// 真机端到端（ECHO_DUAL_E2E=1）：双路同时采集 → 播放测试音 → 停止 →
    /// 验证合并立体声入库、偏移入库、合并文件可被转写管线预处理。
    /// 需要宿主进程已有麦克风与屏幕（系统声）权限；无权限时明确失败不静默。
    #[test]
    fn dual_capture_merges_and_ingests_end_to_end() {
        let Ok(flag) = std::env::var("ECHO_DUAL_E2E") else {
            return;
        };
        if flag != "1" {
            return;
        }
        let root = std::env::temp_dir().join(format!("echo-dual-e2e-{}", Uuid::new_v4()));
        let state = crate::state::AppState::initialize(root.clone()).expect("临时资料库应可建");

        // 播放 3 秒测试音（系统声侧的可听内容；麦克风侧无人在讲，右侧应有声）。
        let tone_path = std::env::temp_dir().join(format!("echo-dual-tone-{}.wav", Uuid::new_v4()));
        {
            let spec = hound::WavSpec {
                channels: 1,
                sample_rate: 48_000,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            };
            let mut writer = hound::WavWriter::create(&tone_path, spec).unwrap();
            for i in 0..48_000 * 3 {
                let value = (2.0 * std::f32::consts::PI * 440.0 * i as f32 / 48_000.0).sin();
                writer.write_sample((value * 0.6 * 32_767.0) as i16).unwrap();
            }
            writer.finalize().unwrap();
        }

        let started = state.dual.start(&state, None).expect("双路启动（需麦克风+屏幕权限已授予）");
        assert!(started.session_id.is_some());
        let play = std::process::Command::new("/usr/bin/afplay")
            .arg(&tone_path)
            .status()
            .expect("afplay 应可运行");
        assert!(play.success());
        std::thread::sleep(std::time::Duration::from_millis(600));

        let result = state.dual.stop(&state).expect("双路停止并合并");
        eprintln!(
            "双路端到端：record={} 偏移={}ms 系统会话={}",
            result.stopped.record_id, result.alignment_offset_ms, result.system_session_id
        );
        assert!(result.alignment_offset_ms.abs() < 10_000, "偏移应小于 10 秒");

        // 合并记录入库，音频是立体声 48k。
        let repository = state.library.repository();
        let record = repository.get_record(&result.stopped.record_id).expect("记录应入库");
        assert!(record.audio_duration_ms > 2_000, "合并时长应覆盖测试音");
        let audio_path = state.library.root().join("audio").join(&record.audio_hash[..2]).join(&record.audio_hash);
        let audio_path = if audio_path.is_file() {
            audio_path
        } else {
            // 存储布局可能不同：按 records.audio_path 逻辑兜底查找。
            state.library.root().join("audio").join(format!("{}.wav", record.audio_hash))
        };
        let mut reader = hound::WavReader::open(&audio_path).expect("合并音频应存在");
        assert_eq!(reader.spec().channels, 2, "合并文件必须是立体声（不预混音的两声道）");
        assert_eq!(reader.spec().sample_rate, 48_000);
        let samples: Vec<i16> = reader.samples::<i16>().map(Result::unwrap).collect();
        let right_rms = {
            let rights: Vec<f64> = samples.chunks_exact(2).map(|pair| (pair[1] as f64) * (pair[1] as f64)).collect();
            (rights.iter().sum::<f64>() / rights.len().max(1) as f64).sqrt()
        };
        eprintln!("双路端到端：右声道（系统声）rms={right_rms:.1}");
        assert!(right_rms > 50.0, "右声道应录到测试音（rms={right_rms}）");

        // 副会话 canceled + 指向合并记录；主会话记录实测偏移。
        let system_session = repository.get_recording_session(&result.system_session_id).unwrap();
        assert_eq!(system_session.status, "canceled");
        assert_eq!(system_session.record_id.as_deref(), Some(result.stopped.record_id.as_str()));
        let mic_session = repository
            .get_recording_session(result.stopped.session_id.as_str())
            .unwrap();
        assert_eq!(mic_session.paired_session_id.as_deref(), Some(result.system_session_id.as_str()));
        assert_eq!(mic_session.alignment_offset_ms, Some(result.alignment_offset_ms));

        // 合并文件可被转写管线预处理（入库→转写→纪要链路的入口）。
        let preprocessed = std::env::temp_dir().join(format!("echo-dual-pre-{}.wav", Uuid::new_v4()));
        crate::audio::preprocess(&audio_path, &preprocessed).expect("合并立体声应可预处理为转写输入");
        let _ = std::fs::remove_file(&preprocessed);
        let _ = std::fs::remove_file(&tone_path);
        let _ = std::fs::remove_dir_all(root);
    }
}
