/**
 * 工作区「导入组折叠 + 内容归类」的行为测试。
 *
 * 盯住这次改动的三件承诺：
 *  1. 三种导入入口（自然录音 / 文档 / AI 对话）收进一个可折叠组：有内容默认收起、
 *     点击展开、偏好记入 localStorage；空资料库首次打开时自动展开；
 *  2. 内容按来源归类：AI 对话是「档案」不进「待处理」队列，徽标显示「对话档案」；
 *  3. 列表混有两种以上类型时出现筛选条，可按类型过滤。
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import RecordPanel from "../../src/components/RecordPanel";
import * as tauri from "../../src/lib/tauri";
import type { Project, RecordBrief } from "../../src/shared/types";

vi.mock("@tauri-apps/api/core", () => ({ isTauri: () => false }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: () => Promise.resolve(() => undefined),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: () => Promise.resolve(null) }));
vi.mock("@tauri-apps/api/webviewWindow", () => ({
  getCurrentWebviewWindow: () => ({ onDragDropEvent: () => Promise.resolve(() => undefined) }),
}));
vi.mock("../../src/lib/tauri");

function record(id: string, title: string, overrides: Partial<RecordBrief> = {}): RecordBrief {
  return {
    id,
    title,
    projectId: null,
    projectName: null,
    sourceType: "import",
    audioHash: `hash-${id}`,
    audioDurationMs: 60_000,
    importedAt: "2026-10-14T10:00:00.000Z",
    status: "completed",
    hasTranscript: true,
    hasAnalysis: true,
    analysisStatus: "completed",
    analysisHasQualityWarning: false,
    lastAnalysisError: null,
    analysisTemplateId: "builtin-standard",
    processingStage: null,
    progressCurrent: 0,
    progressTotal: 0,
    archivedAt: null,
    ...overrides,
  };
}

function renderPanel() {
  return render(
    <RecordPanel
      projectId={null}
      unfiledOnly={false}
      onImported={() => undefined}
      selectedId={null}
      onSelect={() => undefined}
      onPlay={() => undefined}
    />,
  );
}

beforeEach(() => {
  vi.resetAllMocks();
  window.localStorage.clear();
  vi.mocked(tauri.getInboxStatus).mockResolvedValue({
    watchFolders: [],
    usbDetection: false,
    counts: { pending: 0, imported: 0, failed: 0 },
    recentFiles: [],
  });
  vi.mocked(tauri.getDiskSpaceStatus).mockResolvedValue({
    availableBytes: 100 * 1024 * 1024 * 1024,
    lowSpaceThresholdBytes: 2 * 1024 * 1024 * 1024,
  });
  vi.mocked(tauri.listProjects).mockResolvedValue([] as Project[]);
  vi.mocked(tauri.listArchivedRecords).mockResolvedValue([]);
});

describe("工作区导入组与内容归类", () => {
  it("有内容的资料库默认收起导入组，展开后可见三种入口并记住偏好", async () => {
    vi.mocked(tauri.listRecords).mockResolvedValue([record("r1", "季度产品评审")]);
    renderPanel();
    await screen.findByText("季度产品评审");

    const header = screen.getByRole("button", { name: /导入内容/ });
    expect(header.getAttribute("aria-expanded")).toBe("false");
    // 收起时三种入口不占版面。
    expect(screen.queryByText("导入录音")).toBeNull();
    expect(screen.queryByText("导入 AI 对话")).toBeNull();

    fireEvent.click(header);
    expect(await screen.findByText("导入录音")).toBeTruthy();
    expect(screen.getByText("导入文档")).toBeTruthy();
    expect(screen.getByText("导入 AI 对话")).toBeTruthy();
    expect(header.getAttribute("aria-expanded")).toBe("true");
    expect(window.localStorage.getItem("workspace.import-collapse")).toBe("expanded");

    fireEvent.click(header);
    expect(screen.queryByText("导入录音")).toBeNull();
    expect(window.localStorage.getItem("workspace.import-collapse")).toBe("collapsed");
  });

  it("记住的收起偏好优先于空库默认展开", async () => {
    window.localStorage.setItem("workspace.import-collapse", "collapsed");
    vi.mocked(tauri.listRecords).mockResolvedValue([]);
    renderPanel();
    await screen.findByText(/当前没有待处理资料/);
    expect(screen.queryByText("导入录音")).toBeNull();
  });

  it("空资料库首次打开时自动展开导入组", async () => {
    vi.mocked(tauri.listRecords).mockResolvedValue([]);
    renderPanel();
    // 首次加载完成且一条记录都没有：导入组自动展开，引导首次导入。
    expect(await screen.findByText("导入录音")).toBeTruthy();
    expect(screen.getByText("导入文档")).toBeTruthy();
  });

  it("AI 对话不进待处理队列，在最近里显示「对话档案」", async () => {
    vi.mocked(tauri.listRecords).mockResolvedValue([
      record("r1", "季度产品评审"),
      record("c1", "AI 对话 2026-10-14 17:20", {
        sourceType: "chat",
        hasAnalysis: false,
        analysisStatus: null,
        lastAnalysisError: "分析失败",
      }),
    ]);
    renderPanel();
    await screen.findByText("季度产品评审");

    // 待处理只算真正的待办内容：对话不进来（此前 185 条对话会把队列淹没）。
    expect(screen.queryByText("AI 对话 2026-10-14 17:20")).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: /最近/ }));
    await screen.findByText("AI 对话 2026-10-14 17:20");
    expect(screen.getByText("对话档案")).toBeTruthy();
    // 对话的类目词不是待办状态：不再显示「分析失败」这类噪音。
    expect(screen.queryByText("分析失败")).toBeNull();
  });

  it("混有两种类型时出现筛选条，可按类型过滤", async () => {
    vi.mocked(tauri.listRecords).mockResolvedValue([
      record("r1", "季度产品评审"),
      record("d1", "需求说明文档", { sourceType: "document" }),
      record("c1", "AI 对话 2026-10-14 17:20", { sourceType: "chat", hasAnalysis: false }),
    ]);
    renderPanel();
    await screen.findByText("季度产品评审");

    const audioChip = screen.getByRole("button", { name: /^录音/ });
    expect(audioChip.textContent).toContain("1");
    expect(screen.getByRole("button", { name: /^文档/ }).textContent).toContain("1");

    fireEvent.click(audioChip);
    expect(screen.queryByText("需求说明文档")).toBeNull();
    expect(screen.queryByText("AI 对话 2026-10-14 17:20")).toBeNull();
    expect(screen.getByText("季度产品评审")).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: /^全部/ }));
    expect(screen.getByText("需求说明文档")).toBeTruthy();
  });

  it("只有单一类型时不出现筛选条", async () => {
    vi.mocked(tauri.listRecords).mockResolvedValue([
      record("r1", "季度产品评审"),
      record("r2", "客户访谈"),
    ]);
    renderPanel();
    await screen.findByText("季度产品评审");
    expect(screen.queryByRole("button", { name: /^全部/ })).toBeNull();
  });
});
