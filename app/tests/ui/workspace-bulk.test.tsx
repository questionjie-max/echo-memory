/**
 * 工作区批量管理的行为测试。
 *
 * 这里盯的是这次新增真正承诺的东西：
 *  1. 每行有勾选框，勾选不打开记录（不能一点就跳详情）；
 *  2. 表头能全选/取消全选，且只作用于当前可见列表；
 *  3. 勾选后出现操作条，能批量移动到指定知识库或移回未归档；
 *  4. 删除要弹确认，确认后调用批量删除并刷新列表；
 *  5. 列表刷新后，已消失的记录自动从勾选里去掉。
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import RecordPanel from "../../src/components/RecordPanel";
import * as tauri from "../../src/lib/tauri";
import type { Project, RecordBrief } from "../../src/shared/types";

vi.mock("@tauri-apps/api/core", () => ({ isTauri: () => false }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: () => Promise.resolve(() => undefined),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn(() => Promise.resolve(null)) }));
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
    importedAt: "2026-09-20T10:00:00.000Z",
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

const records = [
  record("r1", "季度产品评审"),
  record("r2", "客户访谈", { projectId: "p1", projectName: "产品研发" }),
  record("r3", "英文对照"),
];

function renderPanel() {
  const result = render(
    <RecordPanel
      projectId={null}
      unfiledOnly={false}
      onImported={() => undefined}
      selectedId={null}
      onSelect={() => undefined}
      onPlay={() => undefined}
    />,
  );
  // v1.4 包1：默认视图是「待处理」；这些测试操作的记录在新语义下属于「最近」视图。
  fireEvent.click(screen.getByRole("button", { name: /最近/ }));
  return result;
}

beforeEach(() => {
  vi.resetAllMocks();
  vi.mocked(tauri.getInboxStatus).mockResolvedValue({
    watchFolders: [],
    usbDetection: false,
    counts: { pending: 0, imported: 0, failed: 0 },
    seen: 0,
  });
  vi.mocked(tauri.listProjects).mockResolvedValue([
    { id: "p1", name: "产品研发", status: "active", createdAt: "", updatedAt: "" } as Project,
    { id: "p2", name: "客户与增长", status: "active", createdAt: "", updatedAt: "" } as Project,
  ]);
  vi.mocked(tauri.listArchivedRecords).mockResolvedValue([]);
  vi.mocked(tauri.moveRecords).mockResolvedValue(2);
  vi.mocked(tauri.deleteRecords).mockResolvedValue({
    deletedCount: 2,
    fileCleanupFailures: [],
  });
  vi.mocked(tauri.importAudio).mockResolvedValue({
    recordId: "x",
    hash: "h",
    duplicate: false,
    durationMs: 1,
    title: "x",
  });
  vi.mocked(tauri.transcribeRecord).mockResolvedValue(undefined);
});

describe("工作区批量管理", () => {
  it("默认显示最近列表，勾选不打开记录", async () => {
    vi.mocked(tauri.listRecords).mockResolvedValue(records);
    renderPanel();
    await screen.findByText("季度产品评审");

    // 待处理视图下这三条都已完成且有归属判断，切到「最近」才看得全。
    fireEvent.click(screen.getByRole("button", { name: /最近/ }));
    await screen.findByText("英文对照");

    fireEvent.click(screen.getByRole("checkbox", { name: "选择 季度产品评审" }));
    expect(tauri.getRecord).not.toHaveBeenCalled();
    await screen.findByText("已选 1 条");
  });

  it("区分完成提醒和阻断性分析状态", async () => {
    vi.mocked(tauri.listRecords).mockResolvedValue([
      record("advisory", "有质量提醒的录音", {
        analysisHasQualityWarning: true,
      }),
      record("blocking", "分析不完整的录音", {
        analysisStatus: "incomplete",
        hasAnalysis: false,
      }),
    ]);

    renderPanel();

    await screen.findByText("已完成 · 有质量提醒");
    await screen.findByText("分析不完整");
  });

  it("表头全选只作用于当前可见列表", async () => {
    vi.mocked(tauri.listRecords).mockResolvedValue(records);
    renderPanel();
    await screen.findByText("季度产品评审");
    fireEvent.click(screen.getByRole("button", { name: /最近/ }));
    await screen.findByText("英文对照");

    fireEvent.click(screen.getByRole("checkbox", { name: "全选当前列表" }));
    await screen.findByText("已选 3 条");

    // 再点一次取消全选，操作条收起。
    fireEvent.click(screen.getByRole("checkbox", { name: "全选当前列表" }));
    await waitFor(() => expect(screen.queryByText("已选 3 条")).toBeNull());
  });

  it("批量移动到选中的知识库", async () => {
    vi.mocked(tauri.listRecords).mockResolvedValue(records);
    renderPanel();
    await screen.findByText("季度产品评审");
    fireEvent.click(screen.getByRole("button", { name: /最近/ }));
    await screen.findByText("英文对照");

    fireEvent.click(screen.getByRole("checkbox", { name: "全选当前列表" }));
    await screen.findByText("已选 3 条");

    fireEvent.change(screen.getByLabelText("选择目标知识库"), { target: { value: "p2" } });
    fireEvent.click(screen.getByRole("button", { name: "移动" }));

    await waitFor(() =>
      expect(tauri.moveRecords).toHaveBeenCalledWith(["r1", "r2", "r3"], "p2"),
    );
    await screen.findByText("已将 2 条记录移动到「客户与增长」。");
    // 移动后清空勾选，避免用户对同一批记录重复操作。
    await waitFor(() => expect(screen.queryByText("已选 3 条")).toBeNull());
  });

  it("删除需要确认，确认后才真的删", async () => {
    vi.mocked(tauri.listRecords).mockResolvedValue(records);
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(false);
    renderPanel();
    await screen.findByText("季度产品评审");
    fireEvent.click(screen.getByRole("button", { name: /最近/ }));
    await screen.findByText("英文对照");

    fireEvent.click(screen.getByRole("checkbox", { name: "选择 季度产品评审" }));
    await screen.findByText("已选 1 条");
    fireEvent.click(screen.getByRole("button", { name: "删除" }));

    expect(confirm).toHaveBeenCalled();
    expect(tauri.deleteRecords).not.toHaveBeenCalled();

    confirm.mockReturnValue(true);
    fireEvent.click(screen.getByRole("checkbox", { name: "选择 英文对照" }));
    await screen.findByText("已选 2 条");
    fireEvent.click(screen.getByRole("button", { name: "删除" }));

    await waitFor(() => expect(tauri.deleteRecords).toHaveBeenCalledWith(["r1", "r3"]));
    await screen.findByText("已删除 2 条记录及其音频文件。");
    confirm.mockRestore();
  });

  it("列表刷新后自动清掉已消失记录的勾选", async () => {
    // 勾选清理真正发挥作用的场景：转写中的轮询拉到了新列表，某条记录被别处删掉了。
    // 用假计时器推进那 2 秒轮询。
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      const withProcessing = [
        record("r1", "季度产品评审", { status: "transcribing" }),
        record("r2", "客户访谈"),
        record("r3", "英文对照"),
      ];
      vi.mocked(tauri.listRecords).mockResolvedValue(withProcessing);
      renderPanel();
      await screen.findByText("季度产品评审");
      await screen.findByText("英文对照");

      fireEvent.click(screen.getByRole("checkbox", { name: "全选当前列表" }));
      await screen.findByText("已选 3 条");

      // 轮询拉到的新列表里少了一条。
      vi.mocked(tauri.listRecords).mockResolvedValue(withProcessing.slice(0, 2));
      await act(async () => {
        await vi.advanceTimersByTimeAsync(2_500);
      });

      await waitFor(() => expect(screen.queryByText("英文对照")).toBeNull());
      // 消失的那条从勾选里去掉，还活着的两条仍然选中——不会误伤用户的选择。
      await screen.findByText("已选 2 条");
      expect(tauri.moveRecords).not.toHaveBeenCalled();
      expect(tauri.deleteRecords).not.toHaveBeenCalled();
    } finally {
      vi.useRealTimers();
    }
  });

  it("数据库删除失败时如实报错，不清空勾选", async () => {
    vi.mocked(tauri.listRecords).mockResolvedValue(records);
    vi.mocked(tauri.deleteRecords).mockRejectedValue(new Error("数据库事务失败"));
    vi.spyOn(window, "confirm").mockReturnValue(true);
    renderPanel();
    await screen.findByText("季度产品评审");
    fireEvent.click(screen.getByRole("button", { name: /最近/ }));
    await screen.findByText("英文对照");

    fireEvent.click(screen.getByRole("checkbox", { name: "选择 季度产品评审" }));
    await screen.findByText("已选 1 条");
    fireEvent.click(screen.getByRole("button", { name: "删除" }));

    await screen.findByText("数据库事务失败");
    expect(screen.getByText("已选 1 条")).toBeTruthy();
  });

  it("数据库删除成功后遗留文件只显示警告并清空选择", async () => {
    vi.mocked(tauri.listRecords).mockResolvedValue(records);
    vi.mocked(tauri.deleteRecords).mockResolvedValue({
      deletedCount: 1,
      fileCleanupFailures: ["/library/audio/r1.m4a"],
    });
    vi.spyOn(window, "confirm").mockReturnValue(true);
    renderPanel();
    await screen.findByText("季度产品评审");
    fireEvent.click(screen.getByRole("button", { name: /最近/ }));
    await screen.findByText("英文对照");

    fireEvent.click(screen.getByRole("checkbox", { name: "选择 季度产品评审" }));
    fireEvent.click(screen.getByRole("button", { name: "删除" }));

    await screen.findByText("已删除 1 条记录，但有 1 个文件遗留。");
    await waitFor(() => expect(screen.queryByText("已选 1 条")).toBeNull());
  });
});

describe("v1.4 包7：批量导出包", () => {
  beforeEach(() => {
    vi.resetAllMocks();
    vi.mocked(tauri.getInboxStatus).mockResolvedValue({
      watchFolders: [],
      usbDetection: false,
      counts: { pending: 0, imported: 0, failed: 0 },
      seen: 0,
    });
    vi.mocked(tauri.listProjects).mockResolvedValue([]);
    vi.mocked(tauri.listArchivedRecords).mockResolvedValue([]);
    vi.mocked(tauri.listAnalysisTemplates).mockResolvedValue([]);
    vi.mocked(tauri.getMcpStatus).mockResolvedValue({
      enabled: false,
      authorizedScope: "",
      executableAvailable: true,
      executablePath: null,
      recentCalls: [],
    });
    vi.mocked(tauri.listRecords).mockResolvedValue(records);
    vi.mocked(tauri.createExportTicket).mockResolvedValue("ticket-bulk");
    vi.mocked(tauri.exportRecordPackages).mockResolvedValue({
      exported: [
        { recordId: "r1", folder: "/tmp/out/导出包-季度产品评审" },
        { recordId: "r2", folder: "/tmp/out/导出包-客户访谈" },
      ],
      skipped: [{ recordId: "r3", reason: "该记录还没有逐字稿，无法导出包" }],
      destination: "/tmp/out",
    });
  });

  it("选中记录后批量条出现导出包入口，点击后先领票据再选文件夹", async () => {
    const { open } = await import("@tauri-apps/plugin-dialog");
    vi.mocked(open).mockResolvedValue("/tmp/out");
    renderPanel();

    // 切到「最近」并勾选两条（复选框按渲染行序，取前两个）
    fireEvent.click(screen.getByRole("button", { name: /最近/ }));
    await screen.findByText("季度产品评审");
    const boxes = screen.getAllByRole("checkbox");
    fireEvent.click(boxes[0]);
    fireEvent.click(boxes[1]);
    fireEvent.click(screen.getByRole("button", { name: "导出包" }));

    await waitFor(() => expect(tauri.createExportTicket).toHaveBeenCalled());
    await waitFor(() => expect(open).toHaveBeenCalled());
    await waitFor(() => {
      const call = vi.mocked(tauri.exportRecordPackages).mock.calls[0];
      expect(call[0]).toHaveLength(2); // 勾了两条
      expect(call[1]).toBe("/tmp/out");
      expect(call[2]).toBe("ticket-bulk");
    });
    // 回报：成功 N 条 + 跳过 M 条及原因方向
    expect(await screen.findByText(/已导出 2 条/)).toBeTruthy();
    expect(screen.getByText(/1 条跳过/)).toBeTruthy();
    // 导出后清空勾选
    expect(screen.queryByRole("button", { name: "导出包" })).toBeNull();
  });

  it("用户取消文件夹选择时不发起导出", async () => {
    const { open } = await import("@tauri-apps/plugin-dialog");
    vi.mocked(open).mockResolvedValue(null);
    renderPanel();

    fireEvent.click(screen.getByRole("button", { name: /最近/ }));
    await screen.findByText("季度产品评审");
    fireEvent.click(screen.getAllByRole("checkbox")[0]);
    fireEvent.click(screen.getByRole("button", { name: "导出包" }));

    await waitFor(() => expect(open).toHaveBeenCalled());
    expect(tauri.exportRecordPackages).not.toHaveBeenCalled();
  });

  it("全部成功时不显示跳过提示", async () => {
    const { open } = await import("@tauri-apps/plugin-dialog");
    vi.mocked(open).mockResolvedValue("/tmp/out");
    vi.mocked(tauri.exportRecordPackages).mockResolvedValue({
      exported: [{ recordId: "r1", folder: "/tmp/out/导出包-季度产品评审" }],
      skipped: [],
      destination: "/tmp/out",
    });
    renderPanel();

    fireEvent.click(screen.getByRole("button", { name: /最近/ }));
    await screen.findByText("季度产品评审");
    fireEvent.click(screen.getAllByRole("checkbox")[0]);
    fireEvent.click(screen.getByRole("button", { name: "导出包" }));

    expect(await screen.findByText(/已导出 1 条/)).toBeTruthy();
    expect(screen.queryByText(/跳过/)).toBeNull();
  });
});
