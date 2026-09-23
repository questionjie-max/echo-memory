import { beforeEach, describe, expect, it, vi } from "vitest";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import KnowledgeChatView from "../../src/components/KnowledgeChatView";
import * as tauri from "../../src/lib/tauri";
import type { KnowledgeIndexStatus } from "../../src/shared/types";

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
vi.mock("../../src/lib/tauri");

function indexFixture(overrides: Partial<KnowledgeIndexStatus> = {}): KnowledgeIndexStatus {
  return {
    scopeKey: "all",
    status: "completed",
    totalRecords: 2,
    processedRecords: 2,
    chunkCount: 4,
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

function renderChat() {
  return render(
    <KnowledgeChatView
      scope="all"
      projectId={null}
      unfiledOnly={false}
      refreshKey={0}
      onOpenCitation={() => undefined}
      onOpenSettings={() => undefined}
    />,
  );
}

beforeEach(() => {
  vi.resetAllMocks();
  eventMock.handlers.clear();
  Element.prototype.scrollIntoView = vi.fn();
  vi.mocked(tauri.getKnowledgeIndexStatus).mockResolvedValue(indexFixture());
  vi.mocked(tauri.listProjects).mockResolvedValue([]);
});

describe("知识库问答视图", () => {
  it("状态读取期间显示加载态", async () => {
    vi.mocked(tauri.getKnowledgeIndexStatus).mockReturnValue(new Promise(() => undefined));
    renderChat();

    expect(await screen.findByText("正在读取知识库状态")).toBeTruthy();
  });

  it("状态读取失败时显示错误并可重新加载", async () => {
    vi.mocked(tauri.getKnowledgeIndexStatus).mockRejectedValue(new Error("状态库不可用"));
    renderChat();

    expect(await screen.findByText("状态库不可用")).toBeTruthy();
    const before = vi.mocked(tauri.getKnowledgeIndexStatus).mock.calls.length;
    fireEvent.click(screen.getByRole("button", { name: "重新加载" }));
    await waitFor(() =>
      expect(vi.mocked(tauri.getKnowledgeIndexStatus).mock.calls.length).toBeGreaterThan(before),
    );
  });

  it("完成但无片段时保留空状态并禁用输入", async () => {
    vi.mocked(tauri.getKnowledgeIndexStatus).mockResolvedValue(indexFixture({ chunkCount: 0 }));
    renderChat();

    await screen.findByText("当前范围没有可检索内容");
    expect(screen.queryByRole("textbox")).toBeNull();
    expect(screen.queryByRole("button", { name: "建立索引" })).toBeNull();
  });

  it("完成索引不创建 1 秒常驻轮询", async () => {
    const timeout = vi.spyOn(window, "setTimeout");
    renderChat();
    await screen.findByText("从 全部资料 中查找答案");

    const appPollTimers = timeout.mock.calls.filter(
      ([callback, delay]) => delay === 1_000 && callback.name !== "handleTimeout",
    );
    expect(appPollTimers).toHaveLength(0);
    timeout.mockRestore();
  });

  it("索引事件立即刷新，不等待兜底轮询", async () => {
    vi.mocked(tauri.getKnowledgeIndexStatus).mockResolvedValue(
      indexFixture({ status: "indexing", processedRecords: 0, chunkCount: 0 }),
    );
    renderChat();
    await screen.findAllByText("正在建立知识索引");
    const before = vi.mocked(tauri.getKnowledgeIndexStatus).mock.calls.length;

    emit("knowledge-index-update");

    await waitFor(() =>
      expect(vi.mocked(tauri.getKnowledgeIndexStatus).mock.calls.length).toBeGreaterThan(before),
    );
  });

  it("事件丢失时由 1 秒兜底恢复状态", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      vi.mocked(tauri.getKnowledgeIndexStatus).mockResolvedValue(
        indexFixture({ status: "indexing", processedRecords: 0, chunkCount: 0 }),
      );
      renderChat();
      await screen.findAllByText("正在建立知识索引");
      const before = vi.mocked(tauri.getKnowledgeIndexStatus).mock.calls.length;

      await act(async () => {
        await vi.advanceTimersByTimeAsync(1_100);
      });

      expect(vi.mocked(tauri.getKnowledgeIndexStatus).mock.calls.length).toBeGreaterThan(before);
    } finally {
      vi.useRealTimers();
    }
  });

  it("问答成功后显示回答，失败时保留草稿入口并可重试", async () => {
    renderChat();
    await screen.findByText("从 全部资料 中查找答案");
    vi.mocked(tauri.askKnowledgeBase).mockResolvedValueOnce({
      answer: "关键结论是先验证。",
      citations: [],
      insufficientEvidence: false,
    });

    const question = screen.getByRole("button", { name: "最近有哪些重要会议结论？" });
    fireEvent.click(question);
    await screen.findByText("关键结论是先验证。");

    fireEvent.change(screen.getByRole("textbox"), { target: { value: "还有哪些待办？" } });
    vi.mocked(tauri.askKnowledgeBase)
      .mockRejectedValueOnce(new Error("本地模型不可用"));
    fireEvent.click(screen.getByRole("button", { name: "发送" }));
    await screen.findByText("本次回答失败");
    expect(screen.getByText("本地模型不可用")).toBeTruthy();

    vi.mocked(tauri.askKnowledgeBase).mockResolvedValueOnce({
      answer: "还有一项待办。",
      citations: [],
      insufficientEvidence: false,
    });
    fireEvent.click(screen.getByRole("button", { name: "重试这个问题" }));
    await screen.findByText("还有一项待办。");
  });
});
