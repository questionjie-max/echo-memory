/**
 * 「待处理」语义测试（v1.4 包1）。
 *
 * 承诺的行为：
 *  1. 加工完成（有分析且非 stale/incomplete）的未归档记录**不算待处理**——
 *     它们出现在「最近」视图，徽标「待归档」；
 *  2. 分析失败、分析需要更新（stale）、分析不完整的记录仍在待处理；
 *  3. 存在待归档记录时出现「一键归档」入口：选目标知识库 → 确认 → 批量移动；
 *  4. 对话档案永不进待处理。
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
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

const projects = [
  { id: "p1", name: "产品研发", status: "active", createdAt: "", updatedAt: "" } as Project,
];

beforeEach(() => {
  vi.resetAllMocks();
  vi.mocked(tauri.getInboxStatus).mockResolvedValue({
    watchFolders: [],
    usbDetection: false,
    counts: { pending: 0, imported: 0, failed: 0 },
    seen: 0,
  });
  vi.mocked(tauri.listProjects).mockResolvedValue(projects);
  vi.mocked(tauri.listArchivedRecords).mockResolvedValue([]);
  vi.mocked(tauri.listAnalysisTemplates).mockResolvedValue([]);
  vi.mocked(tauri.getMcpStatus).mockResolvedValue({
    enabled: false,
    authorizedScope: "",
    executableAvailable: true,
    executablePath: null,
    recentCalls: [],
  });
});

describe("待处理语义（v1.4 包1）", () => {
  it("已分析未归档的记录不算待处理：进已完成视图、徽标待归档", async () => {
    vi.mocked(tauri.listRecords).mockResolvedValue([
      record("r1", "季度产品评审"),
    ]);
    renderPanel();

    // 待处理计数为 0
    await screen.findByText("待处理");
    expect(screen.getByText("待处理").textContent).toContain("0");
    // 默认视图是待处理（空），切到「最近」看记录行
    fireEvent.click(screen.getByRole("button", { name: /最近/ }));
    // 徽标显示「待归档」而不是「待处理」
    expect(await screen.findByText("待归档")).toBeTruthy();
  });

  it("分析失败的记录仍在待处理", async () => {
    vi.mocked(tauri.listRecords).mockResolvedValue([
      record("r1", "转写坏了", { hasAnalysis: false, lastAnalysisError: "模型未响应" }),
    ]);
    renderPanel();

    await screen.findByText("待处理");
    expect(screen.getByText("待处理").textContent).toMatch(/待处理\s*[1-9]/);
    expect(await screen.findByText("分析失败")).toBeTruthy();
  });

  it("分析需要更新（stale）的记录仍在待处理", async () => {
    vi.mocked(tauri.listRecords).mockResolvedValue([
      record("r1", "模板改了", { analysisStatus: "stale" }),
    ]);
    renderPanel();

    await screen.findByText("待处理");
    expect(screen.getByText("待处理").textContent).toMatch(/待处理\s*[1-9]/);
    expect(await screen.findByText("分析需要更新")).toBeTruthy();
  });

  it("存在待归档记录时出现一键归档入口，选库后批量移动", async () => {
    vi.mocked(tauri.listRecords).mockResolvedValue([
      record("r1", "季度产品评审"),
      record("r2", "客户访谈", { projectId: "p1", projectName: "产品研发" }),
    ]);
    vi.mocked(tauri.moveRecords).mockResolvedValue(1);
    renderPanel();

    const entry = await screen.findByRole("button", { name: /一键归档/ });
    expect(entry.textContent).toContain("1");

    fireEvent.change(screen.getByLabelText("归档目标知识库"), {
      target: { value: "p1" },
    });
    fireEvent.click(entry);
    await waitFor(() =>
      expect(tauri.moveRecords).toHaveBeenCalledWith(["r1"], "p1"),
    );
    // 完成后刷新列表
    await waitFor(() => expect(tauri.listRecords).toHaveBeenCalled());
  });

  it("没有待归档记录时不出现一键归档入口", async () => {
    vi.mocked(tauri.listRecords).mockResolvedValue([
      record("r1", "客户访谈", { projectId: "p1", projectName: "产品研发" }),
    ]);
    renderPanel();

    fireEvent.click(await screen.findByRole("button", { name: /最近/ }));
    await screen.findByText("客户访谈");
    expect(screen.queryByRole("button", { name: /一键归档/ })).toBeNull();
  });

  it("未选择归档目标库时按钮禁用且不会触发移动（评审 P2 守卫锁定）", async () => {
    vi.mocked(tauri.listRecords).mockResolvedValue([
      record("r1", "季度产品评审"),
    ]);
    renderPanel();

    const entry = await screen.findByRole("button", { name: /一键归档/ });
    expect(entry.hasAttribute("disabled")).toBe(true);
    fireEvent.click(entry);
    expect(tauri.moveRecords).not.toHaveBeenCalled();
    // 选择目标库后按钮可用
    fireEvent.change(screen.getByLabelText("归档目标知识库"), {
      target: { value: "p1" },
    });
    expect(screen.getByRole("button", { name: /一键归档/ }).hasAttribute("disabled")).toBe(false);
  });

  it("对话档案永不进待处理", async () => {
    vi.mocked(tauri.listRecords).mockResolvedValue([
      // 已过导入确认门的 chat 记录：类目徽标「对话档案」，绝不进待处理队列
      record("c1", "AI 对话", { sourceType: "chat", importConfirmedAt: "2026-09-20T10:00:00.000Z" }),
    ]);
    renderPanel();

    await screen.findByText("待处理");
    expect(screen.getByText("待处理").textContent).toContain("0");
    fireEvent.click(screen.getByRole("button", { name: /最近/ }));
    expect(await screen.findByText("对话档案")).toBeTruthy();
  });
});
