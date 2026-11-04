/**
 * 本地记录模块（首页顶部）。
 *
 * 它是自足的：开始、暂停、继续、停止、放弃全部发生在这张卡片内部，
 * 不另行拉起全局控制条——状态变化只在框内发生，避免"点一下上面多出一条"的割裂感。
 * 录音时的计时与电平也在这里刷新。
 *
 * v1.1 起支持两种来源：本机麦克风（原有链路）与系统内放声（ScreenCaptureKit）。
 * 系统内录的隐私底线在界面层落地：开始前必须走一遍显性告知与确认（确认框里写清
 * 会录到谁的声音、声音存哪里、入记忆前还要确认），权限被拒时只报错、绝不静默
 * 继续或改走麦克风。
 */
import { useEffect, useRef, useState } from "react";
import type {
  CaptureSource,
  MicrophonePermission,
  RecoverableRecording,
  RecordingState,
  SystemAudioPermission,
} from "../shared/types";
import {
  cancelRecording,
  cancelSystemRecording,
  discardRecoverableRecording,
  getRecordingState,
  getSystemRecordingState,
  listRecoverableRecordings,
  microphonePermission,
  pauseRecording,
  pauseSystemRecording,
  recoverRecording,
  recordingLevels,
  requestMicrophonePermission,
  requestSystemAudioPermission,
  resumeRecording,
  resumeSystemRecording,
  startRecording,
  startSystemRecording,
  stopRecording,
  stopSystemRecording,
  systemAudioPermission,
  systemRecordingLevels,
  transcribeRecord,
} from "../lib/tauri";
import { formatBytes, formatMinutesSeconds as formatDuration } from "../lib/format";
import { confirmAction } from "../lib/dialogs";

interface Props {
  projectId: string | null;
  /** 恢复出正式记录后触发，让工作区与首页刷新。 */
  onImported: () => void;
  /** 外部刷新信号（例如同一个录音在其他视图停止后回到首页）。 */
  refreshKey?: number;
}

function displayError(reason: unknown): string {
  return reason instanceof Error ? reason.message : String(reason);
}

function permissionHint(permission: MicrophonePermission): string {
  switch (permission) {
    case "denied":
      return "麦克风权限已拒绝，请在系统设置中允许回声记忆访问麦克风。";
    case "unsupported":
      return "当前平台不支持本地麦克风录音。";
    case "granted":
      return "录音结束后自动进入转写队列。";
    default:
      return "首次使用会请求麦克风权限；记录结束后自动进入转写队列。";
  }
}

function systemPermissionHint(permission: SystemAudioPermission): string {
  switch (permission) {
    case "denied":
      return "系统声音权限已拒绝。请在「系统设置 → 隐私与安全性 → 屏幕与系统声音录制」中允许回声记忆并重启应用；麦克风录音不受影响。";
    case "unsupported":
      return "当前 macOS 不支持系统声采集（需要 macOS 12.3 及以上），可以继续用麦克风录音。";
    case "granted":
      return "系统声音录制结束后自动进入转写队列。";
    default:
      return "首次使用会请求「屏幕与系统声音录制」权限。";
  }
}

/**
 * 系统内录的显性告知（隐私红线）：开始前用户必须确认过这段话才能开始。
 * 说清三件事——录到谁的声音、存在哪里、入记忆前还要确认。
 */
const SYSTEM_CAPTURE_NOTICE = [
  "开始录制系统声音前请确认：",
  "系统内录会录到这台 Mac 正在播放的所有声音——包括线上会议里对方说的话、视频、音乐和提示音。麦克风不会被打开。",
  "音频只保存在这台 Mac 上，转写与分析全部本机完成；只有你之后亲自确认的内容才会进入记忆库。",
  "如果录的是会议，请确认你已告知参会者或有权录制。",
].join("\n\n");

/** 电平条宽度：把 -60dB~0dB 映射到 0%~100%，与系统表头量程一致。 */
function levelWidth(db: number | null | undefined): string {
  if (db === null || db === undefined || !Number.isFinite(db)) return "0%";
  const normalized = Math.max(0, Math.min(100, ((db + 60) / 60) * 100));
  return `${normalized}%`;
}

function isActiveState(state: RecordingState | null): boolean {
  return state?.status === "recording" || state?.status === "paused";
}

export default function RecordingEntry({ projectId, onImported, refreshKey }: Props) {
  const [micState, setMicState] = useState<RecordingState | null>(null);
  const [sysState, setSysState] = useState<RecordingState | null>(null);
  const [micPermission, setMicPermission] = useState<MicrophonePermission>("undetermined");
  const [sysPermission, setSysPermission] = useState<SystemAudioPermission>("undetermined");
  const [recoverable, setRecoverable] = useState<RecoverableRecording[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [captureSource, setCaptureSource] = useState<CaptureSource>("microphone");

  const micActive = isActiveState(micState);
  const sysActive = isActiveState(sysState);
  const active = micActive || sysActive;
  // 界面上同一时刻只展示一条进行中的采集；哪条活着就控制哪条。
  const activeSource: CaptureSource = sysActive ? "system" : "microphone";
  const state = sysActive ? sysState : micState;

  // 轮询代际：停止/放弃后，在途的电平快照不得把界面拉回「正在记录」。
  const pollGeneration = useRef(0);

  async function refreshRecoverable() {
    try {
      const sessions = await listRecoverableRecordings();
      setRecoverable(Array.isArray(sessions) ? sessions : []);
    } catch {
      // 旧版本后端或非 Tauri 的测试环境没有该命令：按「没有残留」处理，不阻塞首页。
      setRecoverable([]);
    }
  }

  async function refreshState() {
    // 每个状态各自兜底：任一读取失败（含旧后端没有系统声命令）都不能拖垮
    // 其它状态的渲染，更不能阻塞入口。
    const read = async <T,>(call: () => Promise<T>): Promise<T | null> => {
      try {
        return await Promise.resolve(call());
      } catch {
        return null;
      }
    };
    const [current, currentMic, currentSys, currentSysPermission] = await Promise.all([
      read(getRecordingState),
      read(microphonePermission),
      read(getSystemRecordingState),
      read(systemAudioPermission),
    ]);
    if (current) setMicState(isActiveState(current) ? current : null);
    if (currentMic) setMicPermission(currentMic);
    if (currentSys) setSysState(isActiveState(currentSys) ? currentSys : null);
    if (currentSysPermission) setSysPermission(currentSysPermission);
    await refreshRecoverable();
  }

  useEffect(() => {
    void refreshState();
  }, [refreshKey]);

  // 录音中才轮询电平与计时：空闲时后端会直接报「没有正在进行的录音」。
  useEffect(() => {
    if (!active) return;
    const generation = pollGeneration.current;
    const timer = window.setInterval(() => {
      const poll = activeSource === "system" ? systemRecordingLevels() : recordingLevels();
      void poll
        .then((next) => {
          if (generation !== pollGeneration.current) return;
          if (activeSource === "system") setSysState(next);
          else setMicState(next);
        })
        .catch(() => undefined);
    }, 500);
    return () => window.clearInterval(timer);
  }, [active, activeSource]);

  async function beginMicrophone() {
    let currentPermission = micPermission;
    if (currentPermission !== "granted") {
      currentPermission = await requestMicrophonePermission();
      setMicPermission(currentPermission);
    }
    if (currentPermission !== "granted") {
      throw new Error(permissionHint(currentPermission));
    }
    setMicState(await startRecording(projectId));
  }

  async function beginSystem() {
    let currentPermission = sysPermission;
    if (currentPermission !== "granted") {
      currentPermission = await requestSystemAudioPermission();
      setSysPermission(currentPermission);
    }
    if (currentPermission !== "granted") {
      // 明确报错并停在这里；麦克风录音能力不受影响，用户可切回麦克风来源。
      throw new Error(systemPermissionHint(currentPermission));
    }
    // 隐私红线：没有确认过显性告知，就不开始录系统声。
    // confirmed=true 是后端的硬性参数：没走确认框后端直接拒绝。
    if (!(await confirmAction(SYSTEM_CAPTURE_NOTICE))) return;
    setSysState(await startSystemRecording(projectId, true));
  }

  async function begin() {
    setBusy(true);
    setError("");
    setNotice("");
    try {
      if (captureSource === "system") {
        await beginSystem();
      } else {
        await beginMicrophone();
      }
    } catch (reason) {
      setError(displayError(reason));
    } finally {
      setBusy(false);
    }
  }

  async function pauseOrResume() {
    setBusy(true);
    setError("");
    try {
      if (activeSource === "system") {
        setSysState(
          sysState?.status === "paused" ? await resumeSystemRecording() : await pauseSystemRecording(),
        );
      } else {
        setMicState(
          micState?.status === "paused" ? await resumeRecording() : await pauseRecording(),
        );
      }
    } catch (reason) {
      setError(displayError(reason));
    } finally {
      setBusy(false);
    }
  }

  async function stop() {
    setBusy(true);
    setError("");
    try {
      const result =
        activeSource === "system" ? await stopSystemRecording() : await stopRecording();
      pollGeneration.current += 1;
      if (activeSource === "system") setSysState(null);
      else setMicState(null);
      setNotice(`已保存“${result.title}”，正在准备本机转写。`);
      onImported();
      try {
        await transcribeRecord(result.recordId);
        onImported();
      } catch (reason) {
        setNotice(`已保存“${result.title}”，但转写尚未开始：${displayError(reason)}`);
      }
    } catch (reason) {
      // 停止失败可能留下可恢复会话：重读状态并刷新出恢复入口，而不是静默吞掉。
      pollGeneration.current += 1;
      setError(displayError(reason));
      await refreshState();
      onImported();
    } finally {
      setBusy(false);
    }
  }

  async function cancel() {
    if (
      !(await confirmAction(
        activeSource === "system"
          ? "放弃这次系统声音录制吗？未封存的音频会被删除，无法恢复。"
          : "放弃这次录音吗？未封存的音频会被删除，无法恢复。",
      ))
    ) {
      return;
    }
    setBusy(true);
    setError("");
    setNotice("");
    try {
      if (activeSource === "system") {
        await cancelSystemRecording();
        setSysState(null);
      } else {
        await cancelRecording();
        setMicState(null);
      }
      pollGeneration.current += 1;
      setNotice("已放弃本次录音。");
      onImported();
    } catch (reason) {
      setError(displayError(reason));
    } finally {
      setBusy(false);
    }
  }

  async function recover(session: RecoverableRecording) {
    setBusy(true);
    setError("");
    setNotice("");
    try {
      const result = await recoverRecording(session.sessionId);
      setNotice(`已恢复“${result.title}”，正在准备本机转写。`);
      await refreshState();
      onImported();
      try {
        await transcribeRecord(result.recordId);
        onImported();
      } catch (reason) {
        setNotice(`已恢复“${result.title}”，但转写尚未开始：${displayError(reason)}`);
      }
    } catch (reason) {
      setError(displayError(reason));
    } finally {
      setBusy(false);
    }
  }

  async function discard(session: RecoverableRecording) {
    if (!(await confirmAction(`放弃“${session.title}”的未封存录音吗？`))) return;
    setBusy(true);
    setError("");
    setNotice("");
    try {
      await discardRecoverableRecording(session.sessionId);
      await refreshRecoverable();
      setNotice(`已放弃“${session.title}”。`);
    } catch (reason) {
      setError(displayError(reason));
    } finally {
      setBusy(false);
    }
  }

  const lead =
    captureSource === "system"
      ? "录制这台 Mac 正在播放的声音（例如线上会议里对方的话），不打开麦克风，音频只保存在本机。"
      : "只使用本机麦克风，音频默认只保存在这台 Mac 上。";

  return (
    <section className="recording-entry" aria-label="本地记录">
      <div className="recording-entry-header">
        <div>
          <p className="recording-entry-eyebrow">本地记录</p>
          <h3>电脑设备直录</h3>
          <p className="recording-entry-lead">{lead}</p>
        </div>
        {active ? (
          <div className="recording-entry-state" role="status" aria-live="polite">
            <span className="recording-live-dot" aria-hidden="true" />
            <strong>
              {state?.status === "paused"
                ? "已暂停"
                : activeSource === "system"
                  ? "正在记录系统声音"
                  : "正在记录"}
            </strong>
            <time>{formatDuration(state?.elapsedMs ?? 0, true)}</time>
          </div>
        ) : (
          <button
            type="button"
            className="primary-button recording-entry-start"
            onClick={() => void begin()}
            disabled={
              busy ||
              (captureSource === "microphone"
                ? micPermission === "unsupported"
                : sysPermission === "unsupported")
            }
          >
            {busy ? "准备中…" : captureSource === "system" ? "开始录系统声音" : "开始记录"}
          </button>
        )}
      </div>

      {!active && (
        <div
          className="segmented-control two"
          role="tablist"
          aria-label="录音来源"
        >
          <button
            type="button"
            role="tab"
            aria-selected={captureSource === "microphone"}
            className={captureSource === "microphone" ? "selected" : ""}
            onClick={() => setCaptureSource("microphone")}
          >
            麦克风
          </button>
          <button
            type="button"
            role="tab"
            aria-selected={captureSource === "system"}
            className={captureSource === "system" ? "selected" : ""}
            onClick={() => setCaptureSource("system")}
          >
            系统声音
          </button>
        </div>
      )}

      {active && (
        <>
          <div className="recording-meter" aria-label="输入电平">
            <span style={{ width: levelWidth(state?.peakLevelDb ?? state?.inputLevelDb) }} />
          </div>
          <div className="recording-entry-actions">
            <button type="button" className="secondary-button" onClick={() => void pauseOrResume()} disabled={busy}>
              {state?.status === "paused" ? "继续" : "暂停"}
            </button>
            <button type="button" className="primary-button" onClick={() => void stop()} disabled={busy}>
              {busy ? "封存中…" : "停止并保存"}
            </button>
            <button type="button" className="link-button" onClick={() => void cancel()} disabled={busy}>
              放弃
            </button>
          </div>
        </>
      )}

      {!active && (
        <p className="recording-entry-hint">
          {captureSource === "system"
            ? systemPermissionHint(sysPermission)
            : permissionHint(micPermission)}
        </p>
      )}

      {recoverable.length > 0 && (
        <div className="recording-recovery" role="region" aria-label="可恢复录音">
          <div className="recording-recovery-heading">
            <strong>发现未封存录音</strong>
            <span>上次应用退出前有录音没有完成保存。</span>
          </div>
          {recoverable.map((session) => (
            <div className="recording-recovery-row" key={session.sessionId}>
              <div>
                <strong>
                  {session.source === "system" ? "系统声音 · " : ""}
                  {session.title}
                </strong>
                <span>{new Date(session.startedAt).toLocaleString()} · {formatBytes(session.writtenBytes)}</span>
              </div>
              <div className="recording-recovery-actions">
                <button type="button" className="secondary-button" onClick={() => void discard(session)} disabled={busy}>放弃</button>
                <button type="button" className="primary-button" onClick={() => void recover(session)} disabled={busy}>恢复</button>
              </div>
            </div>
          ))}
        </div>
      )}

      {notice && <p className="recording-entry-notice" role="status">{notice}</p>}
      {error && <p className="recording-entry-error" role="alert">{error}</p>}
    </section>
  );
}
