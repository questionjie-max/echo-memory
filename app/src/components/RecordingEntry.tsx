/**
 * 本地记录模块（首页顶部）。
 *
 * 它是自足的：开始、暂停、继续、停止、放弃全部发生在这张卡片内部，
 * 不另行拉起全局控制条——状态变化只在框内发生，避免"点一下上面多出一条"的割裂感。
 * 录音时的计时与电平也在这里刷新。
 */
import { useEffect, useRef, useState } from "react";
import type {
  MicrophonePermission,
  RecoverableRecording,
  RecordingState,
} from "../shared/types";
import {
  cancelRecording,
  discardRecoverableRecording,
  getRecordingState,
  listRecoverableRecordings,
  microphonePermission,
  pauseRecording,
  recoverRecording,
  recordingLevels,
  requestMicrophonePermission,
  resumeRecording,
  startRecording,
  stopRecording,
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

/** 电平条宽度：把 -60dB~0dB 映射到 0%~100%，与系统表头量程一致。 */
function levelWidth(db: number | null | undefined): string {
  if (db === null || db === undefined || !Number.isFinite(db)) return "0%";
  const normalized = Math.max(0, Math.min(100, ((db + 60) / 60) * 100));
  return `${normalized}%`;
}

export default function RecordingEntry({ projectId, onImported, refreshKey }: Props) {
  const [state, setState] = useState<RecordingState | null>(null);
  const [permission, setPermission] = useState<MicrophonePermission>("undetermined");
  const [recoverable, setRecoverable] = useState<RecoverableRecording[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");

  const active = state?.status === "recording" || state?.status === "paused";
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
    try {
      const [current, currentPermission] = await Promise.all([
        getRecordingState(),
        microphonePermission(),
      ]);
      if (current) setState(current);
      if (currentPermission) setPermission(currentPermission);
    } catch {
      // 状态读取失败不阻塞入口渲染；开始录音时后端会给出明确错误。
    }
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
      void recordingLevels()
        .then((next) => {
          // 停止/放弃后丢弃在途快照，避免界面被拉回「正在记录」。
          if (generation === pollGeneration.current) setState(next);
        })
        .catch(() => undefined);
    }, 500);
    return () => window.clearInterval(timer);
  }, [active]);

  async function begin() {
    setBusy(true);
    setError("");
    setNotice("");
    try {
      let currentPermission = permission;
      if (currentPermission !== "granted") {
        currentPermission = await requestMicrophonePermission();
        setPermission(currentPermission);
      }
      if (currentPermission !== "granted") {
        throw new Error(permissionHint(currentPermission));
      }
      setState(await startRecording(projectId));
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
      setState(state?.status === "paused" ? await resumeRecording() : await pauseRecording());
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
      const result = await stopRecording();
      pollGeneration.current += 1;
      setState(null);
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
    if (!(await confirmAction("放弃这次录音吗？未封存的音频会被删除，无法恢复。"))) return;
    setBusy(true);
    setError("");
    setNotice("");
    try {
      await cancelRecording();
      pollGeneration.current += 1;
      setState(null);
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

  return (
    <section className="recording-entry" aria-label="本地记录">
      <div className="recording-entry-header">
        <div>
          <p className="recording-entry-eyebrow">本地记录</p>
          <h3>电脑设备直录</h3>
          <p className="recording-entry-lead">只使用本机麦克风，音频默认只保存在这台 Mac 上。</p>
        </div>
        {active ? (
          <div className="recording-entry-state" role="status" aria-live="polite">
            <span className="recording-live-dot" aria-hidden="true" />
            <strong>{state?.status === "paused" ? "已暂停" : "正在记录"}</strong>
            <time>{formatDuration(state?.elapsedMs ?? 0, true)}</time>
          </div>
        ) : (
          <button
            type="button"
            className="primary-button recording-entry-start"
            onClick={() => void begin()}
            disabled={busy || permission === "unsupported"}
          >
            {busy ? "准备中…" : "开始记录"}
          </button>
        )}
      </div>

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

      {!active && <p className="recording-entry-hint">{permissionHint(permission)}</p>}

      {recoverable.length > 0 && (
        <div className="recording-recovery" role="region" aria-label="可恢复录音">
          <div className="recording-recovery-heading">
            <strong>发现未封存录音</strong>
            <span>上次应用退出前有录音没有完成保存。</span>
          </div>
          {recoverable.map((session) => (
            <div className="recording-recovery-row" key={session.sessionId}>
              <div>
                <strong>{session.title}</strong>
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
