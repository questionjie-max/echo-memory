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

  // 块3/F5 回归：重新运行引导向导必须真的触发「关闭设置 → 重查状态 → 弹向导」链路。
  it("重新运行引导向导：调用 resetOnboarding 并关闭设置", async () => {
    vi.mocked(tauri.resetOnboarding).mockResolvedValue(undefined);
    const onClose = vi.fn();
    render(<SettingsPanel open onClose={onClose} />);
    await screen.findByText("AI 模型", { selector: "button" });
    fireEvent.click(screen.getByText("关于", { selector: "button" }));
    fireEvent.click(screen.getByRole("button", { name: "重新运行引导向导" }));
    await waitFor(() => expect(tauri.resetOnboarding).toHaveBeenCalled());
    await waitFor(() => expect(onClose).toHaveBeenCalled());
  });
});

describe("v1.4 包6：词汇库批量导入", () => {
  beforeEach(() => {
    vi.resetAllMocks();
    setupSettingsMocks(tauri);
  });

  async function openHotwordsTab() {
    render(<SettingsPanel open onClose={() => undefined} />);
    await screen.findByText("AI 模型", { selector: "button" });
    fireEvent.click(screen.getByText("词汇库", { selector: "button" }));
    await screen.findByPlaceholderText(/输入热词/);
  }

  it("多行粘贴一次导入：每行一个热词，逐条添加", async () => {
    await openHotwordsTab();
    fireEvent.change(screen.getByPlaceholderText(/输入热词/), {
      target: { value: "通义听悟\n语音识别\n\n会议纪要" },
    });
    fireEvent.click(screen.getByRole("button", { name: "添加" }));
    await waitFor(() =>
      expect(tauri.addHotword).toHaveBeenCalledWith("通义听悟"),
    );
    expect(tauri.addHotword).toHaveBeenCalledWith("语音识别");
    expect(tauri.addHotword).toHaveBeenCalledWith("会议纪要");
    // 空行被跳过，不重复添加同一个词
    expect(tauri.addHotword).toHaveBeenCalledTimes(3);
  });

  it("单行输入仍然可以一次添加一个", async () => {
    await openHotwordsTab();
    fireEvent.change(screen.getByPlaceholderText(/输入热词/), {
      target: { value: "回声记忆" },
    });
    fireEvent.click(screen.getByRole("button", { name: "添加" }));
    await waitFor(() =>
      expect(tauri.addHotword).toHaveBeenCalledWith("回声记忆"),
    );
    expect(tauri.addHotword).toHaveBeenCalledTimes(1);
  });

  it("重复粘贴同一个词只导入一次（输入内去重）", async () => {
    await openHotwordsTab();
    fireEvent.change(screen.getByPlaceholderText(/输入热词/), {
      target: { value: "回声记忆\n回声记忆" },
    });
    fireEvent.click(screen.getByRole("button", { name: "添加" }));
    await waitFor(() =>
      expect(tauri.addHotword).toHaveBeenCalledTimes(1),
    );
  });

  it("批量导入给出明确反馈：添加了几个", async () => {
    await openHotwordsTab();
    fireEvent.change(screen.getByPlaceholderText(/输入热词/), {
      target: { value: "热词甲\n热词乙" },
    });
    fireEvent.click(screen.getByRole("button", { name: "添加" }));
    await waitFor(() => expect(tauri.addHotword).toHaveBeenCalledTimes(2));
    expect(await screen.findByText(/已添加 2 个热词/)).toBeTruthy();
    // 输入框清空
    expect((screen.getByPlaceholderText(/输入热词/) as HTMLInputElement).value).toBe("");
  });
});
