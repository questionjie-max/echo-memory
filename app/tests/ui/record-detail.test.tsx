import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import RecordDetail from "../../src/components/RecordDetail";
import * as tauri from "../../src/lib/tauri";
import type { RecordBrief, StoredAnalysis } from "../../src/shared/types";
import { engineStatusFixture, localAiStatusFixture } from "./settings-mocks";

const eventMock = vi.hoisted(() => ({
  handlers: new Map<string, Set<(event: { payload: unknown }) => void>>(),
}));

vi.mock("@tauri-apps/api/core", () => ({
  convertFileSrc: vi.fn((path: string) => `asset://${path}`),
  isTauri: () => true,
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn((name: string, handler: (event: { payload: unknown }) => void) => {
    const handlers = eventMock.handlers.get(name) ?? new Set();
    handlers.add(handler);
    eventMock.handlers.set(name, handlers);
    return Promise.resolve(() => {
      handlers.delete(handler);
    });
  }),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ save: () => Promise.resolve(null) }));
vi.mock("../../src/lib/tauri");

beforeAll(() => {
  Element.prototype.scrollTo = vi.fn();
});

function recordFixture(overrides: Partial<RecordBrief> = {}): RecordBrief {
  return {
    id: "record-1",
    title: "产品评审",
    projectId: null,
    projectName: null,
    sourceType: "import",
    audioHash: "hash-1",
    audioDurationMs: 60_000,
    importedAt: "2026-09-23T10:00:00.000Z",
    status: "completed",
    hasTranscript: true,
    hasAnalysis: false,
    analysisStatus: null,
    analysisHasQualityWarning: false,
    lastAnalysisError: null,
    analysisTemplateId: null,
    processingStage: null,
    progressCurrent: 0,
    progressTotal: 0,
    archivedAt: null,
    ...overrides,
  };
}

function emit(name: string, payload: unknown) {
  act(() => {
    eventMock.handlers.get(name)?.forEach((handler) => handler({ payload }));
  });
}

function analysisFixture(qualityWarning: string): StoredAnalysis {
  const content = {
    summary: "这是一段完整的分析摘要，能够说明讨论主题与结论。",
    key_points: [],
    decisions: [],
    action_items: [],
    open_questions: [],
    custom_sections: [],
    quality_warning: qualityWarning,
  };
  return {
    id: "analysis-1",
    recordId: "record-1",
    sourceTranscriptVersionId: "transcript-1",
    status: "completed",
    contentJson: JSON.stringify(content),
    provider: "ollama",
    model: "qwen",
    templateVersion: "knowledge-v1",
    templateId: "builtin-standard",
    templateSnapshotJson: "{}",
    createdAt: "2026-09-23T10:00:00.000Z",
  };
}

function renderDetail(record = recordFixture()) {
  return render(
    <RecordDetail record={record} navigation={null} onChanged={() => undefined} />,
  );
}

async function waitForAudioReady() {
  await screen.findByText("正在载入音频");
  fireEvent.canPlay(document.querySelector("audio") as HTMLAudioElement);
  await screen.findByText("音频已就绪");
}

beforeEach(() => {
  vi.resetAllMocks();
  eventMock.handlers.clear();
  vi.mocked(tauri.relatedRecords).mockResolvedValue([]);
  vi.mocked(tauri.getRecordSpeakers).mockResolvedValue([]);
  vi.mocked(tauri.getTranscriptionEngineStatus).mockResolvedValue(engineStatusFixture());
  vi.mocked(tauri.getRecord).mockResolvedValue(recordFixture());
  vi.mocked(tauri.listTranscriptBlocks).mockResolvedValue([]);
  vi.mocked(tauri.latestAnalysis).mockResolvedValue(null);
  vi.mocked(tauri.listProjects).mockResolvedValue([]);
  vi.mocked(tauri.listAnalysisTemplates).mockResolvedValue([]);
  vi.mocked(tauri.getLocalAiStatus).mockResolvedValue(localAiStatusFixture());
  vi.mocked(tauri.recordAudioPath).mockResolvedValue("/library/audio/record-1.m4a");
});

describe("记录详情：事件刷新与轮询边界", () => {
  it("当前记录的处理事件立即刷新，不等待轮询", async () => {
    renderDetail();
    await waitForAudioReady();
    const before = vi.mocked(tauri.getRecord).mock.calls.length;

    emit("processing-progress", { recordId: "record-1" });

    await waitFor(() =>
      expect(vi.mocked(tauri.getRecord).mock.calls.length).toBeGreaterThan(before),
    );
  });

  it("其他记录的事件不会刷新当前详情", async () => {
    renderDetail();
    await waitForAudioReady();
    const before = vi.mocked(tauri.getRecord).mock.calls.length;

    emit("processing-progress", { recordId: "record-2" });
    await act(async () => {
      await Promise.resolve();
    });

    expect(tauri.getRecord).toHaveBeenCalledTimes(before);
  });

  it("处理中的记录保留 2 秒兜底轮询", async () => {
    const interval = vi.spyOn(window, "setInterval");
    renderDetail(recordFixture({ status: "transcribing", processingStage: "transcribing" }));
    await waitForAudioReady();

    expect(interval).toHaveBeenCalledWith(expect.any(Function), 2_000);
    interval.mockRestore();
  });

  it("已完成记录不创建常驻轮询", async () => {
    const interval = vi.spyOn(window, "setInterval");
    renderDetail();
    await waitForAudioReady();
    await act(async () => {
      await Promise.resolve();
    });

    expect(interval).not.toHaveBeenCalledWith(expect.any(Function), 2_000);
    interval.mockRestore();
  });
});

describe("记录详情：分析质量分级", () => {
  it("建议性提醒完成分析并显示质量提醒", async () => {
    const record = recordFixture({
      hasAnalysis: true,
      analysisStatus: "completed",
      analysisHasQualityWarning: true,
    });
    vi.mocked(tauri.getRecord).mockResolvedValue(record);
    vi.mocked(tauri.latestAnalysis).mockResolvedValue(
      analysisFixture("摘要过短，无法充分说明谈话主题和结论"),
    );
    renderDetail(record);
    await waitFor(() => expect(tauri.latestAnalysis).toHaveBeenCalledWith("record-1"));
    await screen.findByText("录音详情 · 已完成 · 有质量提醒");

    fireEvent.click(screen.getByRole("tab", { name: "内容分析" }));

    await screen.findByText(
      "质量提醒：摘要过短，无法充分说明谈话主题和结论",
    );
  });

  it("阻断性问题保留分析不完整状态", async () => {
    const record = recordFixture({
      hasAnalysis: true,
      analysisStatus: "incomplete",
      analysisHasQualityWarning: true,
    });
    vi.mocked(tauri.getRecord).mockResolvedValue(record);
    vi.mocked(tauri.latestAnalysis).mockResolvedValue(
      analysisFixture("没有提取出带可靠出处的关键观点"),
    );
    renderDetail(record);
    await waitFor(() => expect(tauri.latestAnalysis).toHaveBeenCalledWith("record-1"));
    await screen.findByText("录音详情 · 分析不完整");

    fireEvent.click(screen.getByRole("tab", { name: "内容分析" }));

    await screen.findByText(
      "分析不完整：没有提取出带可靠出处的关键观点",
    );
  });
});

describe("记录详情：资料库音频权限", () => {
  it.each([
    ["/library/audio/record-1.m4a", "asset:///library/audio/record-1.m4a"],
    ["/custom-library/audio/record-1.m4a", "asset:///custom-library/audio/record-1.m4a"],
  ])("资料库路径 %s 转换后可进入可播放状态", async (path, expected) => {
    vi.mocked(tauri.recordAudioPath).mockResolvedValue(path);
    renderDetail();
    await waitForAudioReady();
    expect(document.querySelector("audio")?.src).toBe(expected);
    expect(screen.getByRole("button", { name: "播放" }).disabled).toBe(false);
  });

  it("资料库外路径被拒绝时显示载入错误", async () => {
    vi.mocked(tauri.recordAudioPath).mockRejectedValue(
      new Error("音频路径不在当前资料库内"),
    );
    renderDetail();

    await screen.findByText(/无法载入音频：Error: 音频路径不在当前资料库内/);
    expect(screen.getByRole("button", { name: "播放" }).disabled).toBe(true);
  });

  it("主数据读取失败时显示可诊断的加载错误", async () => {
    vi.mocked(tauri.getRecord).mockRejectedValue(new Error("数据库不可用"));
    renderDetail();

    await screen.findByText(/加载记录详情失败：Error: 数据库不可用/);
  });
});
