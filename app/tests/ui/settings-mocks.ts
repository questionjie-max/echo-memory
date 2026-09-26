/**
 * 设置面板测试用的假后端。
 *
 * fixture 必须和 src-tauri 的真实返回结构一致：少一个字段，组件会在真实运行时崩掉
 * 而测试还绿着，那比没有测试更糟。
 *
 * 用法：测试文件里 `vi.mock("../../src/lib/tauri")` 自动 mock 整个模块，
 * 然后在 beforeEach 里调 setupSettingsMocks(tauri)。
 */
import { vi } from "vitest";
import * as Tauri from "../../src/lib/tauri";
import type {
  ExternalAiSettings,
  LocalAiStatus,
  ModelDownloadProgress,
  TranscriptionEngineStatus,
} from "../../src/shared/types";

export function localAiStatusFixture(overrides: Partial<LocalAiStatus> = {}): LocalAiStatus {
  return {
    whisperAvailable: false,
    whisperModelPath: null,
    whisperModelSource: null,
    whisperModels: [],
    recommendedWhisperModel: {
      id: "large-v3-turbo-q5_0",
      label: "large-v3-turbo（q5_0）",
      fileName: "ggml-large-v3-turbo-q5_0.bin",
      bytes: 574_041_195,
    },
    pendingWhisperDownload: null,
    ollamaAvailable: true,
    ollamaModels: [],
    settings: {
      transcriptionLanguage: "zh",
      whisperModelPath: "",
      analysisModel: "qwen2.5:7b",
      embeddingModel: "qwen3-embedding:0.6b",
    },
    ...overrides,
  };
}

export function externalAiSettingsFixture(overrides: Partial<ExternalAiSettings> = {}): ExternalAiSettings {
  return {
    processingMode: "local",
    enabled: false,
    baseUrl: "https://api.openai.com/v1",
    model: "gpt-4.1-mini",
    hasApiKey: false,
    privacyConsentAt: null,
    transcriptionProvider: "openai-compatible",
    transcriptionBaseUrl: "https://dashscope.aliyuncs.com/compatible-mode/v1",
    transcriptionModel: "qwen3-asr-flash",
    transcriptionHasApiKey: false,
    audioUploadConsentAt: null,
    ...overrides,
  };
}

export function engineStatusFixture(overrides: Partial<TranscriptionEngineStatus> = {}): TranscriptionEngineStatus {
  return { engine: "embedded", whisperxAvailable: false, whisperxPath: null, hfTokenSet: false, ...overrides };
}

export function downloadProgressFixture(overrides: Partial<ModelDownloadProgress> = {}): ModelDownloadProgress {
  return {
    model: "large-v3-turbo-q5_0",
    status: "downloading",
    completed: null,
    total: null,
    error: null,
    ...overrides,
  };
}

export function setupSettingsMocks(api: typeof Tauri) {
  vi.mocked(api.getLocalAiStatus).mockResolvedValue(localAiStatusFixture());
  vi.mocked(api.getExternalAiSettings).mockResolvedValue(externalAiSettingsFixture());
  vi.mocked(api.updateExternalAiSettings).mockImplementation((settings: ExternalAiSettings) =>
    Promise.resolve(settings),
  );
  vi.mocked(api.setExternalAiApiKey).mockResolvedValue(externalAiSettingsFixture({ hasApiKey: true }));
  vi.mocked(api.clearExternalAiApiKey).mockResolvedValue(externalAiSettingsFixture());
  vi.mocked(api.setExternalAsrApiKey).mockResolvedValue(
    externalAiSettingsFixture({ transcriptionHasApiKey: true }),
  );
  vi.mocked(api.clearExternalAsrApiKey).mockResolvedValue(externalAiSettingsFixture());
  vi.mocked(api.testExternalAiConnection).mockResolvedValue(undefined);
  vi.mocked(api.updateKnowledgeSettings).mockImplementation((settings) => Promise.resolve(settings));
  vi.mocked(api.listAnalysisTemplates).mockResolvedValue([]);
  vi.mocked(api.getAudioPreprocessorStatus).mockResolvedValue({
    enhancedAvailable: true,
    engine: "ffmpeg",
    version: "7.1.1",
    executablePath: null,
  });
  vi.mocked(api.getInboxStatus).mockResolvedValue({
    usbDetection: false,
    watchFolders: [],
    counts: { pending: 0, imported: 0, failed: 0 },
    recentFiles: [],
  });
  vi.mocked(api.listHotwords).mockResolvedValue([]);
  vi.mocked(api.getOutputStatus).mockResolvedValue({ folder: null, autoExportAnalysis: false, recentFiles: [] });
  vi.mocked(api.getTranscriptCorrectionEnabled).mockResolvedValue(false);
  vi.mocked(api.getAppInfo).mockResolvedValue({ version: "0.7.0", libraryPath: "/tmp/echo-memory" });
  vi.mocked(api.getTranscriptionEngineStatus).mockResolvedValue(engineStatusFixture());
  vi.mocked(api.setTranscriptionEngine).mockResolvedValue(undefined);
  vi.mocked(api.setHfToken).mockResolvedValue(undefined);
  vi.mocked(api.clearHfToken).mockResolvedValue(undefined);
  vi.mocked(api.downloadWhisperModel).mockResolvedValue(undefined);
  vi.mocked(api.cancelModelDownload).mockResolvedValue(undefined);
  vi.mocked(api.pullOllamaModel).mockResolvedValue(undefined);
}
