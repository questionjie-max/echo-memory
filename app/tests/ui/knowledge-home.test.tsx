import { beforeEach, describe, expect, it, vi } from "vitest";
import { act, render, screen, waitFor } from "@testing-library/react";
import KnowledgeHome from "../../src/components/KnowledgeHome";
import * as tauri from "../../src/lib/tauri";
import type { KnowledgeIndexStatus, KnowledgeOverview } from "../../src/shared/types";

const eventMock = vi.hoisted(() => ({
  handlers: new Map<string, Set<(event: { payload: unknown }) => void>>(),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: (name: string, handler: (event: { payload: unknown }) => void) => {
    const handlers = eventMock.handlers.get(name) ?? new Set();
    handlers.add(handler);
    eventMock.handlers.set(name, handlers);
    return Promise.resolve(() => handlers.delete(handler));
  },
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn(() => Promise.resolve(null)) }));
vi.mock("../../src/lib/tauri");

function overviewFixture(overrides: Partial<KnowledgeOverview> = {}): KnowledgeOverview {
  return {
    recordCount: 3,
    transcriptCount: 3,
    analyzedCount: 2,
    decisions: [],
    actionItems: [],
    ...overrides,
  };
}

function indexFixture(overrides: Partial<KnowledgeIndexStatus> = {}): KnowledgeIndexStatus {
  return {
    scopeKey: "all",
    status: "completed",
    totalRecords: 3,
    processedRecords: 3,
    chunkCount: 7,
    embeddingModel: "test-embedding",
    lastError: null,
    updatedAt: "2026-09-23T10:00:00.000Z",
    ...overrides,
  };
}

function emit(name: string, payload: unknown = {}) {
  act(() => {
    eventMock.handlers.get(name)?.forEach((handler) => handler({ payload }));
  });
}

function renderHome() {
  return render(
    <KnowledgeHome
      scope="all"
      projectId={null}
      unfiledOnly={false}
      refreshKey={0}
      onOpenCitation={() => undefined}
      onOpenKnowledgeChat={() => undefined}
    />,
  );
}

beforeEach(() => {
  vi.resetAllMocks();
  eventMock.handlers.clear();
  vi.mocked(tauri.getKnowledgeOverview).mockResolvedValue(overviewFixture());
  vi.mocked(tauri.getKnowledgeIndexStatus).mockResolvedValue(indexFixture());
  vi.mocked(tauri.listProjects).mockResolvedValue([]);
});

describe("知识库主页", () => {
  it("加载期间显示进度状态", async () => {
    vi.mocked(tauri.getKnowledgeOverview).mockReturnValue(new Promise(() => undefined));
    vi.mocked(tauri.getKnowledgeIndexStatus).mockReturnValue(new Promise(() => undefined));
    vi.mocked(tauri.listProjects).mockReturnValue(new Promise(() => undefined));
    renderHome();

    expect(await screen.findByRole("status")).toBeTruthy();
    expect(screen.getByText("正在加载知识库概览…")).toBeTruthy();
  });

  it("读取失败时显示可诊断错误", async () => {
    vi.mocked(tauri.getKnowledgeOverview).mockRejectedValue(new Error("概览库不可用"));
    renderHome();

    expect((await screen.findByRole("alert")).textContent).toContain("概览库不可用");
  });

  it("空资料库显示零指标和空引用", async () => {
    vi.mocked(tauri.getKnowledgeOverview).mockResolvedValue(
      overviewFixture({ recordCount: 0, transcriptCount: 0, analyzedCount: 0 }),
    );
    vi.mocked(tauri.getKnowledgeIndexStatus).mockResolvedValue(
      indexFixture({ status: "not_built", totalRecords: 0, processedRecords: 0, chunkCount: 0 }),
    );
    renderHome();

    await screen.findByText(/^尚未建立/);
    expect(screen.getByText("还没有可验证的决策。")).toBeTruthy();
    expect(screen.getByText("还没有明确待办。")).toBeTruthy();
  });

  it("完成索引不创建 1 秒常驻轮询", async () => {
    const timeout = vi.spyOn(window, "setTimeout");
    renderHome();
    await screen.findByText("知识索引");

    const appPollTimers = timeout.mock.calls.filter(
      ([callback, delay]) => delay === 1_000 && callback.name !== "handleTimeout",
    );
    expect(appPollTimers).toHaveLength(0);
    timeout.mockRestore();
  });

  it("索引事件立即刷新概览", async () => {
    vi.mocked(tauri.getKnowledgeIndexStatus).mockResolvedValue(
      indexFixture({ status: "indexing", processedRecords: 0 }),
    );
    renderHome();
    await screen.findByText(/^0 \/ 3 条录音/);
    const before = vi.mocked(tauri.getKnowledgeOverview).mock.calls.length;

    emit("knowledge-index-update");

    await waitFor(() =>
      expect(vi.mocked(tauri.getKnowledgeOverview).mock.calls.length).toBeGreaterThan(before),
    );
  });

  it("事件丢失时 1 秒兜底继续刷新", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      vi.mocked(tauri.getKnowledgeIndexStatus).mockResolvedValue(
        indexFixture({ status: "indexing", processedRecords: 0 }),
      );
      renderHome();
      await screen.findByText(/^0 \/ 3 条录音/);
      const before = vi.mocked(tauri.getKnowledgeOverview).mock.calls.length;

      await act(async () => {
        await vi.advanceTimersByTimeAsync(1_100);
      });

      expect(vi.mocked(tauri.getKnowledgeOverview).mock.calls.length).toBeGreaterThan(before);
    } finally {
      vi.useRealTimers();
    }
  });
});
