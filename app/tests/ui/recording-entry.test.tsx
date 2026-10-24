/**
 * 本地记录模块（首页顶层）的行为测试。
 *
 * 盯着四件承诺：
 *  1. 「开始记录」把当前知识库范围带进录音会话，且开始/暂停/结束全部发生在卡内——
 *     不另起独立的顶部控制条；
 *  2. 麦克风权限被拒时给出可诊断提示，且绝不启动录音；
 *  3. 录音中「暂停/继续」原地切换，计时与操作按钮都在同一张卡里；
 *  4. 「停止并保存」触发转写 + 通知外部刷新，并原地给出已保存提示；
 *  5. 未封存录音可以恢复：恢复后自动触发转写并通知外部刷新。
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import RecordingEntry from "../../src/components/RecordingEntry";
import * as tauri from "../../src/lib/tauri";
import type { RecordingState } from "../../src/shared/types";

vi.mock("../../src/lib/tauri");

function recordingState(overrides: Partial<RecordingState> = {}): RecordingState {
  return {
    status: "idle",
    sessionId: null,
    elapsedMs: 0,
    inputLevelDb: null,
    peakLevelDb: null,
    filePath: null,
    error: null,
    ...overrides,
  };
}

function activeState(status: "recording" | "paused"): RecordingState {
  return recordingState({
    status,
    sessionId: "session-1",
    elapsedMs: 12_000,
    inputLevelDb: -18,
    peakLevelDb: -10,
    filePath: "/library/recordings/inflight/session-1/capture.wav",
  });
}

function card() {
  return within(screen.getByRole("region", { name: "本地记录" }));
}

beforeEach(() => {
  vi.resetAllMocks();
  vi.mocked(tauri.getRecordingState).mockResolvedValue(recordingState());
  vi.mocked(tauri.microphonePermission).mockResolvedValue("granted");
  vi.mocked(tauri.listRecoverableRecordings).mockResolvedValue([]);
  vi.mocked(tauri.recordingLevels).mockResolvedValue(activeState("recording"));
});

describe("本地记录模块", () => {
  it("开始/暂停/结束都发生在卡内，不另起顶部控制条", async () => {
    vi.mocked(tauri.startRecording).mockResolvedValue(activeState("recording"));
    vi.mocked(tauri.pauseRecording).mockResolvedValue(activeState("paused"));
    vi.mocked(tauri.resumeRecording).mockResolvedValue(activeState("recording"));
    vi.mocked(tauri.stopRecording).mockResolvedValue({
      sessionId: "session-1",
      recordId: "record-1",
      durationMs: 12_000,
      title: "本地录音 2026-10-16 18-00",
    });
    vi.mocked(tauri.transcribeRecord).mockResolvedValue(undefined);
    const onImported = vi.fn();
    const { container } = render(<RecordingEntry projectId="project-1" onImported={onImported} />);

    fireEvent.click(await screen.findByRole("button", { name: "开始记录" }));
    await waitFor(() => expect(tauri.startRecording).toHaveBeenCalledWith("project-1"));

    // 录音中：状态、计时、控制钮全在同一张卡内。
    expect(await screen.findByText("正在记录")).toBeTruthy();
    expect(card().getByText("0:12")).toBeTruthy();
    expect(card().getByRole("button", { name: "暂停" })).toBeTruthy();
    expect(card().getByRole("button", { name: "停止并保存" })).toBeTruthy();
    expect(card().getByRole("button", { name: "放弃" })).toBeTruthy();
    // 旧的独立控制条不再存在。
    expect(container.querySelector(".recording-status-bar")).toBeNull();

    fireEvent.click(card().getByRole("button", { name: "暂停" }));
    expect(await screen.findByText("已暂停")).toBeTruthy();
    expect(card().getByRole("button", { name: "继续" })).toBeTruthy();

    fireEvent.click(card().getByRole("button", { name: "停止并保存" }));
    await waitFor(() => expect(tauri.transcribeRecord).toHaveBeenCalledWith("record-1"));
    expect(onImported).toHaveBeenCalledTimes(2);
    expect(await screen.findByText(/已保存/)).toBeTruthy();
    expect(screen.queryByRole("button", { name: "停止并保存" })).toBeNull();
  });

  it("权限被拒时给出明确提示且不启动录音", async () => {
    vi.mocked(tauri.microphonePermission).mockResolvedValue("denied");
    vi.mocked(tauri.requestMicrophonePermission).mockResolvedValue("denied");
    render(<RecordingEntry projectId={null} onImported={() => undefined} />);

    fireEvent.click(await screen.findByRole("button", { name: "开始记录" }));

    expect((await screen.findAllByText(/麦克风权限已拒绝/)).length).toBeGreaterThan(0);
    expect(tauri.startRecording).not.toHaveBeenCalled();
  });

  it("未封存录音可恢复并自动触发转写", async () => {
    vi.mocked(tauri.listRecoverableRecordings).mockResolvedValue([
      {
        sessionId: "session-crashed",
        status: "recoverable",
        title: "未封存的产品讨论",
        startedAt: "2026-10-16T09:00:00.000Z",
        durationMs: 90_000,
        writtenBytes: 12_345,
        lastError: "上次应用退出时录音未封存，请恢复或放弃",
      },
    ]);
    vi.mocked(tauri.recoverRecording).mockResolvedValue({
      sessionId: "session-crashed",
      recordId: "record-recovered",
      durationMs: 90_000,
      title: "未封存的产品讨论",
    });
    vi.mocked(tauri.transcribeRecord).mockResolvedValue(undefined);
    const onImported = vi.fn();
    render(<RecordingEntry projectId={null} onImported={onImported} />);

    expect(await screen.findByText("未封存的产品讨论")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "恢复" }));

    await waitFor(() => expect(tauri.recoverRecording).toHaveBeenCalledWith("session-crashed"));
    expect(tauri.transcribeRecord).toHaveBeenCalledWith("record-recovered");
    expect(onImported).toHaveBeenCalled();
  });
});
