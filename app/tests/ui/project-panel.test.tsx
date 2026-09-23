import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import ProjectPanel from "../../src/components/ProjectPanel";
import * as tauri from "../../src/lib/tauri";
import type { McpStatus, Project } from "../../src/shared/types";

vi.mock("../../src/lib/tauri");

function projectFixture(id: string, name: string, status: Project["status"] = "active"): Project {
  return {
    id,
    name,
    status,
    createdAt: "2026-09-23T10:00:00.000Z",
    updatedAt: "2026-09-23T10:00:00.000Z",
  };
}

function mcpFixture(): McpStatus {
  return {
    enabled: false,
    authorizedScope: "全部知识库（只读）",
    executableAvailable: true,
    executablePath: "/app/bin/echo-memory-mcp",
    recentCalls: [],
  };
}

function renderPanel(onSelect = vi.fn()) {
  const result = render(
    <ProjectPanel selectedScope="all" refreshKey={0} onSelect={onSelect} />,
  );
  return { ...result, onSelect };
}

beforeEach(() => {
  vi.resetAllMocks();
  vi.mocked(tauri.listProjects).mockResolvedValue([]);
  vi.mocked(tauri.getMcpStatus).mockResolvedValue(mcpFixture());
});

describe("知识库侧栏", () => {
  it("显示活跃和已归档知识库", async () => {
    vi.mocked(tauri.getMcpStatus).mockResolvedValue({
      ...mcpFixture(),
      enabled: true,
    });
    vi.mocked(tauri.listProjects).mockResolvedValue([
      projectFixture("p1", "产品研发"),
      projectFixture("p2", "历史资料", "archived"),
    ]);
    renderPanel();

    await screen.findByRole("button", { name: "产品研发" });
    fireEvent.click(screen.getByText("已归档知识库（1）"));
    expect(screen.getByText("历史资料")).toBeTruthy();
    expect(await screen.findByText("等待客户端连接")).toBeTruthy();
  });

  it("空列表显示明确空状态", async () => {
    vi.mocked(tauri.getMcpStatus).mockResolvedValue({
      ...mcpFixture(),
      executableAvailable: false,
    });
    renderPanel();

    expect(await screen.findByText("还没有知识库。")).toBeTruthy();
    expect(screen.getByText("MCP 组件未安装")).toBeTruthy();
  });

  it("列表读取失败时显示错误", async () => {
    vi.mocked(tauri.listProjects).mockRejectedValue(new Error("项目表不可用"));
    renderPanel();

    expect(await screen.findByText(/项目表不可用/)).toBeTruthy();
  });

  it("新建成功后刷新列表并选中新知识库", async () => {
    vi.mocked(tauri.createProject).mockResolvedValue(projectFixture("p1", "客户访谈"));
    vi.mocked(tauri.listProjects)
      .mockResolvedValueOnce([])
      .mockResolvedValue([projectFixture("p1", "客户访谈")]);
    const { onSelect } = renderPanel();

    await screen.findByText("还没有知识库。");
    fireEvent.change(screen.getByLabelText("新知识库名称"), {
      target: { value: "客户访谈" },
    });
    fireEvent.click(screen.getByRole("button", { name: "新建知识库" }));

    await waitFor(() => expect(tauri.createProject).toHaveBeenCalledWith("客户访谈"));
    await screen.findByRole("button", { name: "客户访谈" });
    expect(onSelect).toHaveBeenCalledWith("p1");
  });
});
