/**
 * 冒烟测试：设置面板在打开与切换 tab 时必须加载数据。
 * 回归背景：v0.3.0 曾误删初始加载语句，v0.4.2 才修复——“正在读取…”永久停留。
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import SettingsPanel from "../../src/components/SettingsPanel";
import * as tauri from "../../src/lib/tauri";
import { setupSettingsMocks } from "./settings-mocks";

vi.mock("@tauri-apps/api/core", () => ({ isTauri: () => true }));
vi.mock("@tauri-apps/api/event", () => ({ listen: () => Promise.resolve(() => undefined) }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: () => Promise.resolve(null) }));
vi.mock("../../src/lib/tauri");

describe("SettingsPanel 数据加载（0.4.2 回归防护）", () => {
  beforeEach(() => {
    vi.resetAllMocks();
    setupSettingsMocks(tauri);
  });

  it("打开面板即加载 AI 模型状态（而不是永远显示正在读取）", async () => {
    render(<SettingsPanel open onClose={() => undefined} />);
    expect(await screen.findByText("AI 模型", { selector: "button" })).toBeTruthy();
    expect(tauri.getLocalAiStatus).toHaveBeenCalled();
    expect(tauri.getExternalAiSettings).toHaveBeenCalled();
  });

  it("打开后切换到收件箱 tab 会触发收件箱加载", async () => {
    render(<SettingsPanel open onClose={() => undefined} />);
    await screen.findByText("AI 模型", { selector: "button" });
    fireEvent.click(screen.getByText("收件箱", { selector: "button" }));
    expect(tauri.getInboxStatus).toHaveBeenCalled();
  });

  it("六个分区都在左侧导航里，旧的两个 AI 分区已经合并掉", async () => {
    render(<SettingsPanel open onClose={() => undefined} />);
    await screen.findByText("分析模板", { selector: "button" });
    for (const label of ["AI 模型", "分析模板", "收件箱", "词汇库", "产出", "关于"]) {
      expect(screen.getByText(label, { selector: "button" })).toBeTruthy();
    }
    expect(screen.queryByText("本机 AI", { selector: "button" })).toBeNull();
    expect(screen.queryByText("外部 AI", { selector: "button" })).toBeNull();
  });

  it("模板栏目可连续添加到上限，删除后恢复且新增项自动可见", async () => {
    const originalScrollIntoView = Element.prototype.scrollIntoView;
    const scrollIntoView = vi.fn();
    Element.prototype.scrollIntoView = scrollIntoView;

    try {
      render(<SettingsPanel open onClose={() => undefined} />);
      fireEvent.click(await screen.findByText("分析模板", { selector: "button" }));
      fireEvent.click(await screen.findByText("新建模板", { selector: "button" }));
      fireEvent.change(await screen.findByLabelText("名称"), {
        target: { value: "连续添加测试" },
      });
      const focusLabel = screen.getByText("分析重点").closest("label");
      const focusInput = focusLabel?.querySelector("textarea");
      expect(focusInput).toBeTruthy();
      fireEvent.change(focusInput as HTMLTextAreaElement, {
        target: { value: "验证栏目上限与滚动。" },
      });

      for (let index = 0; index < 10; index += 1) {
        fireEvent.click(screen.getByRole("button", { name: "添加栏目" }));
      }

      expect(screen.getAllByLabelText("栏目标题")).toHaveLength(10);
      expect(screen.getByRole("button", { name: "最多 10 个栏目" }).disabled).toBe(true);
      await waitFor(() =>
        expect(scrollIntoView).toHaveBeenCalledWith({ block: "nearest" }),
      );

      fireEvent.click(screen.getAllByTitle("删除栏目")[0]);
      expect(screen.getAllByLabelText("栏目标题")).toHaveLength(9);
      expect(screen.getByRole("button", { name: "添加栏目" }).disabled).toBe(false);
    } finally {
      if (originalScrollIntoView) {
        Element.prototype.scrollIntoView = originalScrollIntoView;
      } else {
        Reflect.deleteProperty(Element.prototype, "scrollIntoView");
      }
    }
  });
});
