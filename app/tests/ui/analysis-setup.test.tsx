import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { useRef, useState } from "react";
import { AnalysisSetupProvider, useAnalysisSetup } from "../../src/components/AnalysisSetupSheet";
import * as tauri from "../../src/lib/tauri";
import { externalAiSettingsFixture, localAiStatusFixture } from "./settings-mocks";

vi.mock("@tauri-apps/api/core", () => ({ isTauri: () => false }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: () => Promise.resolve(() => undefined),
}));
vi.mock("../../src/lib/tauri");

/**
 * 测试挂具：一个「触发」按钮模拟功能入口（分析重跑 / 问答 / 记忆生成），
 * 点击即走 ensureReady；动作执行次数显示在页面上供断言。
 */
function Harness({ needEmbedding = false }: { needEmbedding?: boolean }) {
  const { ensureReady, openSheet } = useAnalysisSetup();
  const [ranCount, setRanCount] = useState(0);
  const actionRef = useRef(() => setRanCount((count) => count + 1));
  actionRef.current = () => setRanCount((count) => count + 1);
  return (
    <div>
      <button
        onClick={() => void ensureReady({ needEmbedding, action: () => actionRef.current() })}
      >
        触发
      </button>
      {/* 模拟设置页入口：无条件打开面板 */}
      <button onClick={() => openSheet({ needEmbedding, action: () => actionRef.current() })}>
        直接打开
      </button>
      <span data-testid="ran-count">{ranCount}</span>
    </div>
  );
}

function renderHarness(needEmbedding?: boolean) {
  return render(
    <AnalysisSetupProvider>
      <Harness needEmbedding={needEmbedding} />
    </AnalysisSetupProvider>,
  );
}

beforeEach(() => {
  vi.resetAllMocks();
  // 默认一切未就绪：无 Ollama、无模型、云端未配置。
  vi.mocked(tauri.getLocalAiStatus).mockResolvedValue(
    localAiStatusFixture({ ollamaAvailable: false, ollamaModels: [] }),
  );
  vi.mocked(tauri.getExternalAiSettings).mockResolvedValue(externalAiSettingsFixture());
  vi.mocked(tauri.getOllamaInstallStatus).mockResolvedValue({ installed: false, running: false });
  vi.mocked(tauri.setExternalAiApiKey).mockResolvedValue(externalAiSettingsFixture({ hasApiKey: true }));
  vi.mocked(tauri.updateExternalAiSettings).mockImplementation((settings) => Promise.resolve(settings));
  vi.mocked(tauri.testExternalAiConnection).mockResolvedValue(undefined);
});

describe("分析设置发起面板", () => {
  it("分析未就绪时触发功能弹出面板，展示本地 Ollama 状态与引导", async () => {
    renderHarness();
    fireEvent.click(await screen.findByRole("button", { name: "触发" }));

    expect(await screen.findByRole("dialog", { name: "分析设置" })).toBeTruthy();
    expect(screen.getByText("Ollama 已安装")).toBeTruthy();
    expect(screen.getByText("Ollama 服务运行中")).toBeTruthy();
    // Ollama 未安装：给引导而不是死胡同
    expect(screen.getByText(/还没有安装 Ollama/)).toBeTruthy();
    expect(screen.getByRole("button", { name: "复制 ollama.com 链接" })).toBeTruthy();
    expect(screen.queryByTestId("ran-count").textContent).toBe("0");
  });

  it("云端就绪时触发功能不弹面板，动作直接继续", async () => {
    vi.mocked(tauri.getExternalAiSettings).mockResolvedValue(
      externalAiSettingsFixture({ enabled: true, hasApiKey: true, privacyConsentAt: "2026-10-01T00:00:00Z" }),
    );
    renderHarness();
    fireEvent.click(await screen.findByRole("button", { name: "触发" }));

    await waitFor(() => expect(screen.getByTestId("ran-count").textContent).toBe("1"));
    expect(screen.queryByRole("dialog", { name: "分析设置" })).toBeNull();
  });

  it("面板关闭后同一会话不再自动弹出，动作直接继续", async () => {
    renderHarness();
    fireEvent.click(await screen.findByRole("button", { name: "触发" }));
    await screen.findByRole("dialog", { name: "分析设置" });
    fireEvent.click(screen.getByRole("button", { name: "稍后再说" }));
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "分析设置" })).toBeNull());
    expect(screen.getByTestId("ran-count").textContent).toBe("0");

    fireEvent.click(screen.getByRole("button", { name: "触发" }));
    await waitFor(() => expect(screen.getByTestId("ran-count").textContent).toBe("1"));
    expect(screen.queryByRole("dialog", { name: "分析设置" })).toBeNull();
  });

  it("本地模型已在位时显示已就绪，点继续即 resume 原动作", async () => {
    vi.mocked(tauri.getLocalAiStatus).mockResolvedValue(
      localAiStatusFixture({
        ollamaAvailable: true,
        ollamaModels: [
          { name: "qwen2.5:7b", size: 4_700_000_000 },
          { name: "qwen3-embedding:0.6b", size: 639_000_000 },
        ],
      }),
    );
    vi.mocked(tauri.getOllamaInstallStatus).mockResolvedValue({ installed: true, running: true });
    renderHarness();
    // 走设置页入口无条件打开面板（ensureReady 在已就绪时不会弹面板，直接执行动作）
    fireEvent.click(await screen.findByRole("button", { name: "直接打开" }));

    expect(await screen.findByRole("dialog", { name: "分析设置" })).toBeTruthy();
    const resume = await screen.findByRole("button", { name: "已就绪，继续" });
    fireEvent.click(resume);

    await waitFor(() => expect(screen.getByTestId("ran-count").textContent).toBe("1"));
    expect(screen.queryByRole("dialog", { name: "分析设置" })).toBeNull();
  });

  it("云端选项卡未填 Key 时保存被挡，连通检测失败显示明确原因", async () => {
    vi.mocked(tauri.testExternalAiConnection).mockRejectedValue(new Error("无法连接服务商"));
    renderHarness(true);
    fireEvent.click(await screen.findByRole("button", { name: "触发" }));
    fireEvent.click(await screen.findByRole("button", { name: "云端（填 Key 即用）" }));

    const save = await screen.findByRole("button", { name: "保存并检测连通" });
    expect(save.hasAttribute("disabled")).toBe(true);

    fireEvent.change(screen.getByPlaceholderText("仅用于保存，不会回显"), {
      target: { value: "sk-test-0001" },
    });
    fireEvent.click(screen.getByRole("checkbox"));
    expect(save.hasAttribute("disabled")).toBe(false);

    fireEvent.click(save);
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("连通检测失败");
    expect(alert.textContent).toContain("无法连接服务商");
    // 检测失败不继续原动作，用户留在面板里修正
    expect(screen.getByTestId("ran-count").textContent).toBe("0");
  });
});
