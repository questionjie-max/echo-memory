/**
 * 分析设置发起面板：分析 / 知识索引相关功能第一次被触发且所需模型未就绪时弹出。
 *
 * 两个并列选项卡（默认「本地」）：
 * - 本地：Ollama 安装 / 服务 / 分析模型 / 嵌入模型四行状态 + 一键拉取缺失模型；
 *   Ollama 未安装时给 ollama.com 引导并自动轮询，装好自动刷新。
 * - 云端：OpenAI 兼容通道，预填智谱 / DeepSeek，填 Key 即用；复用设置页的
 *   钥匙串与连通检测命令，不重写 HTTP 与凭据逻辑。云端只覆盖分析，
 *   嵌入模型仍需本机 Ollama —— 这一局限必须写在云端选项卡内，不藏。
 *
 * 会话行为：面板可关闭；关闭后同一会话不再自动弹出（可从设置页随时再开）；
 * 配置完成后自动继续原先触发的动作，不需要用户重点一次。
 */
import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import {
  getExternalAiSettings,
  getLocalAiStatus,
  getOllamaInstallStatus,
  setExternalAiApiKey,
  testExternalAiConnection,
  updateExternalAiSettings,
} from "../lib/tauri";
import type { ExternalAiSettings, LocalAiStatus, OllamaInstallStatus } from "../shared/types";
import { Button, Collapsible, Field, List, ListRow, Progress, Segmented, Stack } from "./SettingsKit";
import { useModelDownloads, localizePullError, type DownloadState } from "./useModelDownloads";

export interface AnalysisSetupOptions {
  /** 该动作是否还依赖知识索引（跨记录问答 / 记忆生成 → 需要嵌入模型）。 */
  needEmbedding?: boolean;
  /** 就绪后（或配置完成后）要继续执行的动作。 */
  action?: () => void;
}

interface AnalysisSetupContextValue {
  ensureReady: (opts: AnalysisSetupOptions) => Promise<void>;
  openSheet: (opts?: AnalysisSetupOptions) => void;
}

const AnalysisSetupContext = createContext<AnalysisSetupContextValue | null>(null);

/** 无 Provider 环境（旧测试 / 隔离渲染）下退回旧行为：动作直接执行。 */
const FALLBACK_CONTEXT: AnalysisSetupContextValue = {
  ensureReady: async ({ action }) => action?.(),
  openSheet: () => undefined,
};

export function useAnalysisSetup(): AnalysisSetupContextValue {
  return useContext(AnalysisSetupContext) ?? FALLBACK_CONTEXT;
}

/** 云端通道就绪：与后端 analyze 的路由判定同口径（启用 + Key + 同意 + 地址模型齐全）。 */
function cloudReady(external: ExternalAiSettings): boolean {
  return Boolean(
    external.enabled &&
      external.hasApiKey &&
      external.privacyConsentAt &&
      external.baseUrl.trim() &&
      external.model.trim(),
  );
}

export async function isAnalysisStackReady(needEmbedding: boolean): Promise<boolean> {
  const [local, external] = await Promise.all([getLocalAiStatus(), getExternalAiSettings()]);
  const localReady =
    local.ollamaAvailable &&
    local.ollamaModels.some((model) => model.name === local.settings.analysisModel);
  const embeddingReady =
    local.ollamaAvailable &&
    local.ollamaModels.some((model) => model.name === local.settings.embeddingModel);
  return (cloudReady(external) || localReady) && (!needEmbedding || embeddingReady);
}

export function AnalysisSetupProvider({ children }: { children: ReactNode }) {
  const [open, setOpen] = useState(false);
  const [needEmbedding, setNeedEmbedding] = useState(false);
  const pendingRef = useRef<(() => void) | null>(null);
  // 会话级标记：用户明确关过面板后，本次运行不再自动弹出。
  const dismissedRef = useRef(false);

  const ensureReady = useCallback(async ({ needEmbedding = false, action }: AnalysisSetupOptions) => {
    if (await isAnalysisStackReady(needEmbedding)) {
      action?.();
      return;
    }
    if (dismissedRef.current) {
      // 不再自动弹，但也不把功能堵死：动作照常执行，缺模型的错误由功能自身如实浮出。
      action?.();
      return;
    }
    pendingRef.current = action ?? null;
    setNeedEmbedding(needEmbedding);
    setOpen(true);
  }, []);

  const openSheet = useCallback((opts?: AnalysisSetupOptions) => {
    pendingRef.current = opts?.action ?? null;
    setNeedEmbedding(Boolean(opts?.needEmbedding));
    setOpen(true);
  }, []);

  const finishAndResume = useCallback(() => {
    setOpen(false);
    const action = pendingRef.current;
    pendingRef.current = null;
    action?.();
  }, []);

  const close = useCallback(() => {
    dismissedRef.current = true;
    pendingRef.current = null;
    setOpen(false);
  }, []);

  const context = useMemo<AnalysisSetupContextValue>(
    () => ({ ensureReady, openSheet }),
    [ensureReady, openSheet],
  );

  return (
    <AnalysisSetupContext.Provider value={context}>
      {children}
      {open && <AnalysisSetupSheet needEmbedding={needEmbedding} onResume={finishAndResume} onClose={close} />}
    </AnalysisSetupContext.Provider>
  );
}

const CLOUD_PRESETS = [
  { id: "zhipu", label: "智谱", baseUrl: "https://open.bigmodel.cn/api/paas/v4", model: "glm-4-flash" },
  { id: "deepseek", label: "DeepSeek", baseUrl: "https://api.deepseek.com/v1", model: "deepseek-chat" },
] as const;

type CloudPresetId = (typeof CLOUD_PRESETS)[number]["id"] | "custom";

function AnalysisSetupSheet({
  needEmbedding,
  onResume,
  onClose,
}: {
  needEmbedding: boolean;
  onResume: () => void;
  onClose: () => void;
}) {
  const [tab, setTab] = useState<"local" | "cloud">("local");
  return (
    <div className="onboarding-scrim" role="presentation">
      <section className="onboarding-panel" role="dialog" aria-modal="true" aria-label="分析设置">
        <header className="onboarding-header">
          <div>
            <h2>分析设置</h2>
            <p>
              {needEmbedding
                ? "跨记录问答与记忆生成还没就绪，先准备好分析能力"
                : "分析功能还没就绪，先准备好分析能力"}
            </p>
          </div>
          <Button onClick={onClose}>稍后再说</Button>
        </header>
        <div className="onboarding-body">
          <Stack>
            <Segmented
              label="准备方式"
              value={tab}
              onChange={setTab}
              options={[
                { value: "local", label: "本地（Ollama 一键准备）" },
                { value: "cloud", label: "云端（填 Key 即用）" },
              ]}
            />
            {tab === "local" ? (
              <LocalOllamaTab onResume={onResume} />
            ) : (
              <CloudKeyTab onResume={onResume} />
            )}
          </Stack>
        </div>
      </section>
    </div>
  );
}

/* ------------------------------ 本地：Ollama 一键准备 ------------------------------ */

function LocalOllamaTab({ onResume }: { onResume: () => void }) {
  const [install, setInstall] = useState<OllamaInstallStatus | null>(null);
  const [local, setLocal] = useState<LocalAiStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const [preparing, setPreparing] = useState(false);
  const refresh = useCallback(async (): Promise<{ install: OllamaInstallStatus; local: LocalAiStatus } | null> => {
    try {
      const [nextInstall, nextLocal] = await Promise.all([getOllamaInstallStatus(), getLocalAiStatus()]);
      setInstall(nextInstall);
      setLocal(nextLocal);
      return { install: nextInstall, local: nextLocal };
    } catch (reason) {
      setError(String(reason));
      return null;
    }
  }, []);
  // 后端 pull 命令是「起线程后立即返回」的，await start() 不代表下载结束；
  // 真正的完成只认进度事件的终态（done/failed），计数到齐后再做就绪判定。
  const missingRef = useRef<string[]>([]);
  const settledRef = useRef(0);
  const resumedRef = useRef(false);
  const { downloads, start } = useModelDownloads((settled) => {
    if (settled.phase === "failed") setError(settled.error ?? "模型拉取失败");
    if (settled.phase !== "done" && settled.phase !== "failed") return;
    settledRef.current += 1;
    if (missingRef.current.length === 0 || settledRef.current < missingRef.current.length) return;
    missingRef.current = [];
    void refresh().then((next) => {
      setPreparing(false);
      if (
        next &&
        !resumedRef.current &&
        next.local.ollamaModels.some((model) => model.name === next.local.settings.analysisModel) &&
        next.local.ollamaModels.some((model) => model.name === next.local.settings.embeddingModel)
      ) {
        resumedRef.current = true;
        onResume();
      }
    });
  });

  useEffect(() => {
    void refresh();
  }, [refresh]);

  // Ollama 未安装或服务未运行时自动轮询，装好/启动后自动刷新状态。
  useEffect(() => {
    if (install?.installed && install?.running) return;
    const timer = window.setInterval(() => void refresh(), 3_000);
    return () => window.clearInterval(timer);
  }, [install, refresh]);

  const analysisInstalled = Boolean(
    local?.ollamaAvailable &&
      local.ollamaModels.some((model) => model.name === local.settings.analysisModel),
  );
  const embeddingInstalled = Boolean(
    local?.ollamaAvailable &&
      local.ollamaModels.some((model) => model.name === local.settings.embeddingModel),
  );
  const missingModels = local
    ? [local.settings.analysisModel, local.settings.embeddingModel].filter(
        (name) => !local.ollamaModels.some((model) => model.name === name),
      )
    : [];
  const serviceReady = Boolean(install?.running);

  function prepareAll() {
    if (!local || missingModels.length === 0) return;
    setPreparing(true);
    setError(null);
    settledRef.current = 0;
    missingRef.current = [...missingModels];
    // start 立即返回；完成判定统一走 onSettled 的终态回调。
    for (const name of missingModels) {
      void start("ollama", name);
    }
  }

  async function copyOllamaLink() {
    try {
      await navigator.clipboard.writeText("https://ollama.com");
      setCopied(true);
    } catch {
      setError("复制链接失败，请手动访问 https://ollama.com");
    }
  }

  return (
    <>
      <List>
        <ListRow
          static
          state={install?.installed ? "done" : "idle"}
          title="Ollama 已安装"
          meta={install?.installed ? "命令行可用" : "未检测到 ollama 命令"}
        />
        <ListRow
          static
          state={install?.running ? "done" : "idle"}
          title="Ollama 服务运行中"
          meta={install?.running ? "本机服务已就绪" : "未检测到本机服务"}
        />
        {local && (
          <>
            <ModelStatusRow
              title={`分析模型 ${local.settings.analysisModel}`}
              installed={analysisInstalled}
              hint={modelSizeHint(local.settings.analysisModel, "qwen2.5:7b", "约 4.7GB · 中文分析更稳")}
              download={downloads[local.settings.analysisModel]}
            />
            <ModelStatusRow
              title={`嵌入模型 ${local.settings.embeddingModel}`}
              installed={embeddingInstalled}
              hint={modelSizeHint(local.settings.embeddingModel, "qwen3-embedding:0.6b", "约 639MB · 知识索引（跨记录问答 / 记忆生成）必需")}
              download={downloads[local.settings.embeddingModel]}
            />
          </>
        )}
      </List>
      {!install?.installed && (
        <p className="em-note">
          这台 Mac 还没有安装 Ollama。到 ollama.com 下载安装并启动，这里会自动检测到；
          也可以稍后在 设置 → AI 模型 里改用云端分析。
          <Button variant="dashed" onClick={() => void copyOllamaLink()}>
            {copied ? "链接已复制" : "复制 ollama.com 链接"}
          </Button>
        </p>
      )}
      {install?.installed && !install.running && (
        <p className="em-note">Ollama 已安装但服务没有运行。启动 Ollama 应用后这里会自动检测到。</p>
      )}
      <div className="em-card-foot">
        {missingModels.length > 0 ? (
          serviceReady ? (
            <Button variant="primary" disabled={preparing} onClick={() => void prepareAll()}>
              {preparing ? "正在准备…" : "一键准备（拉取缺失模型）"}
            </Button>
          ) : (
            <Button variant="primary" disabled onClick={() => undefined}>
              等服务就绪后可一键准备
            </Button>
          )
        ) : (
          <Button variant="primary" onClick={onResume}>
            已就绪，继续
          </Button>
        )}
      </div>
      {error && (() => {
        const localized = localizePullError(error);
        return (
          <div className="em-note err" role="alert">
            {localized.friendly}
            {localized.detail !== localized.friendly && (
              <Collapsible title="原始错误信息">{localized.detail}</Collapsible>
            )}
          </div>
        );
      })()}
    </>
  );
}

/** 体积提示只写给写死在推荐位上的默认模型；换过的模型体积只有拉取时才准，不写固定数。 */
function modelSizeHint(modelName: string, defaultModel: string, defaultHint: string): string {
  return modelName === defaultModel ? defaultHint : "体积以拉取时为准";
}

function ModelStatusRow({
  title,
  installed,
  hint,
  download,
}: {
  title: string;
  installed: boolean;
  hint: string;
  download?: DownloadState;
}) {
  const running = download?.phase === "running";
  const meta = running
    ? download?.total
      ? `${download.label} · ${Math.round(((download.completed ?? 0) / download.total) * 100)}%`
      : (download?.label ?? "正在准备")
    : installed
      ? "已拉取"
      : hint;
  return (
    <>
      <ListRow static state={installed ? "done" : running ? "working" : "idle"} title={title} meta={meta} />
      {running && (
        <Progress completed={download?.completed ?? null} total={download?.total ?? null} label={download?.label ?? "正在下载"} />
      )}
    </>
  );
}

/* ------------------------------ 云端：填 Key 即用 ------------------------------ */

function CloudKeyTab({ onResume }: { onResume: () => void }) {
  const [preset, setPreset] = useState<CloudPresetId>("zhipu");
  const [baseUrl, setBaseUrl] = useState<string>(CLOUD_PRESETS[0].baseUrl);
  const [model, setModel] = useState<string>(CLOUD_PRESETS[0].model);
  const [apiKey, setApiKey] = useState("");
  const [consent, setConsent] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [savedButUnreachable, setSavedButUnreachable] = useState(false);

  function applyPreset(next: CloudPresetId) {
    setPreset(next);
    const found = CLOUD_PRESETS.find((item) => item.id === next);
    if (found) {
      setBaseUrl(found.baseUrl);
      setModel(found.model);
    }
  }

  async function saveCloud() {
    if (!apiKey.trim()) {
      setError("请先填写 API Key。");
      return;
    }
    if (!consent) {
      setError("请先勾选文本发送说明，云端通道才能启用。");
      return;
    }
    setBusy(true);
    setError("");
    setSavedButUnreachable(false);
    try {
      await setExternalAiApiKey(apiKey.trim());
      const current = await getExternalAiSettings();
      await updateExternalAiSettings({
        ...current,
        enabled: true,
        baseUrl: baseUrl.trim(),
        model: model.trim(),
        privacyConsentAt: new Date().toISOString(),
      });
      await testExternalAiConnection();
      onResume();
    } catch (reason) {
      // Key 与配置都已落库；检测失败时如实区分，让用户能针对性修正。
      setSavedButUnreachable(true);
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  const canSave = apiKey.trim().length > 0 && consent && baseUrl.trim().length > 0 && model.trim().length > 0;

  return (
    <>
      <p className="em-note">
        云端只覆盖分析（理解整理）；知识索引（向量嵌入）仍需本机 Ollama，
        只配云端时跨记录问答/记忆生成仍会提示缺嵌入模型。
      </p>
      <Segmented
        label="服务商"
        value={preset}
        onChange={applyPreset}
        options={[...CLOUD_PRESETS.map((item) => ({ value: item.id, label: item.label })), { value: "custom" as const, label: "自定义" }]}
      />
      <Field label="接口地址" hint="OpenAI 兼容的 /v1 地址。只有本机地址允许用 http。">
        <input
          className="em-input"
          value={baseUrl}
          placeholder="https://api.example.com/v1"
          onChange={(event) => {
            setBaseUrl(event.target.value);
            setPreset("custom");
          }}
        />
      </Field>
      <Field label="模型名称" hint="服务商文档里的模型 ID，可按需更换。">
        <input
          className="em-input"
          value={model}
          placeholder="glm-4-flash"
          onChange={(event) => {
            setModel(event.target.value);
            setPreset("custom");
          }}
        />
      </Field>
      <Field label="API Key" hint="保存在 macOS 钥匙串里，不会回显。">
        <input
          className="em-input"
          type="password"
          autoComplete="new-password"
          value={apiKey}
          placeholder="仅用于保存，不会回显"
          onChange={(event) => setApiKey(event.target.value)}
        />
      </Field>
      <label className="em-check-row">
        <input
          type="checkbox"
          checked={consent}
          onChange={(event) => setConsent(event.target.checked)}
        />
        <span>我了解逐字稿全文与分析内容会发送给所填服务商。</span>
      </label>
      <div className="em-card-foot">
        <Button variant="primary" disabled={!canSave || busy} onClick={() => void saveCloud()}>
          {busy ? "正在检测连通…" : "保存并检测连通"}
        </Button>
        <span className="em-hint">未填 Key 或未勾选说明时无法保存。</span>
      </div>
      {error && (
        <p className="em-note err" role="alert">
          {savedButUnreachable ? "Key 与配置已保存，但连通检测失败：" : ""}
          {error}
          {savedButUnreachable ? "。请检查接口地址、模型名称与 Key 后重试。" : ""}
        </p>
      )}
    </>
  );
}
