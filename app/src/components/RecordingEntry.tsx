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
import { isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type {
  CaptureSource,
  LiveMinutesEventPayload,
  LiveStatusPayload,
  LiveTickPayload,
  MicrophonePermission,
  RecoverableRecording,
  RecordingInputDevice,
  RecordingState,
  SystemAudioApplication,
  SystemAudioPermission,
} from "../shared/types";
import {
  applyLiveTick,
  emptyLiveView,
  formatSegmentTime,
  type LiveTranscriptView,
} from "../lib/liveTranscript";
import {
  cancelRecording,
  cancelSystemRecording,
  discardRecoverableRecording,
  getRecordingState,
  getSystemRecordingState,
  listRecoverableRecordings,
  listMicrophoneDevices,
  listSystemAudioApplications,
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
  isDualRecording,
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

/** 双路录音的显性告知：同时录麦克风与系统声（合并为立体声两个声道）。 */
const DUAL_CAPTURE_NOTICE = [
  "开始双路录音前请确认：",
  "双路录音会同时录制麦克风（你这边的声音）和这台 Mac 正在播放的所有声音（包括线上会议里对方说的话），合并成一个立体声文件保存。",
  "音频只保存在这台 Mac 上，转写与分析全部本机完成；只有你之后亲自确认的内容才会进入记忆库。",
  "如果录的是会议，请确认你已告知参会者或有权录制。",
].join("\n\n");

function systemCaptureNotice(applicationName: string | null): string {
  if (applicationName) {
    return [
      `开始录制「${applicationName}」的声音前请确认：`,
      `系统内录只会录制所选应用「${applicationName}」发出的声音，其他应用的声音不会被录制；麦克风不会被打开。`,
      "音频只保存在这台 Mac 上，转写与分析全部本机完成；只有你之后亲自确认的内容才会进入记忆库。",
      "如果录的是会议，请确认你已告知参会者或有权录制。",
    ].join("\n\n");
  }
  return SYSTEM_CAPTURE_NOTICE;
}

function dualCaptureNotice(applicationName: string | null): string {
  if (applicationName) {
    return [
      "开始双路录音前请确认：",
      `双路录音会同时录制麦克风（你这边的声音）和所选应用「${applicationName}」发出的声音，合并成一个立体声文件保存。`,
      "其他应用的声音不会被录制；音频只保存在这台 Mac 上，转写与分析全部本机完成。",
      "如果录的是会议，请确认你已告知参会者或有权录制。",
    ].join("\n\n");
  }
  return DUAL_CAPTURE_NOTICE;
}

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
  const [inputDevices, setInputDevices] = useState<RecordingInputDevice[]>([]);
  const [inputDeviceId, setInputDeviceId] = useState("");
  const [audioApplications, setAudioApplications] = useState<SystemAudioApplication[]>([]);
  const [applicationBundleId, setApplicationBundleId] = useState("");
  const [devicesLoading, setDevicesLoading] = useState(false);
  const [applicationsLoading, setApplicationsLoading] = useState(false);
  const [pickerNotice, setPickerNotice] = useState("");
  const [liveView, setLiveView] = useState<LiveTranscriptView>(emptyLiveView());
  const [liveMessage, setLiveMessage] = useState("");
  const [liveMinutes, setLiveMinutes] = useState<LiveMinutesEventPayload | null>(null);
  // 块3/F4：本会话实时纪要通道不可用（后端每会话发一次，文案随事件带出）。
  const [liveMinutesUnavailable, setLiveMinutesUnavailable] = useState<{
    sessionId: string;
    message: string;
  } | null>(null);
  // 双路模式由前端发起（backend 侧 stop/pause/cancel 按双路态自动分发）。
  const [dualActive, setDualActive] = useState(false);

  const micActive = isActiveState(micState);
  const sysActive = isActiveState(sysState);
  const active = micActive || sysActive;
  // 界面上同一时刻只展示一条进行中的采集；哪条活着就控制哪条。
  // 双路态必须走麦克风系命令（后端按双路态自动分发到两路）——否则切视图
  // 重挂载后会把系统声单路停掉，拆散双路编排（块8 审核 P0）。
  const activeSource: CaptureSource = dualActive ? "microphone" : sysActive ? "system" : "microphone";
  const state = sysActive && !dualActive ? sysState : micState;

  // 轮询代际：停止/放弃后，在途的电平快照不得把界面拉回「正在记录」。
  const pollGeneration = useRef(0);
  // 实时文字：事件订阅一次，会话切换时清空视图；旧会话的迟到 tick 一律丢弃。
  const liveSessionRef = useRef<string | null>(null);

  useEffect(() => {
    const sessionId = state?.sessionId ?? null;
    if (liveSessionRef.current !== sessionId) {
      liveSessionRef.current = sessionId;
      setLiveView(emptyLiveView());
      setLiveMessage("");
      setLiveMinutes(null);
      setLiveMinutesUnavailable(null);
    }
  }, [state?.sessionId]);

  useEffect(() => {
    if (!active) {
      setLiveView(emptyLiveView());
      setLiveMessage("");
      // 停止后增量纪要整体丢弃：权威来源是停止后的事后纪要（无冲突残留）。
      setLiveMinutes(null);
      setLiveMinutesUnavailable(null);
      setDualActive(false);
    }
  }, [active]);

  useEffect(() => {
    if (!isTauri()) return;
    let cancelled = false;
    const disposers: Array<() => void> = [];
    const track = (promise: Promise<() => void>) => {
      void promise.then((dispose) => {
        if (cancelled) dispose();
        else disposers.push(dispose);
      });
    };
    track(
      listen<LiveTickPayload | LiveStatusPayload>("live-transcript", (event) => {
        const payload = event.payload;
        if ("available" in payload) {
          // 状态事件也按会话过滤：旧会话线程退出前的失败消息不串台到新会话。
          if (payload.sessionId !== liveSessionRef.current) return;
          setLiveMessage(payload.available ? "" : payload.message);
          return;
        }
        if (payload.sessionId !== liveSessionRef.current) return;
        // 收到新的一拍说明实时链路活着：清掉上一拍的瞬时失败提示。
        setLiveMessage("");
        setLiveView((view) => applyLiveTick(view, payload));
      }),
    );
    track(
      listen<LiveMinutesEventPayload>("live-minutes", (event) => {
        const payload = event.payload;
        if (payload.sessionId !== liveSessionRef.current) return;
        setLiveMinutes(payload);
      }),
    );
    track(
      listen<{ sessionId: string; message: string }>("live-minutes-unavailable", (event) => {
        const payload = event.payload;
        if (payload.sessionId !== liveSessionRef.current) return;
        // 后端每会话只发一次；事件到达即表示本会话不会再有 live-minutes 数据。
        setLiveMinutesUnavailable(payload);
      }),
    );
    return () => {
      cancelled = true;
      disposers.forEach((dispose) => dispose());
    };
  }, []);

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
    const [current, currentMic, currentSys, currentSysPermission, dual] = await Promise.all([
      read(getRecordingState),
      read(microphonePermission),
      read(getSystemRecordingState),
      read(systemAudioPermission),
      read(isDualRecording),
    ]);
    if (current) setMicState(isActiveState(current) ? current : null);
    if (currentMic) setMicPermission(currentMic);
    if (currentSys) setSysState(isActiveState(currentSys) ? currentSys : null);
    if (currentSysPermission) setSysPermission(currentSysPermission);
    // 双路态以后端真值为准（组件重挂载后本地 flag 会丢）；
    // busy 窗口内（begin 尚未返回）不降级本地乐观值，防止 activeSource 翻转。
    setDualActive((previous) => (dual === true ? true : previous && busy));
    await refreshRecoverable();
  }

  useEffect(() => {
    void refreshState();
  }, [refreshKey]);

  useEffect(() => {
    setDevicesLoading(true);
    void listMicrophoneDevices()
      .then((devices) => {
        const next = Array.isArray(devices) ? devices : [];
        setInputDevices(next);
        setInputDeviceId((current) =>
          next.some((device) => device.id === current) ? current : "",
        );
      })
      .catch(() => setInputDevices([]))
      .finally(() => setDevicesLoading(false));
  }, []);

  async function loadAudioApplications(requestPermission: boolean) {
    setApplicationsLoading(true);
    setPickerNotice("");
    try {
      let permission = sysPermission;
      if (permission !== "granted") {
        if (!requestPermission) {
          setPickerNotice("允许「屏幕与系统声音录制」后，可以选择只录制某个应用。");
          return;
        }
        permission = await requestSystemAudioPermission();
        setSysPermission(permission);
      }
      if (permission !== "granted") {
        setPickerNotice(systemPermissionHint(permission));
        return;
      }
      const applications = await listSystemAudioApplications();
      const next = Array.isArray(applications) ? applications : [];
      setAudioApplications(next);
      setApplicationBundleId((current) =>
        next.some((application) => application.bundleId === current) ? current : "",
      );
      if (next.length === 0) {
        setPickerNotice("没有找到可单独录制的应用；仍可录制所有系统声音。");
      }
    } catch (reason) {
      setPickerNotice(displayError(reason));
    } finally {
      setApplicationsLoading(false);
    }
  }

  useEffect(() => {
    if (captureSource === "microphone" || sysPermission !== "granted") return;
    if (audioApplications.length === 0) void loadAudioApplications(false);
    // 只在切到系统声且已有权限时自动读取；用户点「读取应用」会显式发起授权。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [captureSource, sysPermission]);

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
    setMicState(await startRecording(projectId, undefined, undefined, inputDeviceId || null));
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
    if (!(await confirmAction(systemCaptureNotice(selectedApplicationName)))) return;
    setSysState(await startSystemRecording(projectId, true, applicationBundleId || null));
  }

  async function begin() {
    setBusy(true);
    setError("");
    setNotice("");
    try {
      if (captureSource === "system") {
        await beginSystem();
      } else if (captureSource === "both") {
        // 双路的隐私红线：与系统声同款显性确认，确认后才带 confirmed=true。
        if (!(await confirmAction(dualCaptureNotice(selectedApplicationName)))) return;
        setDualActive(true);
        setMicState(await startRecording(
          projectId,
          "both",
          true,
          inputDeviceId || null,
          applicationBundleId || null,
        ));
      } else {
        setDualActive(false);
        await beginMicrophone();
      }
    } catch (reason) {
      setDualActive(false);
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
      // 双路成功后系统声状态一并清掉：重挂载场景 refreshState 可能已把
      // sysState 置 active，不清会让 UI 停在幽灵「正在记录（双路）」。
      setSysState(null);
      setDualActive(false);
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
        dualActive
          ? "放弃这次双路录音吗？两路未封存的音频都会被删除，无法恢复。"
          : activeSource === "system"
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
      setSysState(null);
      setDualActive(false);
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

  const selectedApplicationName =
    audioApplications.find((application) => application.bundleId === applicationBundleId)?.name ?? null;
  const lead =
    captureSource === "system"
      ? selectedApplicationName
        ? `只录制「${selectedApplicationName}」发出的声音，不打开麦克风，音频只保存在本机。`
        : "录制这台 Mac 正在播放的声音（例如线上会议里对方的话），不打开麦克风，音频只保存在本机。"
      : captureSource === "both"
        ? selectedApplicationName
          ? `同时录麦克风与「${selectedApplicationName}」的声音，合并成一个立体声文件，开始前会再次向你确认。`
          : "同时录麦克风与系统声音，合并成一个立体声文件（左=麦克风，右=系统声），开始前会再次向你确认。"
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
                  : dualActive
                    ? "正在记录（双路）"
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
            {busy
              ? "准备中…"
              : captureSource === "system"
                ? "开始录系统声音"
                : captureSource === "both"
                  ? "开始双路录音"
                  : "开始记录"}
          </button>
        )}
      </div>

      {!active && (
        <div className="segmented-control" role="tablist" aria-label="录音来源">
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
          <button
            type="button"
            role="tab"
            aria-selected={captureSource === "both"}
            className={captureSource === "both" ? "selected" : ""}
            onClick={() => setCaptureSource("both")}
          >
            双路
          </button>
        </div>
      )}

      {!active && (
        <div className="recording-source-options">
          {(captureSource === "microphone" || captureSource === "both") && (
            <label className="recording-source-field">
              <span>麦克风</span>
              <select
                aria-label="麦克风设备"
                value={inputDeviceId}
                disabled={devicesLoading || busy}
                onChange={(event) => setInputDeviceId(event.target.value)}
              >
                <option value="">系统默认麦克风</option>
                {inputDevices.map((device) => (
                  <option value={device.id} key={device.id}>
                    {device.name}{device.isDefault ? "（系统默认）" : ""}
                  </option>
                ))}
              </select>
            </label>
          )}
          {(captureSource === "system" || captureSource === "both") && (
            <label className="recording-source-field">
              <span>系统声音来源</span>
              <select
                aria-label="系统声音来源"
                value={applicationBundleId}
                disabled={applicationsLoading || busy}
                onChange={(event) => setApplicationBundleId(event.target.value)}
              >
                <option value="">所有系统声音</option>
                {audioApplications.map((application) => (
                  <option value={application.bundleId} key={application.bundleId}>
                    {application.name}
                  </option>
                ))}
              </select>
            </label>
          )}
          {(captureSource === "system" || captureSource === "both") && (
            <button
              type="button"
              className="link-button recording-source-refresh"
              disabled={applicationsLoading || busy}
              onClick={() => void loadAudioApplications(true)}
            >
              {applicationsLoading ? "读取中…" : audioApplications.length ? "刷新应用列表" : "读取可录制应用"}
            </button>
          )}
        </div>
      )}
      {!active && pickerNotice && <p className="recording-entry-hint">{pickerNotice}</p>}

      {active && (
        <>
          <div className="recording-meter" aria-label="输入电平">
            <span style={{ width: levelWidth(state?.peakLevelDb ?? state?.inputLevelDb) }} />
          </div>
          <div className="live-transcript" aria-label="实时文字">
            <div className="live-transcript-heading">
              <strong>实时文字</strong>
              <span>边录边出，停止后以全量转写为准</span>
            </div>
            <div className="live-transcript-lines">
              {liveView.finals.length === 0 && liveView.provisional.length === 0 && (
                <p className="live-transcript-empty">说话后这里会出现带时间戳的文字。</p>
              )}
              {liveView.finals.map((segment) => (
                <p className="live-transcript-line" key={segment.startMs}>
                  <time>{formatSegmentTime(segment.startMs)}</time>
                  <span>{segment.text}</span>
                </p>
              ))}
              {liveView.provisional.map((segment) => (
                <p className="live-transcript-line provisional" key={`p-${segment.startMs}`}>
                  <time>{formatSegmentTime(segment.startMs)}</time>
                  <span>{segment.text}…</span>
                </p>
              ))}
            </div>
            {liveMessage && <p className="live-transcript-status" role="status">{liveMessage}</p>}
          </div>
          {liveMinutesUnavailable && liveMinutesUnavailable.sessionId === state?.sessionId && !liveMinutes && (
            <div className="live-minutes" aria-label="实时纪要不可用">
              <p className="live-transcript-status" role="status">{liveMinutesUnavailable.message}</p>
            </div>
          )}
          {liveMinutes && (
            <div className="live-minutes" aria-label="实时纪要">
              <div className="live-minutes-heading">
                <strong>实时纪要（暂定，以事后为准）</strong>
                <span>第 {liveMinutes.runIndex} 次生长</span>
              </div>
              <div className="live-minutes-body">
                {liveMinutes.document.conclusions.length === 0 &&
                liveMinutes.document.actionItems.length === 0 &&
                liveMinutes.document.topics.length === 0 ? (
                  <p className="live-transcript-empty">还在听，结论与待办会在这里逐步浮现。</p>
                ) : (
                  <>
                    {liveMinutes.document.topics.slice(0, 3).map((topic) => (
                      <p className="live-minutes-topic" key={topic.id}>
                        <strong>{topic.title}</strong>
                        {topic.summary && <span> — {topic.summary}</span>}
                      </p>
                    ))}
                    <ul className="live-minutes-list">
                      {liveMinutes.document.conclusions.map((item) => (
                        <li key={item.id}>
                          <time>{formatSegmentTime(item.evidence.startMs)}</time>
                          <span>{item.text}</span>
                        </li>
                      ))}
                      {liveMinutes.document.actionItems.map((item) => (
                        <li key={item.id} className="todo">
                          <time>{formatSegmentTime(item.evidence.startMs)}</time>
                          <span>
                            待办：{item.text}
                            {item.owner ? `（${item.owner}）` : ""}
                          </span>
                        </li>
                      ))}
                    </ul>
                  </>
                )}
              </div>
            </div>
          )}
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
            : captureSource === "both"
              ? `${permissionHint(micPermission)} ${systemPermissionHint(sysPermission)}`
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
