/**
 * 「AI 模型」tab：把原来的「本机 AI」和「外部 AI」并成一页。
 *
 * 它们其实回答同一个问题——「谁来转写、谁来分析」。所以按三件事分组：
 * 转写（引擎 + 模型）、分析（本机 Ollama 或外部 API）、知识索引（嵌入模型）。
 * 本地只是其中一个选项，不是另一套信息架构。
 *
 * 所有改动即时生效，成功飘一句「已保存」，失败留在该卡片自己的提示里。
 */
import { useState, type ReactNode } from "react";
import type {
  AudioPreprocessorStatus,
  ExternalAiSettings,
  KnowledgeSettings,
  LocalAiStatus,
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
  ToggleRow,
  useDraftSave,
} from "./SettingsKit";
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
  downloads: Record<string, DownloadState>;
  busy: boolean;
  onSaveSettings: (patch: Partial<KnowledgeSettings>) => void;
  onStartDownload: (kind: DownloadKind, model: string) => void;
  onCancelDownload: (model: string) => void;
  onSwitchEngine: (next: "embedded" | "whisperx") => void;
  onSaveHfToken: (token: string) => void;
  onClearHfToken: () => void;
  onChooseWhisperFile: () => void;
  onSaveExternal: (patch: Partial<ExternalAiSettings>) => void;
  onSaveExternalKey: (key: string) => void;
  onClearExternalKey: () => void;
  onSaveExternalAsrKey: (key: string) => void;
  onClearExternalAsrKey: () => void;
  onTestExternal: () => void;
  onRefresh: () => void;
  onOpenKnowledge: () => void;
  testResult: ReactNode;
  error: string | null;
}

export default function AiModelSettings(props: AiModelSettingsProps) {
  const { status, engine, external, downloads, busy } = props;
  const recommendedDownload = downloads[status.recommendedWhisperModel.id];
  const analysisDownload = downloads[status.settings.analysisModel];
  const embeddingDownload = downloads[status.settings.embeddingModel];
  const channel = external?.processingMode ?? "local";
  const asrReady = Boolean(
    channel === "external" &&
      external &&
      !["", "none"].includes(external.transcriptionProvider) &&
      external.transcriptionBaseUrl.trim() &&
      external.transcriptionModel.trim() &&
      external.transcriptionHasApiKey &&
      external.audioUploadConsentAt,
  );
  const externalAnalysisReady = analysisReady(status, channel, external);
  const externalMissing = channel === "external"
    ? [
        !asrReady && "第三方转写",
        !externalAnalysisReady && "第三方理解整理",
      ].filter(Boolean)
    : [];
  const [whisperxGuide, setWhisperxGuide] = useState(false);
  const whisperxActive = engine?.engine === "whisperx";
  const whisperxDetailVisible = whisperxGuide || whisperxActive;

  return (
    <Stack>
      <p className="em-hint">
        {summary(status, engine, channel, external)}
      </p>

      <Card
        title="录音处理通道"
        description="选择新录音和手动导入音频由谁完成转写与结构化分析。"
        status={
          <StatusPill
            ok={channel === "local" || Boolean(external?.enabled && external.hasApiKey)}
            okText="已配置"
            badText="待配置"
          />
        }
      >
        <Segmented
          label="处理方式"
          value={channel}
          onChange={(next) => props.onSaveExternal({ processingMode: next })}
          options={[
            { value: "local", label: "本机处理", hint: "Whisper 转写，Ollama 分析，音频与文本不出本机" },
            { value: "external", label: "第三方处理", hint: "分别配置语音转文字与理解整理模型" },
          ]}
        />
        {channel === "external" && (
          <p className="em-note">
            第三方处理需要分别确认音频上传与文本发送说明，并填写服务商的 API Key。
          </p>
        )}
        {externalMissing.length > 0 && (
          <p className="em-note warn">
            第三方通道待完成：{externalMissing.join("、")}。完成配置后新导入的录音才会走完整链路。
          </p>
        )}
      </Card>

      {/* ---------------- 转写 ---------------- */}
      <Card
        title="转写"
        description={
          channel === "local"
            ? "把录音变成文字。音频只在这台机器上处理，不上传。"
            : "把录音上传给第三方服务商，返回逐字稿。"
        }
        status={
          <StatusPill
            ok={channel === "local" ? status.whisperAvailable : asrReady}
            okText="可用"
            badText={channel === "local" ? "缺模型" : "待配置"}
          />
        }
      >
        {channel === "local" ? (
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
                  label: engine?.whisperxAvailable ? "whisperX" : "whisperX（未检测到）",
                  hint: engine?.whisperxAvailable
                    ? "说话人分离：自动区分谁在说话"
                    : "需要先安装，见下方安装说明",
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
        ) : (
          <ExternalAsrChannel
            external={external}
            busy={busy}
            onSave={props.onSaveExternal}
            onSaveKey={props.onSaveExternalAsrKey}
            onClearKey={props.onClearExternalAsrKey}
          />
        )}

        {channel === "local" && whisperxDetailVisible && (
          <Collapsible
            title="配置说话人分离（3 步）"
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
                    : "未检测到 whisperx，请先在终端安装。"}
                </p>
                <code>uv tool install --python 3.11 whisperx</code>
                <p className="em-hint">
                  没有 uv 时先安装 uv，或在 Python 3.11/3.12 虚拟环境中使用
                  <code>pip install whisperx</code>。
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
        description="把逐字稿变成摘要、决策和待办，按当前处理通道执行。"
        status={<StatusPill ok={externalAnalysisReady} okText="可用" badText={channel === "local" ? "未运行" : "未配置"} />}
      >
        {channel === "local" ? (
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
        ) : (
          <ExternalChannel
            external={external}
            busy={busy}
            testResult={props.testResult}
            onSave={props.onSaveExternal}
            onSaveKey={props.onSaveExternalKey}
            onClearKey={props.onClearExternalKey}
            onTest={props.onTestExternal}
          />
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

      {props.error && <p className="em-note err">{props.error}</p>}
    </Stack>
  );
}

function ExternalAsrChannel({
  external,
  busy,
  onSave,
  onSaveKey,
  onClearKey,
}: {
  external: ExternalAiSettings | null;
  busy: boolean;
  onSave: (patch: Partial<ExternalAiSettings>) => void;
  onSaveKey: (key: string) => void;
  onClearKey: () => void;
}) {
  const [apiKey, setApiKey] = useState("");
  const [revealed, setRevealed] = useState(false);
  const { draft, update, flush } = useDraftSave(
    {
      transcriptionBaseUrl: external?.transcriptionBaseUrl ?? "",
      transcriptionModel: external?.transcriptionModel ?? "",
      transcriptionProvider:
        external && !["", "none"].includes(external.transcriptionProvider)
          ? external.transcriptionProvider
          : "",
    },
    (next) => {
      if (!external) return;
      const patch: Partial<ExternalAiSettings> = {};
      if (
        next.transcriptionBaseUrl.trim() !== external.transcriptionBaseUrl &&
        looksLikeUrl(next.transcriptionBaseUrl)
      ) {
        patch.transcriptionBaseUrl = next.transcriptionBaseUrl.trim();
      }
      if (
        next.transcriptionModel.trim() !== external.transcriptionModel &&
        next.transcriptionModel.trim().length <= 160
      ) {
        patch.transcriptionModel = next.transcriptionModel.trim();
      }
      if (next.transcriptionProvider !== external.transcriptionProvider) {
        patch.transcriptionProvider = next.transcriptionProvider;
      }
      if (Object.keys(patch).length > 0) onSave(patch);
    },
    null,
  );

  if (!external) return <p className="em-empty">正在读取第三方转写设置…</p>;

  return (
    <>
      <p className="em-note warn">
        选择第三方转写后，原始音频会上传给下方配置的服务商，直到你清除该配置或切回本机处理。
      </p>
      <p className="em-hint">
        说话人标签取决于服务商返回能力；未返回时不会根据文本猜测。
      </p>

      <Field label="服务类型" hint="千问、智谱等服务商可使用 OpenAI 兼容地址。">
        <select
          className="em-select"
          value={draft.transcriptionProvider}
          onChange={(event) => update({ transcriptionProvider: event.target.value })}
          onBlur={flush}
        >
          <option value="">请选择语音转文字服务</option>
          <option value="openai-compatible">通用 OpenAI 兼容接口</option>
          <option value="dashscope">阿里云百炼 / 千问</option>
          <option value="zhipu">智谱 AI</option>
        </select>
      </Field>

      <Field label="接口地址" hint="音频转写接口地址，只有本机地址允许用 http。">
        <input
          className="em-input"
          value={draft.transcriptionBaseUrl}
          placeholder="https://dashscope.aliyuncs.com/compatible-mode/v1"
          onChange={(event) => update({ transcriptionBaseUrl: event.target.value })}
          onBlur={flush}
        />
      </Field>

      <Field label="转写模型" hint="服务商文档里的模型 ID，例如 qwen3-asr-flash。">
        <input
          className="em-input"
          value={draft.transcriptionModel}
          placeholder="qwen3-asr-flash"
          onChange={(event) => update({ transcriptionModel: event.target.value })}
          onBlur={flush}
        />
      </Field>

      <Field
        label="语音转文字 API Key"
        hint={
          external.transcriptionHasApiKey
            ? "已保存在 macOS 钥匙串里，不会回显。"
            : "还没有配置。"
        }
      >
        <div className="em-field-row">
          <input
            className="em-input"
            type={revealed ? "text" : "password"}
            value={apiKey}
            autoComplete="new-password"
            placeholder={
              external.transcriptionHasApiKey ? "填写新 Key 可覆盖旧的" : "仅用于保存，不会回显"
            }
            onChange={(event) => setApiKey(event.target.value)}
          />
          <div className="em-card-actions">
            <Button variant="quiet" onClick={() => setRevealed((value) => !value)}>
              {revealed ? "隐藏" : "显示"}
            </Button>
            <Button
              variant="ghost"
              disabled={busy || !apiKey.trim()}
              onClick={() => {
                onSaveKey(apiKey.trim());
                setApiKey("");
              }}
            >
              保存
            </Button>
            <Button
              variant="danger"
              disabled={busy || !external.transcriptionHasApiKey}
              onClick={onClearKey}
            >
              清除
            </Button>
          </div>
        </div>
      </Field>

      <label className="em-check-row">
        <input
          type="checkbox"
          checked={Boolean(external.audioUploadConsentAt)}
          onChange={(event) =>
            onSave({
              audioUploadConsentAt: event.target.checked ? new Date().toISOString() : null,
            })
          }
        />
        <span>我已了解：第三方转写会把原始音频上传给我填写的服务商。</span>
      </label>
    </>
  );
}

function ExternalChannel({
  external,
  busy,
  testResult,
  onSave,
  onSaveKey,
  onClearKey,
  onTest,
}: {
  external: ExternalAiSettings | null;
  busy: boolean;
  testResult: ReactNode;
  onSave: (patch: Partial<ExternalAiSettings>) => void;
  onSaveKey: (key: string) => void;
  onClearKey: () => void;
  onTest: () => void;
}) {
  const [apiKey, setApiKey] = useState("");
  const [revealed, setRevealed] = useState(false);
  const { draft, update, flush } = useDraftSave(
    { baseUrl: external?.baseUrl ?? "", model: external?.model ?? "" },
    (next) => {
      if (!external) return;
      const patch: Partial<ExternalAiSettings> = {};
      // 明显不成形的地址不写库：后端会拒绝，用户还会看到一个吓人的报错。
      if (next.baseUrl.trim() !== external.baseUrl && looksLikeUrl(next.baseUrl)) {
        patch.baseUrl = next.baseUrl.trim();
      }
      if (next.model.trim() !== external.model && next.model.trim().length <= 160) {
        patch.model = next.model.trim();
      }
      if (Object.keys(patch).length > 0) onSave(patch);
    },
    // 地址和模型名只在失焦时落盘：敲到一半的地址不该被当成最终意图。
    null,
  );

  if (!external) return <p className="em-empty">正在读取外部 AI 设置…</p>;

  return (
    <>
      <p className="em-note">
        开启后，逐字稿全文和现有结构化分析会发送给你配置的服务商。此设置只控制理解与整理；
        音频是否上传由上方的转写通道决定。
      </p>

      <ToggleRow
        label="启用理解与结构化整理"
        checked={external.enabled}
        onChange={(enabled) => onSave({ enabled })}
      />

      <Field label="接口地址" hint="OpenAI 兼容的 /v1 地址。只有本机地址允许用 http。">
        <input
          className="em-input"
          value={draft.baseUrl}
          placeholder="https://api.openai.com/v1"
          onChange={(event) => update({ baseUrl: event.target.value })}
          onBlur={flush}
        />
      </Field>

      <Field label="模型名称" hint="服务商文档里的模型 ID，例如 gpt-4.1-mini。">
        <input
          className="em-input"
          value={draft.model}
          placeholder="gpt-4.1-mini"
          onChange={(event) => update({ model: event.target.value })}
          onBlur={flush}
        />
      </Field>

      <Field
        label="API Key"
        hint={external.hasApiKey ? "已保存在 macOS 钥匙串里，不会回显。" : "还没有配置，启用前必须先填。"}
      >
        <div className="em-field-row">
          <input
            className="em-input"
            type={revealed ? "text" : "password"}
            value={apiKey}
            autoComplete="new-password"
            placeholder={external.hasApiKey ? "填写新 Key 可覆盖旧的" : "仅用于保存，不会回显"}
            onChange={(event) => setApiKey(event.target.value)}
          />
          <div className="em-card-actions">
            <Button variant="quiet" onClick={() => setRevealed((value) => !value)}>
              {revealed ? "隐藏" : "显示"}
            </Button>
            <Button
              variant="ghost"
              disabled={busy || !apiKey.trim()}
              onClick={() => {
                onSaveKey(apiKey.trim());
                setApiKey("");
              }}
            >
              保存
            </Button>
            <Button variant="danger" disabled={busy || !external.hasApiKey} onClick={onClearKey}>
              清除
            </Button>
            <Button variant="ghost" disabled={busy || !external.hasApiKey} onClick={onTest}>
              {testResult ?? "验证"}
            </Button>
          </div>
        </div>
      </Field>

      <label className="em-check-row">
        <input
          type="checkbox"
          checked={Boolean(external.privacyConsentAt)}
          onChange={(event) => onSave({ privacyConsentAt: event.target.checked ? new Date().toISOString() : null })}
        />
        <span>我已了解：启用后会发送选定记录的逐字稿全文和现有结构化分析，但不会发送音频。</span>
      </label>
    </>
  );
}

function looksLikeUrl(value: string): boolean {
  const trimmed = value.trim();
  if (!/^https?:\/\//i.test(trimmed)) return false;
  try {
    new URL(trimmed);
    return true;
  } catch {
    return false;
  }
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
  channel: "local" | "external",
  external: ExternalAiSettings | null,
): string {
  if (channel === "external") {
    const asrReady = Boolean(
      external &&
        !["", "none"].includes(external.transcriptionProvider) &&
        external.transcriptionBaseUrl.trim() &&
        external.transcriptionModel.trim() &&
        external.transcriptionHasApiKey &&
        external.audioUploadConsentAt,
    );
    const analysisIsReady = analysisReady(status, "external", external);
    const asr = asrReady
      ? `${external?.transcriptionProvider ?? "第三方服务"} · ${external?.transcriptionModel || "待配置"}`
      : "第三方转写待配置";
    const analysis = analysisIsReady
      ? `外部模型 · ${external?.model || "待配置"}`
      : "第三方理解待配置";
    return `转写 ${asr} ｜ 分析 ${analysis} ｜ 索引 ${status.settings.embeddingModel}`;
  }
  const whisper = status.whisperAvailable
    ? `${engine?.engine === "whisperx" ? "whisperX" : "内嵌 Whisper"} · ${status.whisperModelPath?.split("/").pop() ?? "已就绪"}`
    : "还没有可用的转写模型";
  const analysis = status.settings.analysisModel;
  const ollama = status.ollamaAvailable ? "本机 Ollama" : "Ollama 未运行";
  return `转写 ${whisper} ｜ 分析 ${ollama} · ${analysis} ｜ 索引 ${status.settings.embeddingModel}`;
}

function analysisReady(
  status: LocalAiStatus,
  channel: "local" | "external",
  external: ExternalAiSettings | null,
): boolean {
  if (channel === "external") {
    return Boolean(
      external?.enabled &&
        external.hasApiKey &&
        external.privacyConsentAt &&
        external.baseUrl.trim() &&
        external.model.trim(),
    );
  }
  return status.ollamaAvailable && status.ollamaModels.some((model) => model.name === status.settings.analysisModel);
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
