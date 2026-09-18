/**
 * 「AI 模型」tab：把原来的「本机 AI」和「外部 AI」并成一页。
 *
 * 它们其实回答同一个问题——「谁来转写、谁来分析」。所以按三件事分组：
 * 转写（引擎 + 模型）、分析（本机 Ollama 或外部 API）、知识索引（嵌入模型）。
 * 本地只是其中一个选项，不是另一套信息架构。
 *
 * 所有改动即时生效，成功飘一句「已保存」，失败留在该卡片自己的提示里。
 */
import { useState } from "react";
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
  onTestExternal: () => void;
  onRefresh: () => void;
  onOpenKnowledge: () => void;
  testResult: string | null;
  error: string | null;
}

export default function AiModelSettings(props: AiModelSettingsProps) {
  const { status, engine, external, downloads, busy } = props;
  const recommendedDownload = downloads[status.recommendedWhisperModel.id];
  const analysisDownload = downloads[status.settings.analysisModel];
  const embeddingDownload = downloads[status.settings.embeddingModel];
  const [channel, setChannel] = useState<"local" | "external">(external?.enabled ? "external" : "local");
  const [whisperxGuide, setWhisperxGuide] = useState(false);
  const whisperxActive = engine?.engine === "whisperx";
  const whisperxDetailVisible = whisperxGuide || whisperxActive;

  return (
    <Stack>
      <p className="em-hint">
        {summary(status, engine)}
      </p>

      {/* ---------------- 转写 ---------------- */}
      <Card
        title="转写"
        description="把录音变成文字。音频只在这台机器上处理，不上传。"
        status={<StatusPill ok={status.whisperAvailable} okText="可用" badText="缺模型" />}
      >
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
                : "需要先安装：pip install whisperx",
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

        {whisperxDetailVisible && !engine?.whisperxAvailable && (
          <div className="em-note warn">
            <p style={{ margin: 0 }}>
              没有找到 whisperx 命令，选它转写会直接失败。安装方法：<code>pip install whisperx</code>，
              装好后回到这里点「重新检测」。
            </p>
            <div className="em-card-actions" style={{ marginTop: 8 }}>
              {whisperxActive && (
                <Button onClick={() => props.onSwitchEngine("embedded")}>先切回内嵌引擎</Button>
              )}
              <Button onClick={props.onRefresh}>重新检测</Button>
            </div>
          </div>
        )}
        {whisperxDetailVisible && (
          <Collapsible
            title={`说话人分离设置${engine?.hfTokenSet ? "（Token 已配置）" : "（还差 HuggingFace Token）"}`}
          >
            <p className="em-hint">
              说话人分离要靠 HuggingFace 上的 pyannote 模型：先在 huggingface.co 接受该模型的使用协议，
              再把 Token 填在这里。首次使用会自动下载模型，建议在独显或大内存机器上跑。
            </p>
            <Field
              label="HuggingFace Token"
              hint={engine?.hfTokenSet ? "已保存在 macOS 钥匙串里，不会回显。" : "还没有配置。"}
            >
              <HfTokenInput
                busy={busy}
                configured={Boolean(engine?.hfTokenSet)}
                onSave={props.onSaveHfToken}
                onClear={props.onClearHfToken}
              />
            </Field>
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
        description="把逐字稿变成摘要、决策和待办。可以完全跑在本机，也可以交给外部 API。"
        status={<StatusPill ok={analysisReady(status, channel, external)} okText="可用" badText={channel === "local" ? "未运行" : "未配置"} />}
      >
        <Segmented
          label="分析由谁来做"
          value={channel}
          onChange={setChannel}
          options={[
            { value: "local", label: "本机 Ollama", hint: "文本不出本机" },
            { value: "external", label: "外部 API", hint: "把逐字稿文本发给服务商，音频不发" },
          ]}
        />

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
  testResult: string | null;
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
        开启后，所选记录的结构化分析和逐字稿全文会发送给你配置的服务商；
        <strong>原始音频永不上传</strong>。本机转写、单条分析、知识库问答和 MCP 仍然全部在本机。
      </p>

      <ToggleRow
        label="启用外部 AI（跨记录记忆生成）"
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
function summary(status: LocalAiStatus, engine: TranscriptionEngineStatus | null): string {
  const whisper = status.whisperAvailable
    ? `${engine?.engine === "whisperx" ? "whisperX" : "内嵌 Whisper"} · ${status.whisperModelPath?.split("/").pop() ?? "已就绪"}`
    : "还没有可用的转写模型";
  const analysis = status.settings.analysisModel;
  const ollama = status.ollamaAvailable ? "本机 Ollama" : "Ollama 未运行";
  return `转写 ${whisper} ｜ 分析 ${ollama} · ${analysis} ｜ 索引 ${status.settings.embeddingModel}`;
}

function analysisReady(status: LocalAiStatus, channel: "local" | "external", external: ExternalAiSettings | null): boolean {
  if (channel === "external") return Boolean(external?.enabled && external.hasApiKey);
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
