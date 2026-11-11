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
  clearExternalCredentials,
  diagnoseExternalProvider,
  getCredentialRevision,
  getExternalAiSettings,
  getLocalAiStatus,
  getOllamaInstallStatus,
  getProviderCatalog,
  getProviderHealthStatus,
  listExternalModels,
  revealExternalCredentials,
  reuseExternalCredentials,
  saveAndVerifyProvider,
  saveExternalCredentials,
  updateExternalAiSettings,
  verifySavedProvider,
} from "../lib/tauri";
import type {
  DiagnosisReport,
  ExternalAiSettings,
  LocalAiStatus,
  OllamaInstallStatus,
  ProviderCatalog,
  ProviderHealthSnapshot,
  SaveAndVerifyOutcome,
} from "../shared/types";
import { Button, Collapsible, List, ListRow, Progress, Segmented, Stack } from "./SettingsKit";
import ProviderConnectionForm, {
  type ProviderCommit,
  type ProviderDiagnoseDraft,
  type ProviderSaveAndVerifyDraft,
} from "./ProviderConnectionForm";
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
    // 只统计本 tab 发起的缺失模型：后台其它下载（如 whisper）的终态不算数。
    if (!missingRef.current.includes(settled.model)) return;
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

/* ------------------------------ 云端：统一服务商表单 ------------------------------ */

function CloudKeyTab({ onResume }: { onResume: () => void }) {
  const [catalog, setCatalog] = useState<ProviderCatalog | null>(null);
  const [settings, setSettings] = useState<ExternalAiSettings | null>(null);
  const [busy, setBusy] = useState(false);
  const [loadError, setLoadError] = useState("");
  const [health, setHealth] = useState<ProviderHealthSnapshot | null>(null);
  const [revision, setRevision] = useState(0);

  useEffect(() => {
    let active = true;
    void Promise.all([
      getProviderCatalog(),
      getExternalAiSettings(),
      getProviderHealthStatus("analysis"),
      getCredentialRevision("analysis"),
    ])
      .then(([nextCatalog, nextSettings, nextHealth, nextRevision]) => {
        if (!active) return;
        setCatalog(nextCatalog);
        setSettings(nextSettings);
        setHealth(nextHealth);
        setRevision(nextRevision);
      })
      .catch((reason) => {
        if (active) setLoadError(String(reason));
      });
    return () => {
      active = false;
    };
  }, []);

  async function refreshVerificationState() {
    try {
      const [nextHealth, nextRevision] = await Promise.all([
        getProviderHealthStatus("analysis"),
        getCredentialRevision("analysis"),
      ]);
      setHealth(nextHealth);
      setRevision(nextRevision);
    } catch {
      // 状态读取失败不阻塞：状态条按「未验证」处理。
    }
  }

  function normalizeProvider(
    draft: Omit<ProviderCommit, "credentials" | "consentAt">,
  ) {
    if (!catalog) throw new Error("服务商目录尚未加载");
    const provider = catalog.providers.find((item) => item.id === draft.providerId);
    if (!provider?.analysis) throw new Error("该服务商不支持文本分析");
    return {
      providerId: provider.id,
      baseUrl:
        provider.id === "custom" ? draft.baseUrl.trim() : provider.analysis.baseUrl,
      model: draft.model.trim() || provider.analysis.recommendedModel,
    };
  }

  function analysisPatch(
    provider: { providerId: string; baseUrl: string; model: string },
    consentAt: string | null,
    enabled: boolean,
  ): Partial<ExternalAiSettings> {
    return {
      analysisProvider: provider.providerId,
      baseUrl: provider.baseUrl,
      model: provider.model,
      privacyConsentAt: consentAt,
      enabled,
    };
  }

  async function save(
    current: ExternalAiSettings,
    patch: Partial<ExternalAiSettings>,
  ): Promise<ExternalAiSettings> {
    const saved = await updateExternalAiSettings({ ...current, ...patch });
    setSettings(saved);
    return saved;
  }

  /** 仅保存（跳过验证）：状态条会落在「已配置，未验证」。 */
  async function commit(draft: ProviderCommit) {
    if (!settings) throw new Error("分析设置尚未加载完成");
    setBusy(true);
    setLoadError("");
    try {
      const provider = normalizeProvider(draft);
      let current = settings;
      const changed =
        current.analysisProvider !== provider.providerId ||
        current.baseUrl !== provider.baseUrl ||
        current.model !== provider.model;
      if (changed) {
        current = await save(current, analysisPatch(provider, null, false));
      }
      current = await save(current, analysisPatch(provider, draft.consentAt, false));
      if (draft.credentials) {
        current = await saveExternalCredentials(
          "analysis",
          provider.providerId,
          draft.credentials,
        );
        setSettings(current);
      }
      current = await save(current, { enabled: true });
      await refreshVerificationState();
    } catch (reason) {
      setLoadError(String(reason));
      throw reason;
    } finally {
      setBusy(false);
    }
  }

  /** 主流程：先诊断草稿（不落盘），通过后凭票据提交，成功才续跑原动作。 */
  async function diagnose(draft: ProviderDiagnoseDraft): Promise<DiagnosisReport> {
    return diagnoseExternalProvider({
      target: "analysis",
      providerId: draft.providerId,
      baseUrl: draft.baseUrl,
      model: draft.model,
      credentials: draft.credentials,
    });
  }

  async function saveAndVerify(
    draft: ProviderSaveAndVerifyDraft,
  ): Promise<SaveAndVerifyOutcome> {
    if (!settings) throw new Error("分析设置尚未加载完成");
    setBusy(true);
    setLoadError("");
    try {
      const outcome = await saveAndVerifyProvider({
        target: "analysis",
        providerId: draft.providerId,
        baseUrl: draft.baseUrl,
        model: draft.model,
        credentials: draft.credentials,
        privacyConsentAt: draft.consentAt,
        audioUploadConsentAt: null,
        enabled: true,
        verificationToken: draft.verificationToken,
      });
      setSettings(outcome.settings);
      await refreshVerificationState();
      onResume();
      return outcome;
    } catch (reason) {
      setLoadError(String(reason));
      throw reason;
    } finally {
      setBusy(false);
    }
  }

  async function verifySaved(): Promise<DiagnosisReport> {
    setBusy(true);
    setLoadError("");
    try {
      const report = await verifySavedProvider("analysis");
      await refreshVerificationState();
      return report;
    } catch (reason) {
      setLoadError(String(reason));
      throw reason;
    } finally {
      setBusy(false);
    }
  }

  async function clearCredentials() {
    const saved = await clearExternalCredentials("analysis");
    setSettings(saved);
  }

  async function reuseCredentials(
    draft: Omit<ProviderCommit, "credentials" | "consentAt">,
  ) {
    if (!settings) throw new Error("分析设置尚未加载完成");
    const provider = normalizeProvider(draft);
    let current = settings;
    if (current.analysisProvider !== provider.providerId) {
      current = await save(current, analysisPatch(provider, null, false));
    }
    const saved = await reuseExternalCredentials("analysis");
    setSettings(saved);
  }

  if (loadError && (!catalog || !settings)) {
    return <p className="em-note err" role="alert">{loadError}</p>;
  }
  if (!catalog || !settings) {
    return <p className="em-empty">正在读取服务商目录…</p>;
  }

  return (
    <>
      <p className="em-note">
        云端只覆盖分析（理解整理）；知识索引（向量嵌入）仍需本机 Ollama，
        只配云端时跨记录问答/记忆生成仍会提示缺嵌入模型。
      </p>
      <ProviderConnectionForm
        target="analysis"
        catalog={catalog}
        settings={settings}
        busy={busy}
        actionLabel="保存并验证"
        health={health}
        credentialRevision={revision}
        catalogVersion={catalog.version}
        onCommit={commit}
        onDiagnose={diagnose}
        onSaveAndVerify={saveAndVerify}
        onVerifySaved={verifySaved}
        onClearCredentials={clearCredentials}
        onReuseCredentials={reuseCredentials}
        onRevealCredentials={(providerId) =>
          revealExternalCredentials("analysis", providerId)
        }
        onListModels={(input) =>
          listExternalModels(
            "analysis",
            input.providerId,
            input.baseUrl,
            input.apiKey,
          )
        }
      />
    </>
  );
}
