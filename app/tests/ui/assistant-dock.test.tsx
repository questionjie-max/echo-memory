import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import AssistantDock from "../../src/components/AssistantDock";
import * as tauri from "../../src/lib/tauri";
import type { DockMessage, DockReply } from "../../src/shared/types";

vi.mock("../../src/lib/tauri");

const STORAGE_KEY = "echo-memory-dock-state";

function message(id: string, role: DockMessage["role"], content: string): DockMessage {
  return {
    id,
    chatId: "chat-1",
    role,
    content,
    mode: "free",
    createdAt: "2026-09-23T10:00:00.000Z",
  };
}

function dockStatus(messages: DockMessage[] = []) {
  return {
    chat: {
      id: "chat-1",
      title: "测试对话",
      engine: "local",
      createdAt: "2026-09-23T10:00:00.000Z",
      updatedAt: "2026-09-23T10:00:00.000Z",
    },
    messages,
    externalAvailable: false,
  };
}

function renderDock() {
  return render(
    <AssistantDock selectedRecordId={null} selectedRecordTitle={null} />,
  );
}

beforeEach(() => {
  vi.resetAllMocks();
  localStorage.clear();
  vi.mocked(tauri.getDockStatus).mockResolvedValue(dockStatus());
});

describe("随行助手", () => {
  it("默认收起，持久化状态展开时显示历史", async () => {
    localStorage.setItem(STORAGE_KEY, JSON.stringify({ collapsed: false, engine: "local" }));
    vi.mocked(tauri.getDockStatus).mockResolvedValue(
      dockStatus([message("m1", "assistant", "上一次的回复")]),
    );
    renderDock();

    expect(await screen.findByText("上一次的回复")).toBeTruthy();
    expect(screen.getByRole("button", { name: /随行助手/ }).getAttribute("aria-expanded")).toBe("true");
  });

  it("默认收起时显示展开后的空状态", async () => {
    renderDock();

    const toggle = await screen.findByRole("button", { name: /随行助手/ });
    expect(toggle.getAttribute("aria-expanded")).toBe("false");
    fireEvent.click(toggle);
    expect(await screen.findByText(/跨全部录音、带原文引用的提问/)).toBeTruthy();
  });

  it("状态读取失败时显示可诊断错误", async () => {
    localStorage.setItem(STORAGE_KEY, JSON.stringify({ collapsed: false, engine: "local" }));
    vi.mocked(tauri.getDockStatus).mockRejectedValue(new Error("助手状态不可用"));
    renderDock();

    expect((await screen.findByRole("alert")).textContent).toContain("助手状态不可用");
  });

  it("发送失败时保留用户草稿并允许重试", async () => {
    localStorage.setItem(STORAGE_KEY, JSON.stringify({ collapsed: false, engine: "local" }));
    vi.mocked(tauri.askDock).mockRejectedValue(new Error("本地模型未运行"));
    renderDock();
    await screen.findByRole("textbox");

    fireEvent.change(screen.getByRole("textbox"), { target: { value: "帮我总结" } });
    fireEvent.click(screen.getByRole("button", { name: "发送" }));

    await screen.findByRole("alert");
    expect((screen.getByRole("textbox") as HTMLTextAreaElement).value).toBe("帮我总结");
  });

  it("发送成功后同时写入用户和助手消息", async () => {
    localStorage.setItem(STORAGE_KEY, JSON.stringify({ collapsed: false, engine: "local" }));
    const reply: DockReply = {
      chat: dockStatus().chat!,
      userMessage: message("u1", "user", "帮我总结"),
      assistantMessage: message("a1", "assistant", "已完成总结"),
    };
    vi.mocked(tauri.askDock).mockResolvedValue(reply);
    renderDock();
    await screen.findByRole("textbox");

    fireEvent.change(screen.getByRole("textbox"), { target: { value: "帮我总结" } });
    fireEvent.click(screen.getByRole("button", { name: "发送" }));

    await screen.findByText("已完成总结");
    expect(screen.getByText("帮我总结")).toBeTruthy();
    expect((screen.getByRole("textbox") as HTMLTextAreaElement).value).toBe("");
    await waitFor(() => expect(tauri.askDock).toHaveBeenCalledTimes(1));
  });
});
