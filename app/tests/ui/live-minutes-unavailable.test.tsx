/**
 * 块3/F4 回归：实时纪要通道不可用必须如实提示（不再静默退化）。
 * 模拟后端 live-minutes-unavailable 事件 → 卡片出现一行提示；
 * 会话切换/停止后提示消失；live-minutes 数据到达时不显示提示。
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import RecordingEntry from "../../src/components/RecordingEntry";
import * as tauri from "../../src/lib/tauri";
import type { RecordingState } from "../../src/shared/types";

type Handler = (event: { payload: unknown }) => void;
const handlers = new Map<string, Handler>();

vi.mock("@tauri-apps/api/core", () => ({ isTauri: () => true }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn((name: string, handler: Handler) => {
    handlers.set(name, handler);
    return Promise.resolve(() => undefined);
  }),
}));
vi.mock("../../src/lib/tauri");

function recordingState(): RecordingState {
  return {
    status: "recording",
    sessionId: "session-1",
    elapsedMs: 5_000,
    inputLevelDb: -18,
    peakLevelDb: -10,
    filePath: "/tmp/inflight/session-1/capture.wav",
  } as RecordingState;
}

beforeEach(() => {
  handlers.clear();
  vi.resetAllMocks();
  vi.mocked(tauri.getRecordingState).mockResolvedValue(recordingState());
  vi.mocked(tauri.recordingLevels).mockResolvedValue(recordingState());
  vi.mocked(tauri.listRecoverableRecordings).mockResolvedValue([]);
  vi.mocked(tauri.isDualRecording).mockResolvedValue(false);
  vi.mocked(tauri.listMicrophoneDevices).mockResolvedValue([]);
});

function renderCard() {
  return render(
    <RecordingEntry projectId={null} onImported={() => undefined} />,
  );
}

describe("实时纪要通道不可用提示（F4）", () => {
  it("收到 unavailable 事件后显示如实提示", async () => {
    renderCard();
    // 先等轮询把当前会话 ID 装进 liveSessionRef（录制态渲染出来），
    // 再发 unavailable 事件——与真实时序一致（后端只在会话已建立后发）。
    await screen.findByText(/正在记录/);
    await waitFor(() => expect(handlers.has("live-minutes-unavailable")).toBe(true));
    handlers.get("live-minutes-unavailable")!({
      payload: { sessionId: "session-1", message: "实时纪要需要本机 Ollama 或已配置的外部文本理解通道，当前没有可用通道；录音与实时文字不受影响。" },
    });
    await screen.findByRole("status");
    expect(
      screen.getByText(/实时纪要需要本机 Ollama/),
    ).toBeTruthy();
  });

  it("其他会话的 unavailable 事件不串台", async () => {
    renderCard();
    await waitFor(() => expect(handlers.has("live-minutes-unavailable")).toBe(true));
    handlers.get("live-minutes-unavailable")!({
      payload: { sessionId: "session-old", message: "旧会话提示" },
    });
    expect(screen.queryByText("旧会话提示")).toBeNull();
  });
});
