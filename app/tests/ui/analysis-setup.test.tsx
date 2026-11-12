import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { useRef, useState } from "react";
import { AnalysisSetupProvider, useAnalysisSetup } from "../../src/components/AnalysisSetupSheet";
import { diagnosisReportFixture, healthSnapshotFixture } from "./settings-mocks";
import * as tauri from "../../src/lib/tauri";
import type { ExternalAiSettings } from "../../src/shared/types";
import {
  externalAiSettingsFixture,
  localAiStatusFixture,
  providerCatalogFixture,
} from "./settings-mocks";

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

let settingsState: ExternalAiSettings;

beforeEach(() => {
  vi.resetAllMocks();
  settingsState = externalAiSettingsFixture();
  // 默认一切未就绪：无 Ollama、无模型、云端未配置。
  vi.mocked(tauri.getLocalAiStatus).mockResolvedValue(
    localAiStatusFixture({ ollamaAvailable: false, ollamaModels: [] }),
  );
  vi.mocked(tauri.getExternalAiSettings).mockImplementation(() => Promise.resolve(settingsState));
  vi.mocked(tauri.getProviderCatalog).mockResolvedValue(providerCatalogFixture());
  vi.mocked(tauri.getOllamaInstallStatus).mockResolvedValue({ installed: false, running: false });
  vi.mocked(tauri.updateExternalAiSettings).mockImplementation((settings) => {
    settingsState = settings;
    return Promise.resolve(settingsState);
  });
  vi.mocked(tauri.saveExternalCredentials).mockImplementation(() => {
    settingsState = { ...settingsState, hasApiKey: true };
    return Promise.resolve(settingsState);
  });
  vi.mocked(tauri.clearExternalCredentials).mockImplementation(() => {
    settingsState = { ...settingsState, hasApiKey: false };
    return Promise.resolve(settingsState);
  });
  vi.mocked(tauri.reuseExternalCredentials).mockImplementation(() => {
    settingsState = { ...settingsState, hasApiKey: settingsState.transcriptionHasApiKey };
    return Promise.resolve(settingsState);
  });
  vi.mocked(tauri.testExternalAiConnection).mockResolvedValue(undefined);
  // Phase 0：草稿诊断默认通过并携带一次性票据；保存并验证落盘为新配置。
  vi.mocked(tauri.diagnoseExternalProvider).mockImplementation((input) =>
    Promise.resolve(diagnosisReportFixture({ target: input.target })),
  );
  vi.mocked(tauri.saveAndVerifyProvider).mockImplementation((args) => {
    settingsState = {
      ...settingsState,
      analysisProvider: args.providerId,
      baseUrl: args.baseUrl,
      model: args.model,
      hasApiKey: true,
      enabled: args.enabled,
      privacyConsentAt: args.privacyConsentAt ?? settingsState.privacyConsentAt,
    };
    return Promise.resolve({
      settings: settingsState,
      saved: ["credentials", "settings", "health"],
      rolledBack: [],
      partial: false,
      health: healthSnapshotFixture({ target: args.target }),
    });
  });
  vi.mocked(tauri.verifySavedProvider).mockImplementation((target) =>
    Promise.resolve(diagnosisReportFixture({ target })),
  );
  vi.mocked(tauri.getProviderHealthStatus).mockResolvedValue(null);
  vi.mocked(tauri.getCredentialRevision).mockResolvedValue(0);
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

  it("首次打开云端复用同一服务商表单，不再维护第二份预置", async () => {
    renderHarness(true);
    fireEvent.click(await screen.findByRole("button", { name: "触发" }));
    fireEvent.click(await screen.findByRole("button", { name: "云端（填 Key 即用）" }));

    const provider = (await screen.findByLabelText("分析服务商")) as HTMLSelectElement;
    expect(provider.value).toBe("openai");
    expect(screen.queryByLabelText("接口地址")).toBeNull();
    fireEvent.change(provider, { target: { value: "zhipu" } });
    expect(screen.getByLabelText("智谱 API Key")).toBeTruthy();
    expect((screen.getByLabelText("分析模型") as HTMLSelectElement).value).toBe("glm-4-flash");
    expect(screen.queryByLabelText("接口地址")).toBeNull();
  });

  it("保存后启用分析并执行一次真实小请求，成功后恢复原动作", async () => {
    renderHarness(true);
    fireEvent.click(await screen.findByRole("button", { name: "触发" }));
    fireEvent.click(await screen.findByRole("button", { name: "云端（填 Key 即用）" }));

    fireEvent.change(await screen.findByLabelText("OpenAI API Key"), {
      target: { value: "sk-analysis" },
    });
    fireEvent.click(screen.getByRole("checkbox"));
    fireEvent.click(screen.getByRole("button", { name: "保存并验证" }));

    // 先草稿诊断（不落盘），通过后凭票据提交，成功才续跑原动作。
    await waitFor(() => expect(tauri.diagnoseExternalProvider).toHaveBeenCalled());
    await waitFor(() => expect(tauri.saveAndVerifyProvider).toHaveBeenCalled());
    await waitFor(() => expect(screen.getByTestId("ran-count").textContent).toBe("1"));
    expect(screen.queryByRole("dialog", { name: "分析设置" })).toBeNull();
    expect(tauri.saveAndVerifyProvider).toHaveBeenCalledWith(
      expect.objectContaining({
        target: "analysis",
        verificationToken: "test-verification-token",
        enabled: true,
      }),
    );
    // 旧的先保存再测路径不该再被走到。
    expect(tauri.saveExternalCredentials).not.toHaveBeenCalled();
    expect(tauri.testExternalAiConnection).not.toHaveBeenCalled();
  });

  it("未填 Key 或未勾同意时保存被挡", async () => {
    renderHarness(true);
    fireEvent.click(await screen.findByRole("button", { name: "触发" }));
    fireEvent.click(await screen.findByRole("button", { name: "云端（填 Key 即用）" }));

    const save = await screen.findByRole("button", { name: "保存并验证" });
    expect(save.hasAttribute("disabled")).toBe(true);

    fireEvent.change(screen.getByLabelText("OpenAI API Key"), {
      target: { value: "sk-test-0001" },
    });
    expect(save.hasAttribute("disabled")).toBe(true);
    fireEvent.click(screen.getByRole("checkbox"));
    expect(save.hasAttribute("disabled")).toBe(false);
  });

  it("草稿诊断失败时不落盘、明确报错且不继续原动作", async () => {
    vi.mocked(tauri.diagnoseExternalProvider).mockResolvedValue(
      diagnosisReportFixture({
        status: "failed",
        stage: "network",
        suggestedAction: "check_network",
        verificationToken: null,
      }),
    );
    renderHarness(true);
    fireEvent.click(await screen.findByRole("button", { name: "触发" }));
    fireEvent.click(await screen.findByRole("button", { name: "云端（填 Key 即用）" }));

    fireEvent.change(await screen.findByLabelText("OpenAI API Key"), {
      target: { value: "sk-test-0001" },
    });
    fireEvent.click(screen.getByRole("checkbox"));
    expect(settingsState.enabled).toBe(false);
    fireEvent.click(screen.getByRole("button", { name: "保存并验证" }));

    // 四段式错误：发生了什么 + 建议动作；配置不落盘，面板保留，原动作不继续。
    const alert = await screen.findByRole("alert", undefined, { timeout: 3000 });
    expect(alert.textContent).toContain("无法连接到服务商");
    expect(alert.textContent).toContain("检查网络");
    // 一个错误只展示一次：同一错误不得在页面内重复出现。
    expect(screen.getAllByRole("alert")).toHaveLength(1);
    expect(settingsState.enabled).toBe(false);
    expect(tauri.saveAndVerifyProvider).not.toHaveBeenCalled();
    expect(screen.getByRole("dialog", { name: "分析设置" })).toBeTruthy();
    expect(screen.getByTestId("ran-count").textContent).toBe("0");
  });
});

describe("v1.4 包3：本机配置黑话清零", () => {
  it("嵌入模型行改称记忆检索口径，不再裸用「嵌入模型」", async () => {
    renderHarness(true);
    fireEvent.click(await screen.findByRole("button", { name: "触发" }));
    const dialog = await screen.findByRole("dialog", { name: "分析设置" });
    const body = dialog.textContent ?? "";
    expect(body).toContain("记忆检索");
    expect(body).not.toContain("嵌入模型");
  });
});
