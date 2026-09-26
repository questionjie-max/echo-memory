import { open } from "@tauri-apps/plugin-dialog";
import { useEffect, useRef, useState, type ReactNode } from "react";
import { SparklesIcon, CheckIcon, XIcon } from "./icons";
import { open as openDirectory } from "@tauri-apps/plugin-dialog";
import AiModelSettings from "./AiModelSettings";
import TemplateWizard from "./TemplateWizard";
import {
  Button,
  Card,
  Field,
  List,
  ListRow,
  PanelShell,
  Pill,
  SavedToast,
  Segmented,
  Stack,
  StatusPill,
  ToggleRow,
  useSavedFlash,
  type NavItem,
} from "./SettingsKit";
import { getStoredTheme, setTheme, type Theme } from "../lib/theme";
import { useModelDownloads } from "./useModelDownloads";
import type {
  AnalysisTemplate,
  AppInfo,
  AudioPreprocessorStatus,
  ExternalAiSettings,
  Hotword,
  InboxStatus,
  KnowledgeSettings,
  LocalAiStatus,
  OutputStatus,
  TemplateSection,
  TranscriptionEngineStatus,
} from "../shared/types";
import {
  addHotword,
  addInboxWatchFolder,
  clearExternalAiApiKey,
  clearExternalAsrApiKey,
  clearHfToken,
  createAnalysisTemplate,
  deleteAnalysisTemplate,
  getAppInfo,
  getAudioPreprocessorStatus,
  getExternalAiSettings,
  getInboxStatus,
  getLocalAiStatus,
  getOutputStatus,
  getTranscriptCorrectionEnabled,
  getTranscriptionEngineStatus,
  listAnalysisTemplates,
  listHotwords,
  removeHotword,
  removeInboxWatchFolder,
  resetOnboarding,
  rescanInbox,
  setAutoExportAnalysis,
  setExternalAiApiKey,
  setExternalAsrApiKey,
  setHfToken,
  setInboxUsbDetection,
  setOutputFolder,
  setTranscriptCorrectionEnabled,
  setTranscriptionEngine,
  testExternalAiConnection,
  updateAnalysisTemplate,
  updateExternalAiSettings,
  updateKnowledgeSettings,
} from "../lib/tauri";

interface Props {
  open: boolean;
  onClose: () => void;
  /** 「去重建索引」需要离开设置面板回到主界面，由外层决定去哪。 */
  onOpenKnowledge?: () => void;
}

type SettingsTab = "models" | "templates" | "inbox" | "hotwords" | "output" | "about";

/**
 * 六个分区按「用户带着什么问题来」划分：
 * AI 模型（谁来转写和分析）/ 分析模板（怎么分析）/ 收件箱（音频从哪来）/
 * 词汇库（怎么认对专有名词）/ 产出（结果写到哪）/ 关于（版本与数据）。
 */
const NAV: NavItem[] = [
  { id: "models", label: "AI 模型" },
  { id: "templates", label: "分析模板" },
  { id: "inbox", label: "收件箱" },
  { id: "hotwords", label: "词汇库" },
  { id: "output", label: "产出" },
  { id: "about", label: "关于" },
];

const TAB_TITLES: Record<SettingsTab, { title: string; subtitle: string }> = {
  models: { title: "AI 模型", subtitle: "谁来把录音转成文字，谁来把文字变成结论。改动即时生效。" },
  templates: { title: "分析模板", subtitle: "决定 AI 从逐字稿里抽出什么。" },
  inbox: { title: "收件箱", subtitle: "监听文件夹，新音频一出现就自动导入并转写。" },
  hotwords: { title: "词汇库", subtitle: "专有名词、人名、产品名，避免被写成同音字。" },
  output: { title: "产出", subtitle: "分析、逐字稿和成稿以 Markdown 写到你的文件夹。" },
  about: { title: "关于", subtitle: "版本、数据目录和首次启动引导。" },
};

export default function SettingsPanel({ open: visible, onClose, onOpenKnowledge }: Props) {
  const [tab, setTab] = useState<SettingsTab>("models");
  const [status, setStatus] = useState<LocalAiStatus | null>(null);
  const [external, setExternal] = useState<ExternalAiSettings | null>(null);
  const [preprocessor, setPreprocessor] = useState<AudioPreprocessorStatus | null>(null);
  const [engineStatus, setEngineStatus] = useState<TranscriptionEngineStatus | null>(null);
  const [templates, setTemplates] = useState<AnalysisTemplate[]>([]);
  const [inbox, setInbox] = useState<InboxStatus | null>(null);
  const [hotwords, setHotwords] = useState<Hotword[]>([]);
  const [hotwordInput, setHotwordInput] = useState("");
  const [correctionEnabled, setCorrectionEnabled] = useState(false);
  const [output, setOutput] = useState<OutputStatus | null>(null);
  const [appInfo, setAppInfo] = useState<AppInfo | null>(null);
  const [editing, setEditing] = useState<AnalysisTemplate | null>(null);
  const [wizardOpen, setWizardOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [tabError, setTabError] = useState<string | null>(null);
  const [testResult, setTestResult] = useState<ReactNode>(null);
  const { flash, show } = useSavedFlash();
  const { downloads, start, cancel } = useModelDownloads((settled) => {
    if (settled.phase === "done") void refresh();
  });
  // 两次写入可能落在同一个 tick 里（例如先失焦再点开关）。用 ref 记录最新值，
  // 每次都基于最新状态合并，否则后一次会用渲染闭包里的旧数据把前一次覆盖掉。
  const settingsRef = useRef<KnowledgeSettings | null>(null);
  const externalRef = useRef<ExternalAiSettings | null>(null);

  function applyStatus(next: LocalAiStatus | null) {
    settingsRef.current = next?.settings ?? null;
    setStatus(next);
  }

  function applyExternal(next: ExternalAiSettings | null) {
    externalRef.current = next;
    setExternal(next);
  }

  async function refresh() {
    setError("");
    try {
      const [ai, audio, items, externalSettings, transcriptionEngine] = await Promise.all([
        getLocalAiStatus(),
        getAudioPreprocessorStatus(),
        listAnalysisTemplates(),
        getExternalAiSettings(),
        getTranscriptionEngineStatus(),
      ]);
      applyStatus(ai);
      setPreprocessor(audio);
      setTemplates(items);
      applyExternal(externalSettings);
      setEngineStatus(transcriptionEngine);
    } catch (reason) {
      setError(String(reason));
    }
  }

  useEffect(() => {
    if (!visible) return;
    if (!appInfo) void getAppInfo().then(setAppInfo).catch(() => setAppInfo(null));
    // 打开设置或切换 tab 都要加载数据：核心状态随面板刷新，
    // 收件箱/词汇库/产出按 tab 按需加载（此前依赖只看 visible，切 tab 永远不加载）。
    void refresh();
    if (tab === "output") {
      void getOutputStatus().then(setOutput).catch(() => setOutput(null));
    }
    if (tab === "inbox" || tab === "hotwords") {
      void loadInbox();
      void loadHotwords();
      void getOutputStatus().then(setOutput).catch(() => setOutput(null));
      void getTranscriptCorrectionEnabled().then(setCorrectionEnabled).catch(() => setCorrectionEnabled(false));
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [visible, tab]);

  async function loadInbox() {
    try {
      setInbox(await getInboxStatus());
      setTabError(null);
    } catch (reason) {
      setTabError(String(reason));
    }
  }

  async function loadHotwords() {
    try {
      setHotwords(await listHotwords());
    } catch (reason) {
      setTabError(String(reason));
    }
  }

  /** 改动即时生效：本地先乐观更新保证点下去有反馈，写库失败再退回并说明原因。 */
  async function report<T>(action: () => Promise<T>, success: string, apply?: (value: T) => void) {
    setError("");
    try {
      const value = await action();
      apply?.(value);
      show("saved", success);
    } catch (reason) {
      show("failed", String(reason));
      setError(String(reason));
      await refresh();
    }
  }

  async function saveSettings(patch: Partial<KnowledgeSettings>) {
    const previous = settingsRef.current;
    if (!previous) return;
    const next = { ...previous, ...patch };
    settingsRef.current = next;
    setStatus((current) => (current ? { ...current, settings: next } : current));
    await report(
      () => updateKnowledgeSettings(next),
      "已保存",
      (saved) => {
        settingsRef.current = saved;
        setStatus((current) => (current ? { ...current, settings: saved } : current));
      },
    );
    await refresh();
  }

  async function switchEngine(next: "embedded" | "whisperx") {
    await report(
      () => setTranscriptionEngine(next).then(getTranscriptionEngineStatus),
      "已切换引擎",
      setEngineStatus,
    );
  }

  async function saveHfToken(token: string) {
    await report(
      () => setHfToken(token).then(getTranscriptionEngineStatus),
      "Token 已保存到钥匙串",
      setEngineStatus,
    );
  }

  async function removeHfToken() {
    await report(
      () => clearHfToken().then(getTranscriptionEngineStatus),
      "Token 已清除",
      setEngineStatus,
    );
  }

  async function chooseWhisperModel() {
    const selected = await open({
      multiple: false,
      directory: false,
      filters: [{ name: "Whisper 模型", extensions: ["bin"] }],
    });
    if (typeof selected === "string") await saveSettings({ whisperModelPath: selected });
  }

  async function saveExternal(patch: Partial<ExternalAiSettings>) {
    const previous = externalRef.current;
    if (!previous) return;
    const next = { ...previous, ...patch };
    externalRef.current = next;
    setExternal(next);
    try {
      const saved = await updateExternalAiSettings(next);
      externalRef.current = saved;
      setExternal(saved);
      show("saved", "已保存");
      setError("");
    } catch (reason) {
      // 校验失败（例如没勾隐私说明就启用）时要退回，不能留下一个假的已启用状态。
      externalRef.current = previous;
      setExternal(previous);
      show("failed", String(reason));
      setError(String(reason));
    }
  }

  async function saveExternalKey(key: string) {
    await report(() => setExternalAiApiKey(key), "API Key 已保存到钥匙串", applyExternal);
  }

  async function removeExternalKey() {
    if (!window.confirm("清除外部 AI API Key？外部 AI 将无法生成记忆快照。")) return;
    await report(() => clearExternalAiApiKey(), "API Key 已清除", applyExternal);
  }

  async function saveExternalAsrKey(key: string) {
    await report(() => setExternalAsrApiKey(key), "语音转文字 API Key 已保存到钥匙串", applyExternal);
  }

  async function removeExternalAsrKey() {
    if (!window.confirm("清除语音转文字 API Key？第三方录音转写将无法执行。")) return;
    await report(() => clearExternalAsrApiKey(), "语音转文字 API Key 已清除", applyExternal);
  }

  async function checkExternal() {
    setBusy(true);
    setError("");
    try {
      const startedAt = Date.now();
      await testExternalAiConnection();
      setTestResult(<><CheckIcon size={12} /> {Date.now() - startedAt}ms</>);
      await refresh();
    } catch (reason) {
      setTestResult(<><XIcon size={12} /> 失败</>);
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  async function submitHotword() {
    const term = hotwordInput.trim();
    if (!term) return;
    try {
      await addHotword(term);
      setHotwordInput("");
      await loadHotwords();
      show("saved", "已添加");
      setError("");
    } catch (reason) {
      setError(String(reason));
    }
  }

  async function chooseWatchFolder() {
    try {
      const selected = await openDirectory({ directory: true, multiple: false });
      if (typeof selected !== "string") return;
      await addInboxWatchFolder(selected);
      await loadInbox();
      show("saved", "已添加");
      setError("");
    } catch (reason) {
      setError(String(reason));
    }
  }

  async function chooseOutputFolder() {
    try {
      const selected = await openDirectory({ directory: true, multiple: false });
      if (typeof selected !== "string") return;
      setOutput(await setOutputFolder(selected));
      show("saved", "已保存");
      setError("");
    } catch (reason) {
      setError(String(reason));
    }
  }

  async function rerunOnboarding() {
    try {
      await resetOnboarding();
      onClose();
    } catch (reason) {
      setError(String(reason));
    }
  }

  if (!visible) return null;

  const header = TAB_TITLES[tab];

  return (
    <div
      className="settings-scrim"
      role="presentation"
      onMouseDown={(event) => event.target === event.currentTarget && onClose()}
    >
      <PanelShell
        brand="回声记忆"
        version={appInfo ? `v${appInfo.version}` : undefined}
        nav={NAV}
        active={tab}
        onSelect={(id) => setTab(id as SettingsTab)}
        title={header.title}
        subtitle={header.subtitle}
        onClose={onClose}
      >
        <SavedToast flash={flash} />

        {tab === "models" ? (
          status ? (
            <AiModelSettings
              status={status}
              preprocessor={preprocessor}
              engine={engineStatus}
              external={external}
              downloads={downloads}
              busy={busy}
              testResult={testResult}
              error={error}
              onSaveSettings={(patch) => void saveSettings(patch)}
              onStartDownload={(kind, model) => void start(kind, model)}
              onCancelDownload={(model) => void cancel(model)}
              onSwitchEngine={(next) => void switchEngine(next)}
              onSaveHfToken={(token) => void saveHfToken(token)}
              onClearHfToken={() => void removeHfToken()}
              onChooseWhisperFile={() => void chooseWhisperModel()}
              onSaveExternal={(patch) => void saveExternal(patch)}
              onSaveExternalKey={(key) => void saveExternalKey(key)}
              onClearExternalKey={() => void removeExternalKey()}
              onSaveExternalAsrKey={(key) => void saveExternalAsrKey(key)}
              onClearExternalAsrKey={() => void removeExternalAsrKey()}
              onTestExternal={() => void checkExternal()}
              onRefresh={() => void refresh()}
              onOpenKnowledge={() => {
                onClose();
                onOpenKnowledge?.();
              }}
            />
          ) : (
            <p className="em-empty">正在读取本机状态…</p>
          )
        ) : tab === "templates" ? (
          editing ? (
            <TemplateEditor
              template={editing}
              busy={busy}
              onCancel={() => setEditing(null)}
              onSaved={async () => {
                setEditing(null);
                await refresh();
                show("saved", "模板已保存");
              }}
              onError={setError}
            />
          ) : (
            <Card
              title={`${templates.length} 个模板`}
              description="分析时选一个模板，AI 就按它的栏目抽取内容。"
              actions={
                <>
                  <Button onClick={() => setWizardOpen(true)}><SparklesIcon size={14} /> AI 生成模板</Button>
                  <Button variant="primary" onClick={() => setEditing(emptyTemplate())}>
                    新建模板
                  </Button>
                </>
              }
            >
              <List>
                {templates.map((template) => (
                  <ListRow
                    key={template.id}
                    state="done"
                    title={template.name}
                    badge={template.isBuiltin ? <Pill>内置</Pill> : undefined}
                    meta={template.description || "自定义分析模板"}
                    trail={
                      <>
                        <Button variant="quiet" disabled={busy} onClick={() => void duplicateTemplate(template)}>
                          复制
                        </Button>
                        {!template.isBuiltin && (
                          <Button variant="quiet" disabled={busy} onClick={() => setEditing(template)}>
                            编辑
                          </Button>
                        )}
                        {!template.isBuiltin && (
                          <Button
                            variant="danger"
                            disabled={busy}
                            ariaLabel={`删除模板 ${template.name}`}
                            onClick={() => void removeTemplate(template)}
                          >
                            删除
                          </Button>
                        )}
                      </>
                    }
                  />
                ))}
              </List>
            </Card>
          )
        ) : tab === "inbox" ? (
          inbox ? (
            <Stack>
              <Card
                title="监听文件夹"
                description="新音频一出现就自动导入并转写。添加监听时的已有文件会被跳过，只接住之后新出现的文件。"
                status={<StatusPill ok={inbox.watchFolders.length > 0 || inbox.usbDetection} okText="已开启" badText="未开启" />}
                actions={
                  <>
                    <Button onClick={() => void chooseWatchFolder()}>添加文件夹…</Button>
                    <Button onClick={() => void rescanInbox().then(loadInbox)}>立即扫描</Button>
                  </>
                }
                footer={
                  <ToggleRow
                    label="插入 USB 录音设备时自动扫描（设备上的录音属于你要导入的内容）"
                    checked={inbox.usbDetection}
                    onChange={(enabled) => {
                      setInbox({ ...inbox, usbDetection: enabled });
                      void setInboxUsbDetection(enabled).then(loadInbox).catch((reason) => setError(String(reason)));
                    }}
                  />
                }
              >
                {inbox.watchFolders.length === 0 ? (
                  <p className="em-empty">还没有监听任何文件夹。点上面的「添加文件夹…」开始。</p>
                ) : (
                  <List>
                    {inbox.watchFolders.map((folder) => (
                      <ListRow
                        key={folder.id}
                        state="done"
                        title={folder.label}
                        meta={folder.path}
                        trail={
                          <Button
                            variant="danger"
                            ariaLabel={`移除监听文件夹 ${folder.label}`}
                            onClick={() => void removeInboxWatchFolder(folder.id).then(loadInbox)}
                          >
                            移除
                          </Button>
                        }
                      />
                    ))}
                  </List>
                )}
              </Card>
              <Card
                title="最近自动导入"
                description={`待处理 ${inbox.counts.pending} · 已导入 ${inbox.counts.imported} · 失败 ${inbox.counts.failed}`}
              >
                {inbox.recentFiles.length === 0 ? (
                  <p className="em-empty">还没有文件进入收件箱。</p>
                ) : (
                  <List>
                    {inbox.recentFiles.map((file) => (
                      <ListRow
                        key={file.id}
                        state={file.status === "failed" ? "error" : file.status === "imported" ? "done" : "working"}
                        title={file.fileName}
                        meta={file.errorMessage ?? inboxStatusLabel(file.status)}
                      />
                    ))}
                  </List>
                )}
              </Card>
            </Stack>
          ) : (
            <p className="em-empty">正在读取收件箱状态…</p>
          )
        ) : tab === "hotwords" ? (
          <Stack>
            <Card
              title="个人词汇库"
              description="热词会注入转写提示与 AI 校对，明显改善专有名词、人名、产品名的中文识别。"
              status={<Pill>{hotwords.length} 个</Pill>}
            >
              <div className="em-field-row">
                <input
                  className="em-input"
                  value={hotwordInput}
                  placeholder="输入热词后回车"
                  onChange={(event) => setHotwordInput(event.target.value)}
                  onKeyDown={(event) => {
                    if (event.key === "Enter" && !event.nativeEvent.isComposing) void submitHotword();
                  }}
                />
                <Button variant="primary" disabled={!hotwordInput.trim()} onClick={() => void submitHotword()}>
                  添加
                </Button>
              </div>
              {hotwords.length === 0 ? (
                <p className="em-empty">还没有热词。</p>
              ) : (
                <List>
                  {hotwords.map((hotword) => (
                    <ListRow
                      key={hotword.id}
                      state="done"
                      title={hotword.term}
                      meta={hotword.note ?? undefined}
                      trail={
                        <Button
                          variant="danger"
                          ariaLabel={`删除热词 ${hotword.term}`}
                          onClick={() => void removeHotword(hotword.id).then(loadHotwords)}
                        >
                          删除
                        </Button>
                      }
                    />
                  ))}
                </List>
              )}
            </Card>
            <Card title="转写 AI 校对" description="校对结果写入独立文本层，原始逐字稿永不覆盖，可随时对比。">
              <ToggleRow
                label="转写完成后自动用本机模型校对（断句、标点、热词纠正）"
                checked={correctionEnabled}
                onChange={(enabled) => {
                  setCorrectionEnabled(enabled);
                  void setTranscriptCorrectionEnabled(enabled).catch((reason) => setError(String(reason)));
                }}
              />
            </Card>
          </Stack>
        ) : tab === "output" ? (
          output ? (
            <Stack>
              <Card
                title="产出文件夹"
                description="所有生成内容以标准 Markdown + YAML frontmatter 写入这里。可以指向 Obsidian 等笔记工具的目录——回声记忆只负责写出，不感知任何外部工具。"
                status={<StatusPill ok={Boolean(output.folder)} okText="自定义" badText="默认位置" />}
              >
                <Field label="当前目录">
                  <div className="em-field-row">
                    <input
                      className="em-input"
                      readOnly
                      value={output.folder ?? "~/Documents/回声记忆产出"}
                    />
                    <div className="em-card-actions">
                      <Button onClick={() => void chooseOutputFolder()}>选择</Button>
                      {output.folder && (
                        <Button
                          onClick={() =>
                            void setOutputFolder(null).then(setOutput).catch((reason) => setError(String(reason)))
                          }
                        >
                          恢复默认
                        </Button>
                      )}
                    </div>
                  </div>
                </Field>
              </Card>
              <Card title="自动导出" description="默认关闭。开启后每条录音分析完成即落盘一个 md 文件；也可随时在记录详情里手动导出。">
                <ToggleRow
                  label="分析完成后自动导出 Markdown"
                  checked={output.autoExportAnalysis}
                  onChange={(enabled) =>
                    void setAutoExportAnalysis(enabled).then(setOutput).catch((reason) => setError(String(reason)))
                  }
                />
              </Card>
              <Card title="最近产出">
                {output.recentFiles.length === 0 ? (
                  <p className="em-empty">还没有产出文件。</p>
                ) : (
                  <List>
                    {output.recentFiles.map((file) => (
                      <ListRow
                        key={file.path}
                        state="done"
                        title={file.fileName}
                        meta={file.path}
                        trail={`${(file.size / 1024).toFixed(1)}KB`}
                      />
                    ))}
                  </List>
                )}
              </Card>
            </Stack>
          ) : (
            <p className="em-empty">正在读取产出设置…</p>
          )
        ) : (
          <Stack>
            <ThemeCard />
            <Card title="版本" status={<Pill tone="accent">{appInfo ? `v${appInfo.version}` : "…"}</Pill>}>
              <List>
                <ListRow static state="done" title="回声记忆（Echo Memory）" meta="本地优先的个人智能记忆系统" />
                <ListRow static title="许可" meta="Apache-2.0 开源 · 内置 Whisper（MIT）与 ffmpeg（LGPL）" />
                <ListRow static title="源码与反馈" meta="github.com/questionjie-max/echo-memory" />
              </List>
            </Card>
            <Card
              title="数据目录"
              description="备份或迁移时复制整个目录即可；删除该目录等于清空全部本地数据。"
            >
              <div className="em-field-row">
                <input className="em-input" readOnly value={appInfo?.libraryPath ?? "…"} />
                <Button
                  onClick={() => {
                    if (!appInfo) return;
                    void navigator.clipboard
                      ?.writeText(appInfo.libraryPath)
                      .then(() => show("saved", "目录已复制"))
                      .catch((reason) => setError(String(reason)));
                  }}
                >
                  复制
                </Button>
              </div>
            </Card>
            <Card title="首次启动引导" description="重新走一遍模型与收件箱配置流程。关闭设置后会自动弹出。">
              <Button onClick={() => void rerunOnboarding()}>重新运行引导向导</Button>
            </Card>
          </Stack>
        )}

        {tabError && <p className="em-note err" role="alert">{tabError}</p>}
      </PanelShell>

      {wizardOpen && (
        <TemplateWizard
          onClose={() => setWizardOpen(false)}
          onCreated={async () => {
            setWizardOpen(false);
            await refresh();
          }}
        />
      )}
    </div>
  );

  async function duplicateTemplate(template: AnalysisTemplate) {
    setBusy(true);
    try {
      const created = await createAnalysisTemplate({
        name: `${template.name} 副本`,
        description: template.description,
        focusInstructions: template.focusInstructions,
        customSections: template.customSections,
      });
      await refresh();
      setEditing(created);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  async function removeTemplate(template: AnalysisTemplate) {
    if (!window.confirm(`删除模板“${template.name}”？历史分析不会被删除。`)) return;
    setBusy(true);
    try {
      await deleteAnalysisTemplate(template.id);
      if (editing?.id === template.id) setEditing(null);
      await refresh();
      show("saved", "模板已删除");
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }
}

/** 外观设置：深色（Lovart 方案，默认）/ 浅色，改动即时生效并持久化到 localStorage。 */
function ThemeCard() {
  const [theme, setThemeState] = useState<Theme>(() => getStoredTheme());
  return (
    <Card
      title="外观"
      description="深色是 Lovart 方案的默认外观；浅色保留原有配色。改动立即生效，下次启动保持所选主题。"
    >
      <Segmented<Theme>
        label="界面主题"
        value={theme}
        onChange={(next) => {
          setThemeState(next);
          setTheme(next);
        }}
        options={[
          { value: "dark", label: "深色" },
          { value: "light", label: "浅色" },
        ]}
      />
    </Card>
  );
}

function inboxStatusLabel(status: string) {
  if (status === "pending") return "等待处理";
  if (status === "importing") return "导入中";
  if (status === "imported") return "已导入";
  if (status === "duplicate") return "重复文件";
  if (status === "failed") return "失败";
  return status;
}

function TemplateEditor({
  template,
  busy,
  onCancel,
  onSaved,
  onError,
}: {
  template: AnalysisTemplate;
  busy: boolean;
  onCancel: () => void;
  onSaved: () => Promise<void>;
  onError: (error: string) => void;
}) {
  const [draft, setDraft] = useState(template);
  const [saving, setSaving] = useState(false);
  const [latestSectionIndex, setLatestSectionIndex] = useState<number | null>(null);
  const latestSectionRef = useRef<HTMLDivElement | null>(null);
  const maxSections = 10;

  useEffect(() => {
    if (latestSectionIndex === null) return;
    const frame = window.requestAnimationFrame(() => {
      latestSectionRef.current?.scrollIntoView({ block: "nearest" });
    });
    return () => window.cancelAnimationFrame(frame);
  }, [latestSectionIndex, draft.customSections.length]);

  async function save() {
    setSaving(true);
    onError("");
    try {
      const input = {
        name: draft.name,
        description: draft.description,
        focusInstructions: draft.focusInstructions,
        customSections: draft.customSections,
      };
      if (template.id.startsWith("new-")) await createAnalysisTemplate(input);
      else await updateAnalysisTemplate(template.id, input);
      await onSaved();
    } catch (reason) {
      onError(String(reason));
    } finally {
      setSaving(false);
    }
  }

  function updateSection(index: number, patch: Partial<TemplateSection>) {
    setDraft({
      ...draft,
      customSections: draft.customSections.map((section, current) =>
        current === index ? { ...section, ...patch } : section,
      ),
    });
  }

  function addSection() {
    if (draft.customSections.length >= maxSections) return;
    const nextIndex = draft.customSections.length;
    setDraft((current) => {
      if (current.customSections.length >= maxSections) return current;
      return {
        ...current,
        customSections: [
          ...current.customSections,
          newSection(current.customSections),
        ],
      };
    });
    setLatestSectionIndex(nextIndex);
  }

  return (
    <form
      className="em-stack"
      onSubmit={(event) => {
        event.preventDefault();
        void save();
      }}
    >
      <Card
        title={template.id.startsWith("new-") ? "新建模板" : "编辑模板"}
        actions={<Button onClick={onCancel}>返回</Button>}
      >
        <Field label="名称">
          <input
            className="em-input"
            required
            maxLength={40}
            value={draft.name}
            onChange={(event) => setDraft({ ...draft, name: event.target.value })}
          />
        </Field>
        <Field label="说明">
          <input
            className="em-input"
            maxLength={120}
            value={draft.description}
            onChange={(event) => setDraft({ ...draft, description: event.target.value })}
          />
        </Field>
        <Field label="分析重点" hint="用一句话说清这个模板关心什么。">
          <textarea
            className="em-input"
            required
            maxLength={800}
            rows={4}
            value={draft.focusInstructions}
            onChange={(event) => setDraft({ ...draft, focusInstructions: event.target.value })}
          />
        </Field>
      </Card>

      <Card
        title="自定义栏目"
        description="每个栏目会在分析结果里单独成段。"
        actions={
          <Button
            disabled={draft.customSections.length >= maxSections}
            onClick={addSection}
          >
            {draft.customSections.length >= maxSections
              ? "最多 10 个栏目"
              : "添加栏目"}
          </Button>
        }
      >
        {draft.customSections.length === 0 && <p className="em-empty">还没有自定义栏目。</p>}
        {draft.customSections.map((section, index) => (
          <div
            className="em-field-row"
            key={`${draft.id}-section-${index}`}
            ref={index === latestSectionIndex ? latestSectionRef : undefined}
          >
            <input
              className="em-input"
              aria-label="栏目标题"
              placeholder="栏目标题"
              value={section.title}
              onChange={(event) =>
                updateSection(index, { title: event.target.value, key: slugKey(event.target.value, index) })
              }
            />
            <select
              className="em-select"
              aria-label="栏目格式"
              value={section.format}
              onChange={(event) => updateSection(index, { format: event.target.value as "paragraph" | "list" })}
            >
              <option value="list">列表</option>
              <option value="paragraph">段落</option>
            </select>
            <input
              className="em-input"
              aria-label="栏目要求"
              placeholder="提取要求"
              value={section.instruction}
              onChange={(event) => updateSection(index, { instruction: event.target.value })}
            />
            <Button
              variant="danger"
              title="删除栏目"
              onClick={() => {
                setLatestSectionIndex(null);
                setDraft({
                  ...draft,
                  customSections: draft.customSections.filter((_, current) => current !== index),
                });
              }}
            >
              ×
            </Button>
          </div>
        ))}
      </Card>

      <div className="em-card-actions">
        <Button onClick={onCancel}>取消</Button>
        <Button
          variant="primary"
          disabled={busy || saving || !draft.name.trim() || !draft.focusInstructions.trim()}
          onClick={() => void save()}
        >
          {saving ? "保存中…" : "保存模板"}
        </Button>
      </div>
    </form>
  );
}

function emptyTemplate(): AnalysisTemplate {
  const now = new Date().toISOString();
  return {
    id: `new-${Date.now()}`,
    name: "",
    description: "",
    focusInstructions: "",
    customSections: [],
    isBuiltin: false,
    createdAt: now,
    updatedAt: now,
  };
}

function newSection(existing: TemplateSection[]): TemplateSection {
  const keys = new Set(existing.map((section) => section.key));
  let index = 1;
  while (keys.has(`section_${index}`)) index += 1;
  return { key: `section_${index}`, title: "", format: "list", instruction: "" };
}

function slugKey(title: string, index: number) {
  const key = title
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9\u4e00-\u9fff]+/g, "_")
    .replace(/^_+|_+$/g, "");
  return key || `section_${index + 1}`;
}
