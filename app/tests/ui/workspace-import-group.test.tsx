/**
 * 工作区「内容分组」的行为测试。
 *
 * 三种输入方式即三类内容，各自是一个可折叠的分类组：组头即该类入口，组内即该类记录。
 * 盯着这几件承诺：
 *  1. 录音 / 文档 / AI 对话 三组默认展开，组头显示计数、格式提示与该类导入按钮；
 *  2. 点击组头收起/展开该组内容，状态记入 localStorage 并在下次挂载生效；
 *  3. 分组不改变行内行为：勾选、播放、更多菜单、批量选择照旧；
 *  4. AI 对话不进「待处理」（那是待办队列），在「最近」里显示类目词「对话档案」；
 *  5. 归档视图保留分组但不再显示导入按钮。
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

const mixedRecords = [
  record("r1", "季度产品评审会议"),
  record("r2", "客户访谈：房产系统二期"),
  record("d1", "需求说明与验收口径", { sourceType: "document" }),
  record("c1", "AI 对话 2026-10-14 17:20", { sourceType: "chat", hasAnalysis: false, analysisStatus: null }),
  record("c2", "AI 对话 2026-10-13 18:03", { sourceType: "chat", hasAnalysis: false, analysisStatus: null }),
];

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

describe("工作区内容分组", () => {
  it("三组默认展开，组头带计数、格式提示与该类导入按钮", async () => {
    vi.mocked(tauri.listRecords).mockResolvedValue(mixedRecords);
    renderPanel();
    await screen.findByText("季度产品评审会议");

    // 切到「最近」：三组齐全（待处理视图不含 AI 对话，见下一条用例）。
    fireEvent.click(screen.getByRole("button", { name: /最近/ }));
    await screen.findByText("AI 对话 2026-10-14 17:20");

    expect(screen.getByRole("button", { name: "录音分组（2 条）" }).getAttribute("aria-expanded")).toBe("true");
    expect(screen.getByRole("button", { name: "文档分组（1 条）" }).getAttribute("aria-expanded")).toBe("true");
    expect(screen.getByRole("button", { name: "AI 对话分组（2 条）" }).getAttribute("aria-expanded")).toBe("true");

    // 组头显示该类支持的格式提示。
    expect(screen.getByText(/MP3、M4A、WAV、AAC/)).toBeTruthy();
    expect(screen.getByText(/Markdown、TXT、Word 文档/)).toBeTruthy();

    // 组头右侧就是该类的导入入口。
    expect(screen.getByRole("button", { name: "选择音频" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "选择文档" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "选择导出" })).toBeTruthy();
  });

  it("点击组头收起该类内容并记住，再点恢复", async () => {
    vi.mocked(tauri.listRecords).mockResolvedValue(mixedRecords);
    renderPanel();
    await screen.findByText("季度产品评审会议");
    fireEvent.click(screen.getByRole("button", { name: /最近/ }));
    await screen.findByText("需求说明与验收口径");

    const documentHeader = screen.getByRole("button", { name: "文档分组（1 条）" });
    fireEvent.click(documentHeader);

    // 内容收起：该类记录行不再渲染，但组头与格式提示仍在。
    expect(screen.queryByText("需求说明与验收口径")).toBeNull();
    expect(screen.queryByText(/Markdown、TXT、Word 文档/)).toBeTruthy();
    expect(documentHeader.getAttribute("aria-expanded")).toBe("false");
    expect(window.localStorage.getItem("workspace.group-collapse")).toBe('["document"]');

    // 其他组不受影响。
    expect(screen.getByText("季度产品评审会议")).toBeTruthy();

    fireEvent.click(documentHeader);
    expect(await screen.findByText("需求说明与验收口径")).toBeTruthy();
    expect(window.localStorage.getItem("workspace.group-collapse")).toBe("[]");
  });

  it("挂载时应用已记住的收起状态", async () => {
    window.localStorage.setItem("workspace.group-collapse", '["chat"]');
    vi.mocked(tauri.listRecords).mockResolvedValue(mixedRecords);
    renderPanel();
    await screen.findByText("季度产品评审会议");
    fireEvent.click(screen.getByRole("button", { name: /最近/ }));
    await screen.findByText("季度产品评审会议");

    expect(screen.getByRole("button", { name: "AI 对话分组（2 条）" }).getAttribute("aria-expanded")).toBe("false");
    expect(screen.queryByText("AI 对话 2026-10-14 17:20")).toBeNull();
    expect(screen.getByText("需求说明与验收口径")).toBeTruthy();
  });

  it("分组不改变行内行为：勾选仍可批量操作，全选作用于当前视图", async () => {
    vi.mocked(tauri.listRecords).mockResolvedValue(mixedRecords.slice(0, 3));
    renderPanel();
    await screen.findByText("季度产品评审会议");

    fireEvent.click(screen.getByRole("checkbox", { name: "选择 季度产品评审会议" }));
    await screen.findByText("已选 1 条");

    fireEvent.click(screen.getByRole("checkbox", { name: "全选当前列表" }));
    await screen.findByText("已选 3 条");
  });

  it("AI 对话不进待处理队列，在最近里显示「对话档案」", async () => {
    vi.mocked(tauri.listRecords).mockResolvedValue(mixedRecords);
    renderPanel();
    await screen.findByText("季度产品评审会议");

    // 待处理只可能是录音与文档：对话组不出现。
    expect(screen.queryByRole("button", { name: "AI 对话分组（2 条）" })).toBeNull();
    expect(screen.queryByText("AI 对话 2026-10-14 17:20")).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: /最近/ }));
    await screen.findByText("AI 对话 2026-10-14 17:20");
    // 对话的类目词是「对话档案」而不是待办状态。
    expect(screen.getAllByText("对话档案").length).toBe(2);
    expect(screen.queryByText("待分析")).toBeNull();
  });

  it("无待处理内容时最近视图仍给全三组入口，归档视图不显示导入按钮", async () => {
    vi.mocked(tauri.listRecords).mockResolvedValue([]);
    vi.mocked(tauri.listArchivedRecords).mockResolvedValue([
      record("a1", "已归档的访谈", { archivedAt: "2026-10-01T00:00:00.000Z" }),
    ]);
    renderPanel();
    // 待处理视图：录音 / 文档两组入口仍在，组内提示可从这里导入。
    expect(await screen.findByRole("button", { name: "选择音频" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "选择文档" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "选择导出" })).toBeNull();
    expect(screen.getAllByText("这一类暂无内容，可从上方导入。").length).toBe(2);

    // 切到最近：三组齐全（AI 对话组此时才有意义）。
    fireEvent.click(screen.getByRole("button", { name: /最近/ }));
    expect(screen.getByRole("button", { name: "选择导出" })).toBeTruthy();
    expect(screen.getAllByText("这一类暂无内容，可从上方导入。").length).toBe(3);

    // 归档视图：保留分组但不给导入按钮，行徽标固定为「已归档」。
    fireEvent.click(screen.getByRole("button", { name: /归档/ }));
    await screen.findByText("已归档的访谈");
    expect(screen.queryByRole("button", { name: "选择音频" })).toBeNull();
    expect(screen.getByRole("button", { name: "录音分组（1 条）" })).toBeTruthy();
    expect(screen.getByText("已归档")).toBeTruthy();
  });
});
