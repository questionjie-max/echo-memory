import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import OnboardingWizard from "../../src/components/OnboardingWizard";
import * as tauri from "../../src/lib/tauri";
import type { OnboardingStatus, SuggestedWatchFolder } from "../../src/shared/types";
import { localAiStatusFixture } from "./settings-mocks";

vi.mock("@tauri-apps/api/core", () => ({ isTauri: () => false }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: () => Promise.resolve(() => undefined),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn(() => Promise.resolve(null)) }));
vi.mock("../../src/lib/tauri");

function statusFixture(overrides: Partial<OnboardingStatus> = {}): OnboardingStatus {
  return {
    completed: false,
    completedAt: null,
    whisperReady: false,
    whisperModelPath: null,
    ollamaRunning: true,
    analysisModelReady: true,
    analysisModel: "qwen2.5:7b",
    watchFolderCount: 0,
    usbDetection: false,
    ...overrides,
  };
}

function suggestionFixture(
  path: string,
  label = "下载文件夹",
): SuggestedWatchFolder {
  return { path, label };
}

function renderWizard() {
  return render(<OnboardingWizard onFinished={() => undefined} />);
}

beforeEach(() => {
  vi.resetAllMocks();
  vi.mocked(tauri.getOnboardingStatus).mockResolvedValue(statusFixture());
  vi.mocked(tauri.suggestWatchFolders).mockResolvedValue([]);
  vi.mocked(tauri.getLocalAiStatus).mockResolvedValue(localAiStatusFixture());
  vi.mocked(tauri.completeOnboarding).mockResolvedValue(undefined);
  vi.mocked(tauri.setInboxUsbDetection).mockResolvedValue(undefined);
  vi.mocked(tauri.addInboxWatchFolder).mockResolvedValue({
    id: "folder-1",
    path: "/Users/me/Downloads",
    label: "下载文件夹",
    enabled: true,
    createdAt: "2026-09-23T10:00:00.000Z",
  });
});

describe("首次启动引导", () => {
  it("环境检查加载期间显示进行中状态", async () => {
    vi.mocked(tauri.getOnboardingStatus).mockReturnValue(new Promise(() => undefined));
    vi.mocked(tauri.suggestWatchFolders).mockReturnValue(new Promise(() => undefined));
    vi.mocked(tauri.getLocalAiStatus).mockReturnValue(new Promise(() => undefined));
    renderWizard();
    fireEvent.click(screen.getByRole("button", { name: "下一步" }));

    expect(await screen.findByText("正在检查…")).toBeTruthy();
  });

  it("环境检查展示本地引擎和分析模型状态", async () => {
    vi.mocked(tauri.getLocalAiStatus).mockResolvedValue(
      localAiStatusFixture({ whisperAvailable: true, whisperModelPath: "/models/turbo.bin" }),
    );
    renderWizard();
    fireEvent.click(screen.getByRole("button", { name: "下一步" }));

    expect(await screen.findByText("本地转写引擎")).toBeTruthy();
    expect(screen.getByText("可用")).toBeTruthy();
    expect(screen.getByText("Ollama 服务")).toBeTruthy();
  });

  it("初始环境读取失败时显示错误", async () => {
    vi.mocked(tauri.getOnboardingStatus).mockRejectedValue(new Error("状态库不可用"));
    renderWizard();

    expect((await screen.findByRole("alert")).textContent).toContain("状态库不可用");
  });

  it("收件箱没有建议目录时保留可继续的空状态", async () => {
    renderWizard();
    fireEvent.click(screen.getByRole("button", { name: "下一步" }));
    fireEvent.click(screen.getByRole("button", { name: "下一步" }));
    fireEvent.click(screen.getByRole("button", { name: "下一步" }));
    fireEvent.click(screen.getByRole("button", { name: "下一步" }));

    expect(await screen.findByText("音频收件箱")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "下载文件夹" })).toBeNull();
    expect(screen.getByRole("checkbox")).toBeTruthy();
  });

  it("稍后再说会完成标记并退出引导", async () => {
    const onFinished = vi.fn();
    render(
      <OnboardingWizard onFinished={onFinished} />,
    );

    fireEvent.click(await screen.findByRole("button", { name: "稍后再说" }));

    await waitFor(() => expect(tauri.completeOnboarding).toHaveBeenCalledTimes(1));
    expect(onFinished).toHaveBeenCalledTimes(1);
    expect(tauri.setInboxUsbDetection).not.toHaveBeenCalled();
  });

  it("选择收件箱建议后完成设置并保存 USB 选项", async () => {
    vi.mocked(tauri.suggestWatchFolders).mockResolvedValue([
      suggestionFixture("/Users/me/Downloads"),
    ]);
    const onFinished = vi.fn();
    render(
      <OnboardingWizard onFinished={onFinished} />,
    );

    fireEvent.click(await screen.findByRole("button", { name: "下一步" }));
    fireEvent.click(screen.getByRole("button", { name: "下一步" }));
    fireEvent.click(screen.getByRole("button", { name: "下一步" }));
    fireEvent.click(screen.getByRole("button", { name: "下一步" }));
    const folder = await screen.findByRole("button", { name: /下载文件夹/ });
    fireEvent.click(folder);
    fireEvent.click(screen.getByRole("checkbox"));
    fireEvent.click(screen.getByRole("button", { name: "下一步" }));
    fireEvent.click(screen.getByRole("button", { name: "开始使用" }));

    await waitFor(() =>
      expect(tauri.addInboxWatchFolder).toHaveBeenCalledWith("/Users/me/Downloads"),
    );
    expect(tauri.setInboxUsbDetection).toHaveBeenCalledWith(true);
    expect(tauri.completeOnboarding).toHaveBeenCalledTimes(1);
    expect(onFinished).toHaveBeenCalledTimes(1);
  });
});
