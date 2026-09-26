/**
 * 单条记录的右键菜单行为。
 *
 * 这组测试锁定工作区的高频管理路径：右键直接操作，不再依赖逐条勾选；
 * 归档记录有独立入口，危险删除必须确认，失败必须留在菜单内可见。
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
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

const activeRecord = record("r1", "季度产品评审");
const archivedRecord = record("r2", "历史客户访谈", {
  projectId: "p1",
  projectName: "产品研发",
  archivedAt: "2026-09-21T10:00:00.000Z",
});

async function renderPanel(onSelect = vi.fn()) {
  const result = render(
    <RecordPanel
      projectId={null}
      unfiledOnly={false}
      onImported={() => undefined}
      selectedId={null}
      onSelect={onSelect}
      onPlay={() => undefined}
    />,
  );
  fireEvent.click(screen.getByRole("button", { name: /最近/ }));
  await screen.findByText(activeRecord.title);
  return { ...result, onSelect };
}

function openMenu(title: string) {
  fireEvent.contextMenu(screen.getByText(title));
  return screen.getByRole("menu", { name: `${title}操作菜单` });
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
  vi.mocked(tauri.listRecords).mockResolvedValue([activeRecord]);
  vi.mocked(tauri.listArchivedRecords).mockResolvedValue([]);
  vi.mocked(tauri.moveRecords).mockResolvedValue(1);
  vi.mocked(tauri.updateRecordTitle).mockResolvedValue(activeRecord);
  vi.mocked(tauri.setRecordArchived).mockResolvedValue(activeRecord);
  vi.mocked(tauri.deleteRecords).mockResolvedValue({
    deletedCount: 1,
    fileCleanupFailures: [],
  });
});

describe("工作区单条记录菜单", () => {
  it("右键打开菜单并可通过打开管理进入记录", async () => {
    const { onSelect } = await renderPanel();
    openMenu("季度产品评审");

    fireEvent.click(screen.getByRole("button", { name: "打开管理" }));

    expect(onSelect).toHaveBeenCalledWith(activeRecord);
    expect(screen.queryByRole("menu")).toBeNull();
  });

  it("在菜单内把单条记录转移到知识库", async () => {
    await renderPanel();
    openMenu("季度产品评审");
    fireEvent.click(screen.getByRole("button", { name: "转移知识库" }));
    fireEvent.change(screen.getByLabelText("单条记录目标知识库"), { target: { value: "p2" } });
    fireEvent.click(screen.getByRole("button", { name: "移动" }));

    await waitFor(() => expect(tauri.moveRecords).toHaveBeenCalledWith(["r1"], "p2"));
    await waitFor(() => expect(screen.queryByRole("menu")).toBeNull());
  });

  it("在菜单内重命名记录", async () => {
    await renderPanel();
    openMenu("季度产品评审");
    fireEvent.click(screen.getByRole("button", { name: "重命名" }));
    fireEvent.change(screen.getByLabelText("记录名称"), { target: { value: "九月产品评审" } });
    fireEvent.click(screen.getByRole("button", { name: "保存" }));

    await waitFor(() => expect(tauri.updateRecordTitle).toHaveBeenCalledWith("r1", "九月产品评审"));
    await waitFor(() => expect(screen.queryByRole("menu")).toBeNull());
  });

  it("归档记录后可从归档视图恢复", async () => {
    vi.mocked(tauri.listArchivedRecords).mockResolvedValue([archivedRecord]);
    await renderPanel();
    const menu = openMenu("季度产品评审");
    fireEvent.click(within(menu).getByRole("button", { name: "归档" }));

    await waitFor(() => expect(tauri.setRecordArchived).toHaveBeenCalledWith("r1", true));
    fireEvent.click(screen.getByRole("button", { name: /^归档\s/ }));
    await screen.findByText("历史客户访谈");
    expect(tauri.listArchivedRecords).toHaveBeenCalled();
    expect(screen.queryByLabelText("全选当前列表")).toBeNull();
    expect(screen.queryByRole("button", { name: "选择音频" })).toBeNull();

    openMenu("历史客户访谈");
    fireEvent.click(screen.getByRole("button", { name: "恢复" }));
    await waitFor(() => expect(tauri.setRecordArchived).toHaveBeenCalledWith("r2", false));
  });

  it("删除必须确认，拒绝时完全不调用删除", async () => {
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(false);
    await renderPanel();
    openMenu("季度产品评审");

    fireEvent.click(screen.getByRole("button", { name: "删除" }));

    expect(confirm).toHaveBeenCalled();
    expect(tauri.deleteRecords).not.toHaveBeenCalled();
    confirm.mockRestore();
  });

  it("操作失败时在菜单内显示错误并保持菜单打开", async () => {
    vi.mocked(tauri.updateRecordTitle).mockRejectedValue(new Error("更新失败"));
    await renderPanel();
    openMenu("季度产品评审");
    fireEvent.click(screen.getByRole("button", { name: "重命名" }));
    fireEvent.change(screen.getByLabelText("记录名称"), { target: { value: "新产品评审" } });
    fireEvent.click(screen.getByRole("button", { name: "保存" }));

    await screen.findByRole("alert");
    expect(screen.getByRole("alert").textContent).toContain("更新失败");
    expect(screen.getByRole("menu")).toBeTruthy();
  });

  it("Escape 关闭菜单", async () => {
    await renderPanel();
    openMenu("季度产品评审");

    fireEvent.keyDown(document, { key: "Escape" });

    expect(screen.queryByRole("menu")).toBeNull();
  });
});
