/**
 * 冒烟测试：设置面板在打开与切换 tab 时必须加载数据。
 * 回归背景：v0.3.0 曾误删初始加载语句，v0.4.2 才修复——"正在读取…"永久停留。
 */
import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import SettingsPanel from "../../src/components/SettingsPanel";

vi.mock("@tauri-apps/api/core", () => ({ isTauri: () => true }));
vi.mock("@tauri-apps/api/event", () => ({ listen: () => Promise.resolve(() => undefined) }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: () => Promise.resolve(null) }));

const mocks = vi.hoisted(() => ({
  getLocalAiStatus: vi.fn(),
  getExternalAiSettings: vi.fn(),
  listAnalysisTemplates: vi.fn(),
  getAudioPreprocessorStatus: vi.fn(),
  getInboxStatus: vi.fn(),
  listHotwords: vi.fn(),
  getOutputStatus: vi.fn(),
  getTranscriptCorrectionEnabled: vi.fn(),
  getAppInfo: vi.fn(),
  getTranscriptionEngineStatus: vi.fn().mockResolvedValue({ engine: 'embedded', whisperxAvailable: false, whisperxPath: null, hfTokenSet: false }),
}));

vi.mock("../../src/lib/tauri", () => ({
  ...mocks,
  // 其余成员给安全空实现，组件按需调用
}));

function resolved() {
  mocks.getLocalAiStatus.mockResolvedValue({
    whisperAvailable: false, whisperModelPath: null, whisperModelSource: null,
    whisperModels: [], ollamaAvailable: true, ollamaModels: [],
    settings: { transcriptionLanguage: "zh", whisperModelPath: "", analysisModel: "qwen2.5:7b", embeddingModel: "qwen3-embedding:0.6b" },
  });
  mocks.getExternalAiSettings.mockResolvedValue({
    enabled: false, baseUrl: "", model: "", hasApiKey: false, privacyConsentAt: null, transcriptionProvider: "",
  });
  mocks.listAnalysisTemplates.mockResolvedValue([]);
  mocks.getAudioPreprocessorStatus.mockResolvedValue({ available: false });
  mocks.getInboxStatus.mockResolvedValue({ usbDetection: false, watchFolders: [], counts: { pending: 0, imported: 0, failed: 0 }, recentFiles: [] });
  mocks.listHotwords.mockResolvedValue([]);
  mocks.getOutputStatus.mockResolvedValue({ folder: null, autoExportAnalysis: false, recentFiles: [] });
  mocks.getTranscriptCorrectionEnabled.mockResolvedValue(false);
  mocks.getAppInfo.mockResolvedValue({ version: "0.5.0", libraryPath: "/tmp/x" });
}

describe("SettingsPanel 数据加载（0.4.2 回归防护）", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    resolved();
  });

  it("打开面板即加载本机 AI 状态（而不是永远显示正在读取）", async () => {
    render(<SettingsPanel open onClose={() => undefined} />);
    expect(await screen.findByText("本机 AI", { selector: "button" })).toBeTruthy();
    expect(mocks.getLocalAiStatus).toHaveBeenCalled();
    expect(mocks.getExternalAiSettings).toHaveBeenCalled();
  });

  it("打开后切换到收件箱 tab 会触发收件箱加载", async () => {
    render(<SettingsPanel open onClose={() => undefined} />);
    await screen.findByText("本机 AI", { selector: "button" });
    fireEvent.click(screen.getByText("收件箱", { selector: "button" }));
    expect(mocks.getInboxStatus).toHaveBeenCalled();
  });
});
