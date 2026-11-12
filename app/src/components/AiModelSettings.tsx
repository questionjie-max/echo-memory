/**
 * 「AI 模型」tab：把原来的「本机 AI」和「外部 AI」并成一页。
 *
 * 它们其实回答同一个问题——「谁来转写、谁来分析」。所以按三件事分组：
 * 转写（引擎 + 模型）、分析（本机 Ollama 或外部 API）、知识索引（嵌入模型）。
 * 本地只是其中一个选项，不是另一套信息架构。
 *
 * 所有改动即时生效，成功飘一句「已保存」，失败留在该卡片自己的提示里。
 */
import { useEffect, useState } from "react";
import type {
  AudioPreprocessorStatus,
  DiagnosisReport,
  ExternalAiSettings,
  KnowledgeSettings,
  LocalAiStatus,
  ProviderCatalog,
  ProviderHealthSnapshot,
  RemoteModelList,
  SaveAndVerifyOutcome,
  TranscriptionEngineStatus,
} from "../shared/types";
import {
  ANALYSIS_MODEL_SUGGESTIONS,
  OllamaModelList,
  WhisperModelList,
} from "./ModelList";
import {
  Button,
  Card,
  Collapsible,
  Field,
  Segmented,
  Stack,
  StatusPill,
  VerifyStatusPill,
} from "./SettingsKit";
import ProviderConnectionForm, {
  type ProviderCommit,
  type ProviderDiagnoseDraft,
  type ProviderSaveAndVerifyDraft,
  type ProviderTarget,
} from "./ProviderConnectionForm";
import { deriveVerifyState, failedDescription, verifiedDescription } from "../lib/verify-state";
import { formatBytes, type DownloadState, type DownloadKind } from "./useModelDownloads";

const LANGUAGES = [
  ["zh", "中文"],
  ["auto", "自动检测"],
  ["en", "英语"],
  ["ja", "日语"],
  ["ko", "韩语"],
  ["fr", "法语"],
  ["de", "德语"],
  ["es", "西班牙语"],
];

export interface AiModelSettingsProps {
  status: LocalAiStatus;
  preprocessor: AudioPreprocessorStatus | null;
  engine: TranscriptionEngineStatus | null;
  external: ExternalAiSettings | null;
  catalog: ProviderCatalog | null;
  downloads: Record<string, DownloadState>;
  busy: boolean;
  onSaveSettings: (patch: Partial<KnowledgeSettings>) => void;
  onStartDownload: (kind: DownloadKind, model: string) => void;
  onCancelDownload: (model: string) => void;
  onSwitchEngine: (next: "embedded" | "whisperx") => void;
  onSaveHfToken: (token: string) => void;
  onClearHfToken: () => void;
  onChooseWhisperFile: () => void;
  onSaveExternal: (patch: Partial<ExternalAiSettings>) => Promise<void>;
  onCommitProvider: (target: ProviderTarget, draft: ProviderCommit) => Promise<void>;
  onClearProviderCredentials: (target: ProviderTarget) => Promise<void>;
  onReuseProviderCredentials: (
    target: ProviderTarget,
    draft: Omit<ProviderCommit, "credentials" | "consentAt">,
  ) => Promise<void>;
  onRevealProviderCredentials: (
    target: ProviderTarget,
    providerId: string,
  ) => Promise<Record<string, string>>;
  onListProviderModels: (
    target: ProviderTarget,
    providerId: string,
    baseUrl: string,
    apiKey: string | null,
  ) => Promise<RemoteModelList>;
  /** 草稿诊断（不落盘）；通过时返回一次性验证票据。 */
  onDiagnoseProvider: (
    target: ProviderTarget,
    draft: ProviderDiagnoseDraft,
  ) => Promise<DiagnosisReport>;
  /** 保存并验证：先核销票据再提交；验证失败原配置不变。 */
  onSaveAndVerifyProvider: (
    target: ProviderTarget,
    draft: ProviderSaveAndVerifyDraft,
  ) => Promise<SaveAndVerifyOutcome>;
  /** 验证当前已保存配置（结果写入健康快照）。 */
  onVerifySavedProvider: (target: ProviderTarget) => Promise<DiagnosisReport>;
  /** 两个槽位的健康快照（null = 尚未验证过）。 */
  providerHealth: {
    analysis: ProviderHealthSnapshot | null;
    transcription: ProviderHealthSnapshot | null;
  };
  /** 两个槽位的凭据修订号（与快照比对判定「需重新验证」）。 */
  credentialRevisions: { analysis: number; transcription: number };
  catalogVersion: number;
  onRefresh: () => void;
  onOpenKnowledge: () => void;
  /** 打开「分析设置」发起面板；不传则不显示入口按钮。 */
  onOpenAnalysisSetup?: () => void;
  error: string | null;
}

export default function AiModelSettings(props: AiModelSettingsProps) {
  const { status, engine, external, downloads, busy } = props;
  const recommendedDownload = downloads[status.recommendedWhisperModel.id];
  const analysisDownload = downloads[status.settings.analysisModel];
  const embeddingDownload = downloads[status.settings.embeddingModel];
  const transcriptionMode = external?.transcriptionMode ?? "local";
  const [analysisMode, setAnalysisMode] = useState<"local" | "external">(
    external?.enabled ? "external" : "local",
  );
  const asrReady = Boolean(
    transcriptionMode === "external" &&
      external &&
      !["", "none"].includes(external.transcriptionProvider) &&
      external.transcriptionBaseUrl.trim() &&
      external.transcriptionModel.trim() &&
      external.transcriptionHasApiKey &&
      external.audioUploadConsentAt,
  );
  const externalAnalysisReady = analysisReady(external);
  const localAnalysisReady = Boolean(
    status.ollamaAvailable &&
      status.ollamaModels.some((model) => model.name === status.settings.analysisModel),
  );
  const currentAnalysisReady =
    analysisMode === "external" ? externalAnalysisReady : localAnalysisReady;
  const [whisperxGuide, setWhisperxGuide] = useState(false);
  const whisperxActive = engine?.engine === "whisperx";
  const whisperxDetailVisible = whisperxGuide || whisperxActive;

  useEffect(() => {
    if (external?.enabled) setAnalysisMode("external");
  }, [external?.enabled]);

  return (
    <Stack>
      <p className="em-hint">
        {summary(
          status,
          engine,
          transcriptionMode,
          analysisMode,
          external,
          props.catalog,
        )}
      </p>

      {/* ---------------- 转写 ---------------- */}
      <Card
        title="转写"
        description={
          transcriptionMode === "local"
            ? "把录音变成文字。音频只在这台机器上处理，不上传。"
            : "把录音上传给第三方服务商，返回逐字稿。"
        }
        status={
          transcriptionMode === "external" ? (
            <ExternalVerifyPill
              target="transcription"
              settings={external}
              health={props.providerHealth.transcription}
              credentialRevision={props.credentialRevisions.transcription}
              catalogVersion={props.catalogVersion}
              fallbackState={asrReady ? "unverified" : "unconfigured"}
            />
          ) : (
            <StatusPill ok={status.whisperAvailable} okText="可用" badText="缺模型" />
          )
        }
      >
        <Segmented
          label="转写方式"
          value={transcriptionMode}
          onChange={(next) => {
            void props.onSaveExternal({ transcriptionMode: next }).catch(() => undefined);
          }}
          options={[
            { value: "local", label: "本机", hint: "音频只在本机处理，不上传。" },
            { value: "external", label: "第三方", hint: "选择服务商并粘贴凭据，地址与模型自动配置。" },
          ]}
        />
        {transcriptionMode === "local" ? (
          <>
            <Segmented
              label="转写引擎"
              value={(engine?.engine ?? "embedded") as "embedded" | "whisperx"}
              onChange={(next) => {
                // 没装 whisperX 就直接切过去，等于把用户推进一个跑不了的状态。
                // 这里改成先摊开安装说明，装好再切。
                if (next === "whisperx" && !engine?.whisperxAvailable) {
                  setWhisperxGuide(true);
                  return;
                }
                setWhisperxGuide(false);
                props.onSwitchEngine(next);
              }}
              options={[
                { value: "embedded", label: "内嵌引擎", hint: "默认，零依赖，不需要装任何外部程序" },
                {
                  value: "whisperx",
                  label: engine?.whisperxAvailable ? "whisperX（实验）" : "whisperX（实验，未检测到）",
                  hint: engine?.whisperxAvailable
                    ? "实验特性 · 说话人分离：自动区分谁在说话。whisperX 与 pyannote 全部在你这台 Mac 上运行，音频不出本机；首次转写需联网下载转写与分离模型"
                    : "实验特性 · 需要先安装，见下方安装说明",
                },
              ]}
            />

            <WhisperModelList
              status={status}
              download={recommendedDownload}
              onSelect={(path) => props.onSaveSettings({ whisperModelPath: path })}
              onDownload={(modelId) => props.onStartDownload("whisper", modelId)}
              onCancel={props.onCancelDownload}
              onChooseFile={props.onChooseWhisperFile}
            />
          </>
        ) : external && props.catalog ? (
          <ProviderConnectionForm
            target="transcription"
            catalog={props.catalog}
            settings={external}
            busy={busy}
            actionLabel="保存并验证"
            onCommit={(draft) => props.onCommitProvider("transcription", draft)}
            onClearCredentials={() =>
              props.onClearProviderCredentials("transcription")
            }
            onReuseCredentials={(draft) =>
              props.onReuseProviderCredentials("transcription", draft)
            }
            onRevealCredentials={(providerId) =>
              props.onRevealProviderCredentials("transcription", providerId)
            }
            onListModels={(input) =>
              props.onListProviderModels(
                "transcription",
                input.providerId,
                input.baseUrl,
                input.apiKey,
              )
            }
            onDiagnose={(draft) => props.onDiagnoseProvider("transcription", draft)}
            onSaveAndVerify={(draft) => props.onSaveAndVerifyProvider("transcription", draft)}
            onVerifySaved={() => props.onVerifySavedProvider("transcription")}
            health={props.providerHealth.transcription}
            credentialRevision={props.credentialRevisions.transcription}
            catalogVersion={props.catalogVersion}
          />
        ) : (
          <p className="em-empty">正在读取服务商目录…</p>
        )}

        {transcriptionMode === "local" && whisperxDetailVisible && (
          <Collapsible
            title="配置说话人分离（实验特性，3 步）"
            open={whisperxDetailVisible}
            onToggle={setWhisperxGuide}
          >
            <ol className="whisperx-setup-steps">
              <li>
                <div className="em-card-foot">
                  <strong>1. 安装并检测 WhisperX</strong>
                  <StatusPill
                    ok={Boolean(engine?.whisperxAvailable)}
                    okText="已完成"
                    badText="未完成"
                  />
                </div>
                <p className="em-hint">
                  {engine?.whisperxAvailable
                    ? "已检测到 whisperx，可继续下一步。"
                    : "未检测到 whisperx。把下面这条命令复制到「终端」App 里运行即可，全程约几分钟，装好后回来点「重新检测」。"}
                </p>
                <code>uv tool install --python 3.11 whisperx</code>
                <p className="em-hint">
                  上面的命令需要先装 uv（同样复制进终端运行：<code>curl -LsS https://astral.sh/uv/install.sh | sh</code>）。
                  装不上也没关系，whisperX 是可选的实验功能，用默认内嵌引擎一样能转写，只是不区分说话人。
                </p>
                <div className="em-card-actions" style={{ marginTop: 8 }}>
                  {whisperxActive && (
                    <Button onClick={() => props.onSwitchEngine("embedded")}>先切回内嵌引擎</Button>
                  )}
                  <Button onClick={props.onRefresh}>重新检测</Button>
                </div>
              </li>
              <li>
                <div className="em-card-foot">
                  <strong>2. 授权三个模型</strong>
                  <StatusPill ok={false} okText="已完成" badText="待确认" />
                </div>
                <p className="em-hint">
                  登录 Hugging Face，在下列模型页逐一同意使用条款：
                </p>
                {[
                  "pyannote/speaker-diarization-community-1",
                  "pyannote/segmentation-3.0",
                  "pyannote/speaker-diarization-3.1",
                ].map((model) => (
                  <div className="whisperx-model-row" key={model}>
                    <code>{model}</code>
                    <Button
                      variant="quiet"
                      onClick={() => void navigator.clipboard?.writeText(model)}
                    >
                      复制
                    </Button>
                  </div>
                ))}
              </li>
              <li>
                <div className="em-card-foot">
                  <strong>3. 保存 HuggingFace Token</strong>
                  <StatusPill
                    ok={Boolean(engine?.hfTokenSet)}
                    okText="已完成"
                    badText="未完成"
                  />
                </div>
                <Field
                  label="HuggingFace Token"
                  hint={
                    engine?.hfTokenSet
                      ? "已保存在 macOS 钥匙串里，不会回显。"
                      : "创建 Read 权限 Token 后填写。"
                  }
                >
                  <HfTokenInput
                    busy={busy}
                    configured={Boolean(engine?.hfTokenSet)}
                    onSave={props.onSaveHfToken}
                    onClear={props.onClearHfToken}
                  />
                </Field>
              </li>
            </ol>
          </Collapsible>
        )}

        <Field label="转写语言">
          <select
            className="em-select"
            value={status.settings.transcriptionLanguage}
            onChange={(event) => props.onSaveSettings({ transcriptionLanguage: event.target.value })}
          >
            {LANGUAGES.map(([value, label]) => (
              <option value={value} key={value}>
                {label}
              </option>
            ))}
          </select>
        </Field>

        <p className="em-hint">
          {preprocessorText(props.preprocessor)}
        </p>
      </Card>

      {/* ---------------- 分析 ---------------- */}
      <Card
        title="分析"
        description="把逐字稿变成摘要、决策和待办。本机用 Ollama，第三方用你选择的文本模型。"
        status={
          analysisMode === "external" ? (
            <ExternalVerifyPill
              target="analysis"
              settings={external}
              health={props.providerHealth.analysis}
              credentialRevision={props.credentialRevisions.analysis}
              catalogVersion={props.catalogVersion}
              fallbackState={externalAnalysisReady ? "unverified" : "unconfigured"}
            />
          ) : (
            <StatusPill
              ok={currentAnalysisReady}
              okText="可用"
              badText={analysisMode === "local" ? "未运行" : "待配置"}
            />
          )
        }
        actions={
          props.onOpenAnalysisSetup ? (
            <Button onClick={props.onOpenAnalysisSetup}>配置分析…</Button>
          ) : undefined
        }
      >
        <Segmented
          label="分析方式"
          value={analysisMode}
          onChange={(next) => {
            setAnalysisMode(next);
            if (next === "local") {
              void props.onSaveExternal({ enabled: false }).catch(() => undefined);
            }
          }}
          options={[
            { value: "local", label: "本机", hint: "使用本机 Ollama，逐字稿不离开设备。" },
            { value: "external", label: "第三方", hint: "选择服务商并粘贴凭据，地址与推荐模型自动配置。" },
          ]}
        />
        {analysisMode === "local" ? (
          <>
            {!status.ollamaAvailable && (
              <p className="em-note warn">
                没有检测到 Ollama 服务。装好并启动 Ollama（ollama.com）之后，点「重新检测」。
              </p>
            )}
            <OllamaModelList
              models={status.ollamaModels}
              selected={status.settings.analysisModel}
              suggestions={ANALYSIS_MODEL_SUGGESTIONS}
              download={analysisDownload}
              available={status.ollamaAvailable}
              role="analysis"
              onSelect={(name) => props.onSaveSettings({ analysisModel: name })}
              onDownload={(name) => props.onStartDownload("ollama", name)}
              onCancel={props.onCancelDownload}
            />
            <div className="em-card-foot">
              <span className="em-hint">本地模型不需要 API Key 和接口地址。</span>
              <Button onClick={props.onRefresh}>重新检测</Button>
            </div>
          </>
        ) : external && props.catalog ? (
          <ProviderConnectionForm
            target="analysis"
            catalog={props.catalog}
            settings={external}
            busy={busy}
            actionLabel="保存并验证"
            onCommit={(draft) => props.onCommitProvider("analysis", draft)}
            onClearCredentials={() => props.onClearProviderCredentials("analysis")}
            onReuseCredentials={(draft) =>
              props.onReuseProviderCredentials("analysis", draft)
            }
            onRevealCredentials={(providerId) =>
              props.onRevealProviderCredentials("analysis", providerId)
            }
            onListModels={(input) =>
              props.onListProviderModels(
                "analysis",
                input.providerId,
                input.baseUrl,
                input.apiKey,
              )
            }
            onDiagnose={(draft) => props.onDiagnoseProvider("analysis", draft)}
            onSaveAndVerify={(draft) => props.onSaveAndVerifyProvider("analysis", draft)}
            onVerifySaved={() => props.onVerifySavedProvider("analysis")}
            health={props.providerHealth.analysis}
            credentialRevision={props.credentialRevisions.analysis}
            catalogVersion={props.catalogVersion}
          />
        ) : (
          <p className="em-empty">正在读取服务商目录…</p>
        )}
      </Card>

      {/* ---------------- 知识索引 ---------------- */}
      <Card
        title="知识索引"
        description="把逐字稿切块并向量化，让「跨全部录音」的问答和检索成为可能。"
        status={<StatusPill ok={status.ollamaAvailable} okText="可用" badText="未运行" />}
      >
        <OllamaModelList
          models={status.ollamaModels}
          selected={status.settings.embeddingModel}
          suggestions={[{ name: status.settings.embeddingModel, hint: "当前的嵌入模型" }]}
          download={embeddingDownload}
          available={status.ollamaAvailable}
          role="embedding"
          onSelect={(name) => props.onSaveSettings({ embeddingModel: name })}
          onDownload={(name) => props.onStartDownload("ollama", name)}
          onCancel={props.onCancelDownload}
        />
        <div className="em-card-foot">
          <span className="em-hint">换嵌入模型会让已有索引失效，需要重建。</span>
          <Button onClick={props.onOpenKnowledge}>去重建索引</Button>
        </div>
      </Card>

      {props.error && <p className="em-note err" role="alert">{props.error}</p>}
    </Stack>
  );
}

function HfTokenInput({
  busy,
  configured,
  onSave,
  onClear,
}: {
  busy: boolean;
  configured: boolean;
  onSave: (token: string) => void;
  onClear: () => void;
}) {
  const [token, setToken] = useState("");
  return (
    <div className="em-field-row">
      <input
        className="em-input"
        type="password"
        value={token}
        autoComplete="new-password"
        placeholder={configured ? "填写新 Token 可覆盖旧的" : "hf_xxx（仅保存，不回显）"}
        onChange={(event) => setToken(event.target.value)}
      />
      <div className="em-card-actions">
        <Button variant="ghost" disabled={busy || !token.trim()} onClick={() => { onSave(token.trim()); setToken(""); }}>
          保存
        </Button>
        <Button variant="danger" disabled={busy || !configured} onClick={onClear}>
          清除
        </Button>
      </div>
    </div>
  );
}

/** 一行说清现在到底能不能用、用的是谁。 */
function summary(
  status: LocalAiStatus,
  engine: TranscriptionEngineStatus | null,
  transcriptionMode: "local" | "external",
  analysisMode: "local" | "external",
  external: ExternalAiSettings | null,
  catalog: ProviderCatalog | null,
): string {
  const transcription = transcriptionMode === "local"
    ? status.whisperAvailable
      ? `${engine?.engine === "whisperx" ? "whisperX" : "内嵌 Whisper"} · ${status.whisperModelPath?.split("/").pop() ?? "已就绪"}`
      : "本机转写待配置"
    : external &&
        !["", "none"].includes(external.transcriptionProvider) &&
        external.transcriptionHasApiKey &&
        external.audioUploadConsentAt
      ? `${providerLabel(catalog, external.transcriptionProvider)} · ${external.transcriptionModel || "待配置"}`
      : "第三方转写待配置";
  const analysis = analysisMode === "local"
    ? `${status.ollamaAvailable ? "本机 Ollama" : "Ollama 未运行"} · ${status.settings.analysisModel}`
    : analysisReady(external)
      ? `${providerLabel(catalog, external?.analysisProvider ?? "")} · ${external?.model || "待配置"}`
      : "第三方分析待配置";
  return `转写 ${transcription} ｜ 分析 ${analysis} ｜ 索引 ${status.settings.embeddingModel}`;
}

function analysisReady(external: ExternalAiSettings | null): boolean {
  return Boolean(
    external?.enabled &&
      external.hasApiKey &&
      external.privacyConsentAt &&
      external.baseUrl.trim() &&
      external.model.trim() &&
      external.analysisProvider.trim(),
  );
}

/** 卡片头的第三方六态药丸：与表单状态条同一推导函数、同一词表。 */
function ExternalVerifyPill({
  target,
  settings,
  health,
  credentialRevision,
  catalogVersion,
  fallbackState,
}: {
  target: "analysis" | "transcription";
  settings: ExternalAiSettings | null;
  health: ProviderHealthSnapshot | null;
  credentialRevision: number;
  catalogVersion: number;
  fallbackState: "unverified" | "unconfigured";
}) {
  if (!settings) return <StatusPill ok={null} />;
  const state = deriveVerifyState({
    hasCredentials: target === "analysis" ? settings.hasApiKey : settings.transcriptionHasApiKey,
    providerId: target === "analysis" ? settings.analysisProvider : settings.transcriptionProvider,
    baseUrl: target === "analysis" ? settings.baseUrl : settings.transcriptionBaseUrl,
    model: target === "analysis" ? settings.model : settings.transcriptionModel,
    health,
    credentialRevision,
    catalogVersion,
    draftDeviation: false,
    busy: false,
  });
  const description =
    state === "verified"
      ? verifiedDescription(health)
      : state === "failed"
        ? failedDescription(health)
        : null;
  // 卡片头窄：长描述只保留到冒号前。
  return (
    <VerifyStatusPill
      state={state === "unverified" ? fallbackState : state}
      description={description}
    />
  );
}

function providerLabel(catalog: ProviderCatalog | null, providerId: string): string {
  return (
    catalog?.providers.find((provider) => provider.id === providerId)?.label ??
    (providerId && providerId !== "none" ? providerId : "第三方服务")
  );
}

function preprocessorText(preprocessor: AudioPreprocessorStatus | null): string {
  if (!preprocessor) return "正在读取音频预处理状态…";
  if (!preprocessor.enhancedAvailable) return "音频预处理：FFmpeg 不可用，将使用兼容模式";
  // 后端给的 version 是 `ffmpeg -version` 的整行，里面带着版权声明；
  // 界面上只要版本号，不要那一段法律文本。
  const version = preprocessor.version?.match(/\d+\.\d+(\.\d+)?/)?.[0];
  return `音频预处理：增强模式 · FFmpeg${version ? ` ${version}` : ""}`;
}

export { formatBytes };
