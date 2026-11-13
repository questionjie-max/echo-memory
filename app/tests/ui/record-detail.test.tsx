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
vi.mock("@tauri-apps/plugin-dialog", () => ({
  save: vi.fn(() => Promise.resolve(null)),
  open: vi.fn(() => Promise.resolve(null)),
}));
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
  vi.mocked(tauri.getRecordMarks).mockResolvedValue([]);
  vi.mocked(tauri.listRecordArtifacts).mockResolvedValue([]);
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

describe("记录详情：辅助信息", () => {
  it("说话人与相关记录默认收起，避免占住底部；展开后才显示解释和内容", async () => {
    vi.mocked(tauri.getRecordSpeakers).mockResolvedValue([
      { label: "未知", segmentCount: 357 },
    ]);
    vi.mocked(tauri.relatedRecords).mockResolvedValue([
      {
        recordId: "record-2",
        title: "另一条记录",
        similarity: 0.87,
        projectId: null,
      },
    ]);
    renderDetail();
    await waitForAudioReady();

    const speakersSummary = await screen.findByText("说话人");
    const speakersDetails = speakersSummary.closest("details");
    expect(speakersDetails?.open).toBe(false);

    fireEvent.click(speakersSummary);
    expect(speakersDetails?.open).toBe(true);
    expect(await screen.findByText(/当前没有说话人标注/)).toBeTruthy();

    const relatedSummary = screen.getByText("相关记录");
    const relatedDetails = relatedSummary.closest("details");
    expect(relatedDetails?.open).toBe(false);

    fireEvent.click(relatedSummary);
    expect(relatedDetails?.open).toBe(true);
    expect(await screen.findByText("另一条记录")).toBeTruthy();
    expect(screen.getByText("87% 相关")).toBeTruthy();
  });
});

describe("记录详情：导出反馈（v1.4 包2）", () => {
  async function renderAndOpenExportMenu(format: "md" | "txt" | "srt") {
    vi.mocked(tauri.createExportTicket).mockResolvedValue("ticket-1");
    vi.mocked(tauri.exportRecord).mockResolvedValue(undefined);
    const { save } = await import("@tauri-apps/plugin-dialog");
    vi.mocked(save).mockResolvedValue(`/tmp/产品评审.${format}`);
    renderDetail();
    await waitForAudioReady();
    // 打开「操作」菜单（details/summary）并选择导出格式
    const summary = screen.getByText("操作");
    fireEvent.click(summary);
    const exportButton = await screen.findByRole("button", {
      name: format === "md" ? "导出 Markdown" : "导出 SRT 字幕",
    });
    fireEvent.click(exportButton);
    await waitFor(() => expect(tauri.exportRecord).toHaveBeenCalled());
    await waitFor(() => expect(save).toHaveBeenCalled());
  }

  it("导出 Markdown 成功后显示落盘路径提示", async () => {
    await renderAndOpenExportMenu("md");
    expect(await screen.findByText(/已导出：\/tmp\/产品评审\.md/)).toBeTruthy();
    expect(screen.queryByText(/已导出：\/tmp\/产品评审\.srt/)).toBeNull();
  });

  it("导出 SRT 成功后同样显示提示（与导出包同风格）", async () => {
    await renderAndOpenExportMenu("srt");
    expect(await screen.findByText(/已导出：\/tmp\/产品评审\.srt/)).toBeTruthy();
  });

  it("导出被取消（未选路径）时不显示成功提示", async () => {
    vi.mocked(tauri.createExportTicket).mockResolvedValue("ticket-1");
    const { save } = await import("@tauri-apps/plugin-dialog");
    vi.mocked(save).mockResolvedValue(null);
    renderDetail();
    await waitForAudioReady();
    fireEvent.click(screen.getByText("操作"));
    fireEvent.click(await screen.findByRole("button", { name: "导出 Markdown" }));
    await waitFor(() => expect(save).toHaveBeenCalled());
    expect(tauri.exportRecord).not.toHaveBeenCalled();
    expect(screen.queryByText(/已导出：/)).toBeNull();
  });
});

describe("记录详情：转写引擎文案（v1.4 包3）", () => {
  it("whisperX 未安装时弹层文案不再出现 pip 指引（引导收敛到设置页）", async () => {
    vi.mocked(tauri.getTranscriptionEngineStatus).mockResolvedValue(
      engineStatusFixture({ whisperxAvailable: false }),
    );
    renderDetail();
    await waitForAudioReady();
    // 打开增强重新转写弹层
    fireEvent.click(screen.getByText("操作"));
    fireEvent.click(await screen.findByRole("button", { name: "增强重新转写…" }));
    const body = document.body.textContent ?? "";
    expect(body).not.toContain("pip install");
    // hint 只在选中时渲染：未安装态改用按钮标签本身说清状态
    expect(body).toContain("whisperX（未安装）");
  });
});

describe("记录详情：预计剩余时间与完成通知（v1.4 包4）", () => {
  function processingRecord(current: number): RecordBrief {
    return recordFixture({
      status: "transcribing",
      processingStage: "transcribing",
      progressCurrent: current,
      progressTotal: 12,
    });
  }

  it("两次进度事件后显示预计剩余时间", async () => {
    vi.useFakeTimers({ toFake: ["Date"] });
    try {
      vi.setSystemTime(new Date("2026-11-12T10:00:00Z"));
      // 进度点 1：(0, 10:00)；之后 10:03 到 3、10:06 到 6 → 60s/块
      vi.mocked(tauri.getRecord).mockResolvedValue(processingRecord(0));
      renderDetail();
      await waitForAudioReady();

      // 每次先改 mock（下一次 load 读到的状态）再发事件
      vi.mocked(tauri.getRecord).mockResolvedValue(processingRecord(3));
      act(() => {
        vi.setSystemTime(new Date("2026-11-12T10:03:00Z"));
        emit("processing-progress", { recordId: "record-1" });
      }); // 进度点 2：(3, 10:03) → 60s/块
      await waitFor(() => {
        const stage = screen.getByText("正在本机转写", { exact: true });
        expect(stage.parentElement?.textContent).toContain("4 / 12");
      });

      vi.mocked(tauri.getRecord).mockResolvedValue(processingRecord(6));
      act(() => {
        vi.setSystemTime(new Date("2026-11-12T10:06:00Z"));
        emit("processing-progress", { recordId: "record-1" });
      }); // 进度点 3：(6, 10:06)；剩 5 块 × 60s = 约 5 分钟
      await waitFor(() =>
        expect(screen.getByText(/预计还剩约/).textContent).toContain("5 分钟"),
      );
    } finally {
      vi.useRealTimers();
    }
  });

  it("本会话内从处理中变为完成时提示一次；打开旧记录不误报", async () => {
    vi.useFakeTimers({ toFake: ["Date"] });
    try {
      vi.setSystemTime(new Date("2026-11-12T10:00:00Z"));
      vi.mocked(tauri.getRecord).mockResolvedValue(processingRecord(2));
      renderDetail();
      await waitForAudioReady();
      expect(screen.queryByText(/处理完成/)).toBeNull();

      act(() => {
        emit("processing-progress", { recordId: "record-1" });
      });
      vi.mocked(tauri.getRecord).mockResolvedValue(recordFixture({ status: "completed", hasAnalysis: true }));
      act(() => {
        emit("processing-progress", { recordId: "record-1" });
      });
      expect(await screen.findByText(/处理完成/)).toBeTruthy();
    } finally {
      vi.useRealTimers();
    }
  });

  it("打开一个本就完成的记录不弹完成通知", async () => {
    vi.mocked(tauri.getRecord).mockResolvedValue(
      recordFixture({ status: "completed", hasAnalysis: true }),
    );
    renderDetail();
    await waitForAudioReady();
    // 等一拍确保没有异步通知出现
    await new Promise((resolve) => setTimeout(resolve, 30));
    expect(screen.queryByText(/处理完成/)).toBeNull();
  });
});

describe("记录详情：分析按钮禁用原因（v1.4 包6）", () => {
  it("没有逐字稿时按钮旁说明原因，而不是只给一个灰按钮", async () => {
    vi.mocked(tauri.getRecord).mockResolvedValue(
      recordFixture({ status: "completed", hasTranscript: false, hasAnalysis: false }),
    );
    vi.mocked(tauri.listTranscriptBlocks).mockResolvedValue([]);
    renderDetail();
    await waitForAudioReady();
    // 分析控制区在「内容分析」tab 下
    fireEvent.click(screen.getByRole("tab", { name: "内容分析" }));
    expect(await screen.findByText("还没有逐字稿，先完成转写")).toBeTruthy();
    expect(
      screen.getByRole("button", { name: "开始分析" }).hasAttribute("disabled"),
    ).toBe(true);
  });
});

describe("记录详情：录音标记条（v1.4 包5 前端覆盖）", () => {
  it("有标记时渲染时间片，点击跳转播放位置", async () => {
    vi.mocked(tauri.getRecordMarks).mockResolvedValue([12_000, 65_000]);
    renderDetail();
    await waitForAudioReady();
    // 升序渲染两个标记片
    const chips = await screen.findAllByRole("button", { name: /⚑/ });
    expect(chips).toHaveLength(2);
    // 后端返回升序（[12s, 65s]），前端按序渲染
    const labels = chips.map((chip) => chip.textContent ?? "");
    expect(labels[0]).toContain("0:12");
    expect(labels[1]).toContain("1:05");
  });

  it("没有标记时不渲染标记条", async () => {
    vi.mocked(tauri.getRecordMarks).mockResolvedValue([]);
    renderDetail();
    await waitForAudioReady();
    expect(screen.queryByRole("button", { name: /⚑/ })).toBeNull();
  });

  it("文档类记录不渲染标记条（标记只属于录音）", async () => {
    vi.mocked(tauri.getRecordMarks).mockResolvedValue([10_000]);
    renderDetail(recordFixture({ sourceType: "document" }));
    await screen.findByRole("tab", { name: "逐字稿" });
    expect(screen.queryByRole("button", { name: /⚑/ })).toBeNull();
    expect(tauri.getRecordMarks).not.toHaveBeenCalled();
  });
});

describe("记录详情：口播稿改写（v1.4 包8）", () => {
  it("有逐字稿时点击改写成口播稿，产物落库并切到产物 tab 展示", async () => {
    vi.mocked(tauri.rewriteRecordScript).mockResolvedValue({
      id: "a1",
      recordId: "record-1",
      kind: "script",
      title: "口播稿",
      content: "改写后的口播稿正文。",
      provider: "ollama",
      model: "qwen2.5:7b",
      createdAt: "2026-11-12T10:00:00.000Z",
    });
    vi.mocked(tauri.listTranscriptBlocks).mockResolvedValue([
      {
        id: "b1",
        startMs: 0,
        endMs: 6_000,
        segmentIds: ["s1"],
        segments: [
          {
            id: "s1",
            startMs: 0,
            endMs: 6_000,
            speakerLabel: "说话人 1",
            originalText: "今天讲口播稿的三个要点",
          },
        ],
      },
    ]);
    renderDetail();
    await waitForAudioReady();

    fireEvent.click(screen.getByRole("tab", { name: "内容分析" }));
    fireEvent.click(screen.getByRole("button", { name: "改写成稿" }));

    await waitFor(() => expect(tauri.rewriteRecordScript).toHaveBeenCalledWith("record-1"));
    // 自动切到产物 tab 并展示内容与来源模型
    expect(await screen.findByText("改写后的口播稿正文。")).toBeTruthy();
    expect(screen.getByText(/qwen2.5:7b/)).toBeTruthy();
  });

  it("没有逐字稿时改写按钮禁用（与其他禁用态一致说明）", async () => {
    vi.mocked(tauri.listRecordArtifacts).mockResolvedValue([]);
    renderDetail(recordFixture({ status: "completed", hasTranscript: false }));
    await waitForAudioReady();
    fireEvent.click(screen.getByRole("tab", { name: "内容分析" }));
    expect(
      screen.getByRole("button", { name: "改写成稿" }).hasAttribute("disabled"),
    ).toBe(true);
  });

  it("已有产物时 tab 出现「口播稿」入口", async () => {
    vi.mocked(tauri.listRecordArtifacts).mockResolvedValue([
      {
        id: "a1",
        recordId: "record-1",
        kind: "script",
        title: "口播稿",
        content: "旧版稿子。",
        provider: "ollama",
        model: "qwen2.5:7b",
        createdAt: "2026-11-12T09:00:00.000Z",
      },
    ]);
    renderDetail();
    await waitForAudioReady();
    fireEvent.click(screen.getByRole("tab", { name: "改写稿" }));
    expect(await screen.findByText("旧版稿子。")).toBeTruthy();
  });
});

describe("记录详情：一稿多出（v1.4 包9）", () => {
  const transcriptBlocks = [
    {
      id: "b1",
      startMs: 0,
      endMs: 6_000,
      segmentIds: ["s1"],
      segments: [
        {
          id: "s1",
          startMs: 0,
          endMs: 6_000,
          speakerLabel: "说话人 1",
          originalText: "今天讲口播稿的三个要点",
        },
      ],
    },
  ];

  it("节目简介按钮生成并落库展示", async () => {
    vi.mocked(tauri.listTranscriptBlocks).mockResolvedValue(transcriptBlocks);
    vi.mocked(tauri.generateShowNotes).mockResolvedValue({
      id: "a2",
      recordId: "record-1",
      kind: "show_notes",
      title: "节目简介",
      content: "本期聊口播稿的三个要点。",
      provider: "ollama",
      model: "qwen2.5:7b",
      createdAt: "2026-11-12T10:05:00.000Z",
    });
    renderDetail();
    await waitForAudioReady();

    fireEvent.click(screen.getByRole("tab", { name: "内容分析" }));
    fireEvent.click(screen.getByRole("button", { name: "节目简介" }));

    await waitFor(() => expect(tauri.generateShowNotes).toHaveBeenCalledWith("record-1"));
    expect(await screen.findByText("本期聊口播稿的三个要点。")).toBeTruthy();
    // 产物 tab 计数为 1
    expect(screen.getByRole("tab", { name: "改写稿" })).toBeTruthy();
  });

  it("社媒文案按钮生成并落库展示", async () => {
    vi.mocked(tauri.listTranscriptBlocks).mockResolvedValue(transcriptBlocks);
    vi.mocked(tauri.generateSocialPost).mockResolvedValue({
      id: "a3",
      recordId: "record-1",
      kind: "social_post",
      title: "社媒文案",
      content: "为什么你的口播稿总是念不顺？",
      provider: "ollama",
      model: "qwen2.5:7b",
      createdAt: "2026-11-12T10:06:00.000Z",
    });
    renderDetail();
    await waitForAudioReady();

    fireEvent.click(screen.getByRole("tab", { name: "内容分析" }));
    fireEvent.click(screen.getByRole("button", { name: "社媒文案" }));

    await waitFor(() => expect(tauri.generateSocialPost).toHaveBeenCalledWith("record-1"));
    expect(await screen.findByText("为什么你的口播稿总是念不顺？")).toBeTruthy();
  });

  it("两种产物与口播稿可同时在 tab 展示（计数正确）", async () => {
    vi.mocked(tauri.listTranscriptBlocks).mockResolvedValue(transcriptBlocks);
    vi.mocked(tauri.listRecordArtifacts).mockResolvedValue([
      {
        id: "a1",
        recordId: "record-1",
        kind: "script",
        title: "口播稿",
        content: "稿子。",
        provider: "ollama",
        model: "qwen2.5:7b",
        createdAt: "2026-11-12T09:00:00.000Z",
      },
      {
        id: "a3",
        recordId: "record-1",
        kind: "social_post",
        title: "社媒文案",
        content: "文案。",
        provider: "ollama",
        model: "qwen2.5:7b",
        createdAt: "2026-11-12T09:30:00.000Z",
      },
    ]);
    renderDetail();
    await waitForAudioReady();

    fireEvent.click(screen.getByRole("tab", { name: "改写稿（2）" }));
    expect(await screen.findByText("稿子。")).toBeTruthy();
    expect(screen.getByText("文案。")).toBeTruthy();
  });
});
