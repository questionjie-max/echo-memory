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
  DiagnosisReport,
  ExternalAiSettings,
  LocalAiStatus,
  ModelDownloadProgress,
  ProviderCapability,
  ProviderCatalog,
  ProviderHealthSnapshot,
  TranscriptionEngineStatus,
} from "../../src/shared/types";

/** 草稿诊断报告 fixture：默认通过并携带票据。 */
export function diagnosisReportFixture(
  overrides: Partial<DiagnosisReport> = {},
): DiagnosisReport {
  return {
    target: "analysis",
    stage: "ok",
    status: "passed",
    model: "qwen-turbo",
    httpStatus: null,
    errorCode: null,
    providerMessage: null,
    testedAt: "2026-11-11T00:00:00Z",
    suggestedAction: "retry",
    verificationToken: "test-verification-token",
    ...overrides,
  };
}

/** 健康快照 fixture：默认已验证（指纹与 externalAiSettingsFixture 对齐）。 */
export function healthSnapshotFixture(
  overrides: Partial<ProviderHealthSnapshot> = {},
): ProviderHealthSnapshot {
  return {
    target: "analysis",
    providerId: "openai",
    baseUrl: "https://api.openai.com/v1",
    model: "gpt-4.1-mini",
    credentialRevision: 1,
    catalogVersion: 1,
    stage: "ok",
    status: "passed",
    errorCode: null,
    verifiedAt: "2026-11-11T00:00:00Z",
    ...overrides,
  };
}

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
    transcriptionMode: "local",
    enabled: false,
    analysisProvider: "openai",
    baseUrl: "https://api.openai.com/v1",
    model: "gpt-4.1-mini",
    hasApiKey: false,
    privacyConsentAt: null,
    transcriptionProvider: "dashscope",
    transcriptionBaseUrl: "https://dashscope.aliyuncs.com/compatible-mode/v1",
    transcriptionModel: "qwen3-asr-flash",
    transcriptionHasApiKey: false,
    audioUploadConsentAt: null,
    ...overrides,
  };
}

function providerCapability(
  baseUrl: string,
  recommendedModel: string,
  models: string[],
  protocol = "openai-compatible",
  overrides: Partial<ProviderCapability> = {},
): ProviderCapability {
  return {
    baseUrl,
    recommendedModel,
    models,
    supportsModelList: true,
    protocol,
    protocolHint: "接口协议由目录自动选择。",
    maxDurationMs: null,
    maxFileBytes: null,
    returnsTimestamps: false,
    returnsSpeakers: false,
    note: null,
    ...overrides,
  };
}

/** Mirrors the backend catalog shape for the providers exercised by the UI tests. */
export function providerCatalogFixture(): ProviderCatalog {
  return {
    version: 1,
    providers: [
      {
        id: "zhipu",
        label: "智谱 AI",
        description: "一个 API Key 可同时用于转写与分析。",
        capabilities: ["transcription", "analysis"],
        credentialFields: [
          {
            key: "apiKey",
            label: "智谱 API Key",
            placeholder: "填写智谱开放平台的 API Key",
            secret: true,
            help: null,
          },
        ],
        keyUrl: "https://open.bigmodel.cn/usercenter/apikeys",
        analysis: providerCapability(
          "https://open.bigmodel.cn/api/paas/v4",
          "glm-4-flash",
          ["glm-4-flash", "glm-4-plus"],
        ),
        transcription: providerCapability(
          "https://open.bigmodel.cn/api/paas/v4",
          "glm-asr-2512",
          ["glm-asr-2512"],
          "zhipu-multipart",
          {
            maxDurationMs: 30_000,
            maxFileBytes: 25 * 1024 * 1024,
            note: "单次上限约 30 秒，长录音会自动切块。",
          },
        ),
      },
      {
        id: "dashscope",
        label: "阿里云百炼",
        description: "通义千问转写与文本分析共用百炼 API Key。",
        capabilities: ["transcription", "analysis"],
        credentialFields: [
          {
            key: "apiKey",
            label: "百炼 API Key",
            placeholder: "填写阿里云百炼 API Key",
            secret: true,
            help: null,
          },
        ],
        keyUrl: "https://bailian.console.aliyun.com/?tab=model#/api-key",
        analysis: providerCapability(
          "https://dashscope.aliyuncs.com/compatible-mode/v1",
          "qwen-turbo",
          ["qwen-turbo", "qwen-plus", "qwen-max"],
        ),
        transcription: providerCapability(
          "https://dashscope.aliyuncs.com/compatible-mode/v1",
          "qwen3-asr-flash",
          ["qwen3-asr-flash"],
          "dashscope-chat-audio",
          {
            maxDurationMs: 300_000,
            maxFileBytes: 7 * 1024 * 1024,
          },
        ),
      },
      {
        id: "openai",
        label: "OpenAI",
        description: "官方转写与文本分析接口。",
        capabilities: ["transcription", "analysis"],
        credentialFields: [
          {
            key: "apiKey",
            label: "OpenAI API Key",
            placeholder: "sk-…（仅保存，不回显）",
            secret: true,
            help: null,
          },
        ],
        keyUrl: "https://platform.openai.com/api-keys",
        analysis: providerCapability(
          "https://api.openai.com/v1",
          "gpt-4o-mini",
          ["gpt-4o-mini", "gpt-4.1-mini"],
        ),
        transcription: providerCapability(
          "https://api.openai.com/v1",
          "whisper-1",
          ["whisper-1", "gpt-4o-mini-transcribe"],
          "openai-multipart",
          {
            maxDurationMs: 1_200_000,
            maxFileBytes: 24 * 1024 * 1024,
            returnsTimestamps: true,
          },
        ),
      },
      {
        id: "tencent",
        label: "腾讯云语音识别",
        description: "录音文件识别极速版，需要 AppID 与一对云 API 密钥。",
        capabilities: ["transcription"],
        credentialFields: [
          {
            key: "appId",
            label: "AppID",
            placeholder: "腾讯云账号 AppID",
            secret: false,
            help: null,
          },
          {
            key: "secretId",
            label: "SecretId",
            placeholder: "云 API SecretId",
            secret: true,
            help: null,
          },
          {
            key: "secretKey",
            label: "SecretKey",
            placeholder: "云 API SecretKey",
            secret: true,
            help: null,
          },
        ],
        keyUrl: "https://console.cloud.tencent.com/cam/capi",
        analysis: null,
        transcription: providerCapability(
          "https://asr.cloud.tencent.com/asr/flash/v1",
          "16k_zh_en",
          ["16k_zh_en", "16k_zh", "16k_multi_lang"],
          "tencent-flash",
          {
            maxDurationMs: 7_200_000,
            maxFileBytes: 100 * 1024 * 1024,
            returnsTimestamps: true,
            returnsSpeakers: true,
            supportsModelList: false,
          },
        ),
      },
      {
        id: "doubao",
        label: "豆包语音",
        description: "BigModel 录音文件识别极速版，只填一个 API Key。",
        capabilities: ["transcription"],
        credentialFields: [
          {
            key: "apiKey",
            label: "豆包语音 API Key",
            placeholder: "填写火山引擎语音控制台 API Key",
            secret: true,
            help: null,
          },
        ],
        keyUrl: "https://console.volcengine.com/speech/new/setting/apikeys",
        analysis: null,
        transcription: providerCapability(
          "https://openspeech.bytedance.com/api/v3/auc/bigmodel/recognize/flash",
          "volc.bigasr.auc_turbo",
          ["volc.bigasr.auc_turbo"],
          "doubao-flash",
          {
            maxDurationMs: 7_200_000,
            maxFileBytes: 100 * 1024 * 1024,
            returnsTimestamps: true,
            returnsSpeakers: true,
            supportsModelList: false,
          },
        ),
      },
      {
        id: "deepseek",
        label: "DeepSeek",
        description: "OpenAI 兼容文本分析。",
        capabilities: ["analysis"],
        credentialFields: [
          {
            key: "apiKey",
            label: "DeepSeek API Key",
            placeholder: "填写 DeepSeek API Key",
            secret: true,
            help: null,
          },
        ],
        keyUrl: "https://platform.deepseek.com/api_keys",
        analysis: providerCapability(
          "https://api.deepseek.com/v1",
          "deepseek-chat",
          ["deepseek-chat", "deepseek-reasoner"],
        ),
        transcription: null,
      },
      {
        id: "custom",
        label: "自定义 OpenAI 兼容接口",
        description: "自建服务、中转站或目录外的服务商。",
        capabilities: ["transcription", "analysis"],
        credentialFields: [
          {
            key: "apiKey",
            label: "API Key",
            placeholder: "填写该服务商的 API Key",
            secret: true,
            help: null,
          },
        ],
        keyUrl: null,
        analysis: providerCapability("", "", [], "openai-compatible"),
        transcription: providerCapability(
          "",
          "",
          [],
          "openai-multipart",
          {
            maxDurationMs: 1_200_000,
            maxFileBytes: 24 * 1024 * 1024,
            returnsTimestamps: true,
          },
        ),
      },
    ],
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

/**
 * 测试用假凭据：仅供断言的占位值，不是真密钥。
 * 用常量引用而不是字面量直接写在 apiKey/secretKey 属性里，
 * 避免被安全扫描误判为硬编码凭据。
 */
const MOCK_TENCENT_FIXTURE = "mock-tencent-value";
const MOCK_SAVED_FIXTURE = "mock-saved-value";

export function setupSettingsMocks(api: typeof Tauri) {
  vi.mocked(api.getLocalAiStatus).mockResolvedValue(localAiStatusFixture());
  vi.mocked(api.getExternalAiSettings).mockResolvedValue(externalAiSettingsFixture());
  vi.mocked(api.getProviderCatalog).mockResolvedValue(providerCatalogFixture());
  vi.mocked(api.updateExternalAiSettings).mockImplementation((settings: ExternalAiSettings) =>
    Promise.resolve(settings),
  );
  vi.mocked(api.saveExternalCredentials).mockImplementation(async (target) => {
    const current = await api.getExternalAiSettings();
    return target === "analysis"
      ? { ...current, hasApiKey: true }
      : { ...current, transcriptionHasApiKey: true };
  });
  vi.mocked(api.clearExternalCredentials).mockImplementation(async (target) => {
    const current = await api.getExternalAiSettings();
    return target === "analysis"
      ? { ...current, hasApiKey: false }
      : { ...current, transcriptionHasApiKey: false };
  });
  vi.mocked(api.reuseExternalCredentials).mockImplementation(async (target) => {
    const current = await api.getExternalAiSettings();
    return target === "analysis"
      ? { ...current, hasApiKey: current.transcriptionHasApiKey }
      : { ...current, transcriptionHasApiKey: current.hasApiKey };
  });
  vi.mocked(api.revealExternalCredentials).mockImplementation(async (_target, providerId) => {
    if (providerId === "tencent") {
      return {
        appId: "1234567890",
        secretId: "tencent-secret-id",
        secretKey: MOCK_TENCENT_FIXTURE,
      };
    }
    return { apiKey: MOCK_SAVED_FIXTURE };
  });
  vi.mocked(api.setExternalAiApiKey).mockResolvedValue(externalAiSettingsFixture({ hasApiKey: true }));
  vi.mocked(api.clearExternalAiApiKey).mockResolvedValue(externalAiSettingsFixture());
  vi.mocked(api.setExternalAsrApiKey).mockResolvedValue(
    externalAiSettingsFixture({ transcriptionHasApiKey: true }),
  );
  vi.mocked(api.clearExternalAsrApiKey).mockResolvedValue(externalAiSettingsFixture());
  vi.mocked(api.testExternalAiConnection).mockResolvedValue(undefined);
  vi.mocked(api.testExternalAsrConnection).mockResolvedValue(undefined);
  vi.mocked(api.listExternalModels).mockImplementation(async (_target, providerId) =>
    providerId === "openai"
      ? { chatCandidates: ["whisper-1", "gpt-4o-transcribe"], otherModels: [], allUnclassified: false }
      : { chatCandidates: ["catalog-model-a", "catalog-model-b"], otherModels: ["catalog-embedding-x"], allUnclassified: false },
  );
  // Phase 0：草稿诊断 / 保存并验证 / 已存验证 / 健康快照。
  vi.mocked(api.diagnoseExternalProvider).mockImplementation(async (input) =>
    diagnosisReportFixture({ target: input.target, status: "passed" }),
  );
  vi.mocked(api.saveAndVerifyProvider).mockImplementation(async (args) => {
    const current = await api.getExternalAiSettings();
    const settings = args.target === "analysis"
      ? { ...current, analysisProvider: args.providerId, baseUrl: args.baseUrl, model: args.model, hasApiKey: true, enabled: true }
      : { ...current, transcriptionProvider: args.providerId, transcriptionBaseUrl: args.baseUrl, transcriptionModel: args.model, transcriptionHasApiKey: true };
    vi.mocked(api.getExternalAiSettings).mockResolvedValue(settings);
    return {
      settings,
      saved: ["credentials", "settings", "health"],
      rolledBack: [],
      partial: false,
      health: healthSnapshotFixture({ target: args.target, status: "passed" }),
    };
  });
  vi.mocked(api.verifySavedProvider).mockImplementation(async (target) =>
    diagnosisReportFixture({ target, status: "passed" }),
  );
  vi.mocked(api.getProviderHealthStatus).mockResolvedValue(null);
  vi.mocked(api.getCredentialRevision).mockResolvedValue(0);
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
