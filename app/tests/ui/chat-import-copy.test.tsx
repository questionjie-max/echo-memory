/**
 * 块3/F2+F3 回归测试：AI 对话导入的文案与扫描默认行为。
 * F2：文案与实际行为对齐——chat 只建索引不逐条分析，文案不得声称「分析/可在认知演化中确认」。
 * F3：Agent 会话扫描结果默认不选，导入必须由用户显式勾选。
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import ChatImportDialog from "../../src/components/ChatImportDialog";
import * as tauri from "../../src/lib/tauri";
import type { AgentScanResult } from "../../src/shared/types";

vi.mock("@tauri-apps/api/core", () => ({ isTauri: () => false }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
vi.mock("../../src/lib/tauri");

function scanFixture(): AgentScanResult {
  return {
    sessions: [
      {
        agent: "claude-code",
        path: "/tmp/s1.jsonl",
        title: "会话一",
        startedAt: "2026-11-07T10:00:00Z",
        sizeBytes: 1024,
        oversized: false,
      },
      {
        agent: "claude-code",
        path: "/tmp/s2.jsonl",
        title: "会话二",
        startedAt: "2026-11-07T11:00:00Z",
        sizeBytes: 2048,
        oversized: false,
      },
    ],
    memoryFiles: [],
  };
}

function renderDialog() {
  return render(
    <ChatImportDialog open={true} projectId={null} onClose={() => undefined} onImported={() => undefined} />,
  );
}

beforeEach(() => {
  vi.resetAllMocks();
});

describe("AI 对话导入：文案与行为对齐", () => {
  it("步骤三讲「后台索引」，不声称分析与认知演化确认（F2）", () => {
    renderDialog();
    expect(screen.getByText("后台索引")).toBeTruthy();
    expect(
      screen.getByText("导入完成后在后台建立检索索引，对话立即可搜索、可被问答引用；对话是档案，不做逐条分析。"),
    ).toBeTruthy();
    expect(screen.queryByText(/认知演化中确认/)).toBeNull();
  });

  it("主按钮只讲「导入」，不声称「并分析」（F2）", async () => {
    const { open } = await import("@tauri-apps/plugin-dialog");
    vi.mocked(open).mockResolvedValue(["/tmp/a.md"] as never);
    renderDialog();
    fireEvent.click(screen.getByRole("button", { name: /选择导出文件/ }));
    await screen.findByText("已选择 1 个文件");
    expect(screen.getByRole("button", { name: "导入（1 个文件）" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: /导入并分析/ })).toBeNull();
  });
});

describe("Agent 会话扫描：默认不选（F3）", () => {
  it("扫描结果默认全不勾选，需用户显式选择", async () => {
    vi.mocked(tauri.scanLocalAgents).mockResolvedValue(scanFixture());
    renderDialog();
    fireEvent.click(screen.getByRole("button", { name: /扫描本机 Agent 对话/ }));
    fireEvent.click(screen.getByRole("checkbox", { name: /Claude Code/ }));
    fireEvent.click(screen.getByRole("button", { name: "开始扫描" }));
    await screen.findByText(/找到 2 个会话/);
    await waitFor(() => expect(tauri.scanLocalAgents).toHaveBeenCalled());
    const checkboxes = screen.getAllByRole("checkbox") as HTMLInputElement[];
    const sessionBoxes = checkboxes.filter((box) => box.checked === false);
    // 所有会话复选框默认未勾选
    const checkedCount = checkboxes.filter((box) => box.checked).length;
    expect(checkedCount).toBe(0);
    expect(sessionBoxes.length).toBeGreaterThanOrEqual(2);
    expect(screen.getByText(/默认不选，请逐个勾选/)).toBeTruthy();
  });
});
