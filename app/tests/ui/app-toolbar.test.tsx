/**
 * 顶栏版式契约测试。
 *
 * 回归背景：搜索框以前只在首页渲染，而它是顶栏里唯一带 flex-grow 的元素，
 * 一旦按视图增减，space-between 会重新分配剩余宽度，导航就横着挪 ——
 * 用户报告的「导航来回跳」就是这个。这里锁住「搜索框常驻、导航固定」的契约。
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import AppToolbar from "../../src/components/AppToolbar";
import * as tauri from "../../src/lib/tauri";

vi.mock("@tauri-apps/api/core", () => ({ isTauri: () => true }));
vi.mock("@tauri-apps/api/event", () => ({ listen: () => Promise.resolve(() => undefined) }));
vi.mock("../../src/lib/tauri");

const VIEWS = ["library", "chat", "growth", "evolution", "actions"] as const;

beforeEach(() => {
  vi.resetAllMocks();
  vi.mocked(tauri.searchRecords).mockResolvedValue([]);
});

describe("顶栏版式契约", () => {
  it("五个导航项用新名字，且顺序固定", () => {
    render(<AppToolbar mainView="library" onSelectView={() => undefined} projectId={null} unfiledOnly={false} onOpenSearchResult={() => undefined} onOpenSettings={() => undefined} />);
    const nav = screen.getByRole("navigation", { name: "主视图" });
    expect(Array.from(nav.querySelectorAll("button")).map((el) => el.textContent)).toEqual([
      "首页",
      "问知识库",
      "成长轨迹",
      "认知演化",
      "行动",
    ]);
  });

  it.each(VIEWS)("搜索框在 %s 视图也渲染 —— 不再按视图增减", (view) => {
    const { unmount } = render(
      <AppToolbar mainView={view} onSelectView={() => undefined} projectId={null} unfiledOnly={false} onOpenSearchResult={() => undefined} onOpenSettings={() => undefined} />,
    );
    expect(screen.getByRole("navigation", { name: "主视图" })).toBeTruthy();
    expect(document.querySelector(".search-panel")).toBeTruthy();
    // 设置按钮按 Lovart 稿改成齿轮图标，但可访问名仍是「设置」。
    expect(screen.getByRole("button", { name: "设置" })).toBeTruthy();
    unmount();
  });

  it("当前视图的导航项带 aria-current，其余不带", () => {
    render(<AppToolbar mainView="growth" onSelectView={() => undefined} projectId={null} unfiledOnly={false} onOpenSearchResult={() => undefined} onOpenSettings={() => undefined} />);
    const buttons = Array.from(screen.getByRole("navigation", { name: "主视图" }).querySelectorAll("button"));
    expect(buttons.filter((el) => el.getAttribute("aria-current") === "page").map((el) => el.textContent)).toEqual(["成长轨迹"]);
    expect(buttons.every((el) => el.textContent !== "AI 伙伴")).toBe(true);
  });

  it("品牌区副标题使用收紧后的隐私口径", () => {
    render(<AppToolbar mainView="library" onSelectView={() => undefined} projectId={null} unfiledOnly={false} onOpenSearchResult={() => undefined} onOpenSettings={() => undefined} />);
    // 旧口径过度承诺「全部本地」；新口径区分硬承诺与选择权。
    expect(screen.getByText("让每次对话，都沉淀为长期记忆")).toBeTruthy();
    expect(screen.queryByText(/默认本地处理/)).toBeNull();
  });
});
