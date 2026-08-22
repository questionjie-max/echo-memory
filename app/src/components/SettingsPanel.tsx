import { open } from "@tauri-apps/plugin-dialog";
import { listen } from "@tauri-apps/api/event";
import { isTauri } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import { open as openDirectory } from "@tauri-apps/plugin-dialog";
import TemplateWizard from "./TemplateWizard";
import type {
  AnalysisTemplate,
  AudioPreprocessorStatus,
  ExternalAiSettings,
  Hotword,
  InboxStatus,
  OutputStatus,
  LocalAiStatus,
  ModelDownloadProgress,
  TemplateSection,
} from "../shared/types";
import {
  createAnalysisTemplate,
  deleteAnalysisTemplate,
  addHotword,
  addInboxWatchFolder,
  downloadWhisperModel,
  getAudioPreprocessorStatus,
  getInboxStatus,
  getTranscriptCorrectionEnabled,
  listHotwords,
  removeHotword,
  removeInboxWatchFolder,
  resetOnboarding,
  rescanInbox,
  getOutputStatus,
  setAutoExportAnalysis,
  setOutputFolder,
  setInboxUsbDetection,
  setTranscriptCorrectionEnabled,
  getExternalAiSettings,
  getLocalAiStatus,
  listAnalysisTemplates,
  pullOllamaModel,
  clearExternalAiApiKey,
  setExternalAiApiKey,
  testExternalAiConnection,
  updateExternalAiSettings,
  updateAnalysisTemplate,
  updateKnowledgeSettings,
} from "../lib/tauri";

interface Props {
  open: boolean;
  onClose: () => void;
}

type SettingsTab = "ai" | "external" | "templates" | "inbox" | "hotwords" | "output";

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

export default function SettingsPanel({ open: visible, onClose }: Props) {
  const [tab, setTab] = useState<SettingsTab>("ai");
  const [inbox, setInbox] = useState<InboxStatus | null>(null);
  const [hotwords, setHotwords] = useState<Hotword[]>([]);
  const [hotwordInput, setHotwordInput] = useState("");
  const [correctionEnabled, setCorrectionEnabled] = useState(false);
  const [inboxError, setInboxError] = useState<string | null>(null);
  const [wizardOpen, setWizardOpen] = useState(false);
  const [output, setOutput] = useState<OutputStatus | null>(null);

  async function refreshInbox() {
    try {
      setInbox(await getInboxStatus());
      setInboxError(null);
    } catch (reason) {
      setInboxError(String(reason));
    }
  }

  async function refreshHotwords() {
    try {
      setHotwords(await listHotwords());
    } catch (reason) {
      setInboxError(String(reason));
    }
  }

  async function chooseWatchFolder() {
    try {
      const selected = await openDirectory({ directory: true, multiple: false });
      if (typeof selected !== "string") return;
      await addInboxWatchFolder(selected);
      await refreshInbox();
    } catch (reason) {
      setInboxError(String(reason));
    }
  }

  async function submitHotword() {
    const term = hotwordInput.trim();
    if (!term) return;
    try {
      await addHotword(term);
      setHotwordInput("");
      await refreshHotwords();
    } catch (reason) {
      setInboxError(String(reason));
    }
  }

  async function toggleCorrection(enabled: boolean) {
    try {
      await setTranscriptCorrectionEnabled(enabled);
      setCorrectionEnabled(enabled);
    } catch (reason) {
      setInboxError(String(reason));
    }
  }

  async function chooseOutputFolder() {
    try {
      const selected = await openDirectory({ directory: true, multiple: false });
      if (typeof selected !== "string") return;
      setOutput(await setOutputFolder(selected));
      setInboxError(null);
    } catch (reason) {
      setInboxError(String(reason));
    }
  }

  async function clearOutputFolder() {
    try {
      setOutput(await setOutputFolder(null));
    } catch (reason) {
      setInboxError(String(reason));
    }
  }

  async function toggleAutoExport(enabled: boolean) {
    try {
      setOutput(await setAutoExportAnalysis(enabled));
    } catch (reason) {
      setInboxError(String(reason));
    }
  }

  async function rerunOnboarding() {
    try {
      await resetOnboarding();
      onClose();
    } catch (reason) {
      setInboxError(String(reason));
    }
  }
  const [status, setStatus] = useState<LocalAiStatus | null>(null);
  const [external, setExternal] = useState<ExternalAiSettings | null>(null);
  const [externalApiKey, setExternalApiKeyDraft] = useState("");
  const [privacyAcknowledged, setPrivacyAcknowledged] = useState(false);
  const [preprocessor, setPreprocessor] = useState<AudioPreprocessorStatus | null>(null);
  const [templates, setTemplates] = useState<AnalysisTemplate[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [download, setDownload] = useState<ModelDownloadProgress | null>(null);
  const [whisperDownload, setWhisperDownload] = useState<ModelDownloadProgress | null>(null);
  const [editing, setEditing] = useState<AnalysisTemplate | null>(null);
  const currentWhisperPath = status?.settings.whisperModelPath || status?.whisperModelPath || "";

  async function refresh() {
    setError("");
    try {
      const [ai, audio, items, externalSettings] = await Promise.all([getLocalAiStatus(), getAudioPreprocessorStatus(), listAnalysisTemplates(), getExternalAiSettings()]);
      setStatus(ai);
      setPreprocessor(audio);
      setTemplates(items);
      setExternal(externalSettings);
      setPrivacyAcknowledged(Boolean(externalSettings.privacyConsentAt));
    } catch (reason) {
      setError(String(reason));
    }
  }

  useEffect(() => {
    if (visible && tab === "output") {
      void getOutputStatus().then(setOutput).catch(() => setOutput(null));
    }
    if (visible && (tab === "inbox" || tab === "hotwords")) {
      void refreshInbox();
      void refreshHotwords();
      void getOutputStatus().then(setOutput).catch(() => setOutput(null));
      void getTranscriptCorrectionEnabled().then(setCorrectionEnabled).catch(() => setCorrectionEnabled(false));
    }
  }, [visible]);

  useEffect(() => {
    if (!isTauri()) return;

    const unlisten = listen<ModelDownloadProgress>("model-download-progress", (event) => {
      setDownload(event.payload);
      if (event.payload.status === "completed") {
        setNotice(`模型“${event.payload.model}”已安装，确认后可设为当前模型。`);
        void refresh();
      }
      if (event.payload.status === "failed") setError(event.payload.error ?? "模型下载失败");
    }).catch((reason) => {
      setError(`无法监听模型下载进度：${String(reason)}`);
      return () => undefined;
    });
    return () => { void unlisten.then((stop) => stop()); };
  }, []);

  useEffect(() => {
    if (!isTauri()) return;

    const unlisten = listen<ModelDownloadProgress>("whisper-model-download-progress", (event) => {
      setWhisperDownload(event.payload);
      if (event.payload.status === "completed") {
        setNotice("Whisper large-v3-turbo-q5_0 已安装，确认后可设为当前模型。");
        void refresh();
      }
      if (event.payload.status === "failed") setError(event.payload.error ?? "Whisper 模型下载失败");
    }).catch((reason) => {
      setError(`无法监听 Whisper 下载进度：${String(reason)}`);
      return () => undefined;
    });
    return () => { void unlisten.then((stop) => stop()); };
  }, []);

  if (!visible) return null;

  async function chooseWhisperModel() {
    const selected = await open({ multiple: false, directory: false, filters: [{ name: "Whisper 模型", extensions: ["bin"] }] });
    if (typeof selected === "string" && status) {
      setStatus({ ...status, settings: { ...status.settings, whisperModelPath: selected } });
    }
  }

  async function saveAi() {
    if (!status) return;
    setBusy(true);
    setError("");
    setNotice("");
    try {
      const settings = await updateKnowledgeSettings(status.settings);
      setStatus({ ...status, settings });
      setNotice("本机 AI 设置已保存");
      await refresh();
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  async function installOllamaModel(model: string, size: string, purpose: string) {
    if (!window.confirm(`下载“${model}”约需 ${size}，仅用于本机${purpose}。下载完成后不会自动切换，继续吗？`)) return;
    setError("");
    setNotice("");
    setDownload({ model, status: "正在连接", completed: null, total: null, error: null });
    try {
      await pullOllamaModel(model);
    } catch (reason) {
      setDownload(null);
      setError(String(reason));
    }
  }

  async function saveExternal() {
    if (!external) return;
    if (external.enabled && !external.hasApiKey) {
      setError("启用外部 AI 前请先配置 API Key。");
      return;
    }
    if (external.enabled && !external.privacyConsentAt && !privacyAcknowledged) {
      setError("请先确认外部发送范围说明。");
      return;
    }
    setBusy(true);
    setError("");
    setNotice("");
    try {
      const saved = await updateExternalAiSettings({
        ...external,
        privacyConsentAt: external.privacyConsentAt || (privacyAcknowledged ? new Date().toISOString() : null),
      });
      setExternal(saved);
      setPrivacyAcknowledged(Boolean(saved.privacyConsentAt));
      setNotice("外部 AI 设置已保存");
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  async function saveExternalKey() {
    if (!externalApiKey.trim()) return;
    setBusy(true);
    setError("");
    try {
      const saved = await setExternalAiApiKey(externalApiKey.trim());
      setExternal(saved);
      setExternalApiKeyDraft("");
      setNotice("API Key 已安全保存到 macOS 钥匙串");
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  async function removeExternalKey() {
    if (!window.confirm("清除外部 AI API Key？外部 AI 将无法生成记忆快照。")) return;
    setBusy(true);
    try {
      setExternal(await clearExternalAiApiKey());
      setNotice("API Key 已清除");
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  async function checkExternalConnection() {
    setBusy(true);
    setError("");
    setNotice("");
    try {
      await testExternalAiConnection();
      setNotice("连接测试成功");
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  async function installWhisperModel() {
    if (!window.confirm("下载 Whisper large-v3-turbo-q5_0 本地模型（574,041,195 字节，约 547.4MB；SHA-256 394221709cd5…）。完成后不会自动切换，继续吗？")) return;
    setError("");
    setNotice("");
    setWhisperDownload({ model: "large-v3-turbo-q5_0", status: "正在连接", completed: null, total: null, error: null });
    try {
      await downloadWhisperModel("large-v3-turbo-q5_0");
    } catch (reason) {
      setWhisperDownload(null);
      setError(String(reason));
    }
  }

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
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="settings-scrim" role="presentation" onMouseDown={(event) => event.target === event.currentTarget && onClose()}>
      <section className="settings-panel material" role="dialog" aria-modal="true" aria-label="设置">
        <header className="settings-header">
          <div><p className="pane-eyebrow">回声记忆</p><h2>设置</h2></div>
          <button type="button" className="close-button" onClick={onClose} aria-label="关闭设置" title="关闭">×</button>
        </header>
        <div className="settings-tabs" role="tablist">
          <button type="button" className={tab === "ai" ? "selected" : ""} onClick={() => setTab("ai")}>本机 AI</button>
          <button type="button" className={tab === "external" ? "selected" : ""} onClick={() => setTab("external")}>外部 AI</button>
          <button type="button" className={tab === "templates" ? "selected" : ""} onClick={() => setTab("templates")}>分析模板</button>
          <button type="button" className={tab === "inbox" ? "selected" : ""} onClick={() => setTab("inbox")}>收件箱</button>
          <button type="button" className={tab === "hotwords" ? "selected" : ""} onClick={() => setTab("hotwords")}>词汇库</button>
          <button type="button" className={tab === "output" ? "selected" : ""} onClick={() => setTab("output")}>产出</button>
        </div>
        <div className="settings-content">
          {tab === "ai" ? (
            status ? (
              <div className="settings-form">
                <section className="settings-section">
                  <div className="settings-section-title"><h3>Whisper</h3><StatusLabel ok={status.whisperAvailable} /></div>
                  <label><span>转写语言</span><select value={status.settings.transcriptionLanguage} onChange={(event) => setStatus({ ...status, settings: { ...status.settings, transcriptionLanguage: event.target.value } })}>{LANGUAGES.map(([value, label]) => <option value={value} key={value}>{label}</option>)}</select></label>
                  <label><span>模型文件</span><div className="path-control"><input readOnly value={status.settings.whisperModelPath || status.whisperModelPath || ""} placeholder="尚未选择" /><button type="button" onClick={() => void chooseWhisperModel()}>选择</button></div></label>
                  {status.whisperModelSource && <p className="settings-meta">{status.whisperModelSource}</p>}
                  {status.whisperModels.length > 0 && <div className="installed-models">{status.whisperModels.map((model) => (
                    <div className="model-row" key={model.path}>
                      <span>{model.id} · {formatBytes(model.size)}</span>
                      <button type="button" className="secondary-button" disabled={currentWhisperPath === model.path} onClick={() => setStatus({ ...status, settings: { ...status.settings, whisperModelPath: model.path } })}>{currentWhisperPath === model.path ? "当前模型" : "设为当前"}</button>
                    </div>
                  ))}</div>}
                  <div className="model-row"><span>large-v3-turbo-q5_0 · 547.4MB · SHA-256 394221709cd5…</span><button type="button" className="secondary-button" disabled={whisperDownload?.status === "downloading" || status.whisperModels.some((model) => model.id === "ggml-large-v3-turbo-q5_0")} onClick={() => void installWhisperModel()}>{status.whisperModels.some((model) => model.id === "ggml-large-v3-turbo-q5_0") ? "已安装" : "下载"}</button></div>
                  {whisperDownload && <DownloadProgress progress={whisperDownload} />}
                </section>
                <section className="settings-section">
                  <div className="settings-section-title"><h3>音频预处理</h3><StatusLabel ok={Boolean(preprocessor?.enhancedAvailable)} /></div>
                  <p className="settings-meta">{preprocessor?.enhancedAvailable ? `增强预处理 · ${preprocessor.engine}${preprocessor.version ? ` · ${preprocessor.version}` : ""}` : "FFmpeg 不可用，将明确使用兼容预处理。"}</p>
                  {preprocessor?.executablePath && <p className="settings-meta">{preprocessor.executablePath}</p>}
                </section>
                <section className="settings-section">
                  <div className="settings-section-title"><h3>Ollama</h3><StatusLabel ok={status.ollamaAvailable} /></div>
                  <label><span>分析模型</span><input value={status.settings.analysisModel} onChange={(event) => setStatus({ ...status, settings: { ...status.settings, analysisModel: event.target.value } })} /></label>
                  <div className="model-row"><span>qwen3.5:4b · 可选分析模型</span>{status.ollamaModels.some((model) => model.name === "qwen3.5:4b") ? <button type="button" className="secondary-button" disabled={status.settings.analysisModel === "qwen3.5:4b"} onClick={() => setStatus({ ...status, settings: { ...status.settings, analysisModel: "qwen3.5:4b" } })}>{status.settings.analysisModel === "qwen3.5:4b" ? "当前模型" : "设为当前"}</button> : <button type="button" className="secondary-button" disabled={!status.ollamaAvailable || download !== null && download.status !== "completed" && download.status !== "failed"} onClick={() => void installOllamaModel("qwen3.5:4b", "约 3GB", "文稿分析")}>下载</button>}</div>
                  <label><span>嵌入模型</span><input value={status.settings.embeddingModel} onChange={(event) => setStatus({ ...status, settings: { ...status.settings, embeddingModel: event.target.value } })} /></label>
                  <div className="model-row"><span>{status.ollamaModels.some((model) => model.name === status.settings.embeddingModel) ? "嵌入模型已安装" : "嵌入模型未安装"}</span><button type="button" className="secondary-button" disabled={!status.ollamaAvailable || status.ollamaModels.some((model) => model.name === status.settings.embeddingModel) || download !== null && download.status !== "completed" && download.status !== "failed"} onClick={() => void installOllamaModel(status.settings.embeddingModel, "639MB", "知识索引")}>{status.ollamaModels.some((model) => model.name === status.settings.embeddingModel) ? "已安装" : "下载模型"}</button></div>
                  {download && <DownloadProgress progress={download} />}
                  {status.ollamaModels.length > 0 && <p className="settings-meta">已安装：{status.ollamaModels.map((model) => model.name).join("、")}</p>}
                </section>
                <button type="button" className="primary-button settings-save" disabled={busy} onClick={() => void saveAi()}>{busy ? "保存中…" : "保存设置"}</button>
              </div>
            ) : <p className="settings-empty">正在读取本机状态…</p>
          ) : tab === "external" ? (
            external ? (
              <div className="settings-form">
                <section className="settings-section external-ai-warning">
                  <div className="settings-section-title"><h3>跨记录记忆生成</h3><StatusLabel ok={external.enabled && external.hasApiKey} /></div>
                  <p className="settings-meta">默认关闭。启用后，所选范围内的结构化分析和逐字稿全文会发送给你配置的 OpenAI-compatible 服务商；原始音频永不上传。本机 Whisper、单条分析、知识库问答和 MCP 仍保持本地。</p>
                </section>
                <section className="settings-section">
                  <label className="settings-switch-row"><span>启用外部 AI</span><input type="checkbox" checked={external.enabled} onChange={(event) => setExternal({ ...external, enabled: event.target.checked })} /></label>
                  <label><span>Base URL</span><input value={external.baseUrl} placeholder="https://api.openai.com/v1" onChange={(event) => setExternal({ ...external, baseUrl: event.target.value })} /></label>
                  <label><span>模型名称</span><input value={external.model} placeholder="gpt-4o-mini" onChange={(event) => setExternal({ ...external, model: event.target.value })} /></label>
                  <div className="model-row"><span>{external.hasApiKey ? "API Key 已配置（不会显示或回填）" : "尚未配置 API Key"}</span><span className={`settings-status ${external.hasApiKey ? "ready" : "missing"}`}>{external.hasApiKey ? "已配置" : "缺失"}</span></div>
                  <label><span>设置新的 API Key</span><input type="password" value={externalApiKey} autoComplete="new-password" placeholder="仅用于保存，不会回显" onChange={(event) => setExternalApiKeyDraft(event.target.value)} /></label>
                  <div className="settings-actions"><button type="button" className="secondary-button" disabled={busy || !externalApiKey.trim()} onClick={() => void saveExternalKey()}>保存 Key</button><button type="button" className="secondary-button" disabled={busy || !external.hasApiKey} onClick={() => void removeExternalKey()}>清除 Key</button><button type="button" className="secondary-button" disabled={busy || !external.hasApiKey} onClick={() => void checkExternalConnection()}>测试连接</button></div>
                </section>
                <section className="settings-section">
                  <label className="settings-check-row"><input type="checkbox" checked={privacyAcknowledged} onChange={(event) => setPrivacyAcknowledged(event.target.checked)} /><span>我已了解：启用后会发送选定记录的逐字稿全文和现有结构化分析，但不会发送音频。</span></label>
                  <p className="settings-meta">云端语音转写预留字段：{external.transcriptionProvider || "未启用 / 本轮不支持"}</p>
                </section>
                <button type="button" className="primary-button settings-save" disabled={busy} onClick={() => void saveExternal()}>{busy ? "保存中…" : "保存外部 AI 设置"}</button>
              </div>
            ) : <p className="settings-empty">正在读取外部 AI 设置…</p>
          ) : tab === "inbox" ? (
            inbox ? (
              <div className="settings-form">
                <section className="settings-section">
                  <div className="settings-section-title"><h3>音频收件箱</h3><StatusLabel ok={inbox.watchFolders.length > 0 || inbox.usbDetection} /></div>
                  <p className="settings-meta">监听下面的文件夹：新音频文件出现后自动导入并转写分析。添加监听时的已有文件会被跳过（只接住之后新出现的文件）；U 盘录音设备例外——插入后设备上的录音属于你要导入的内容。</p>
                  <label className="settings-switch-row"><span>插入 USB 录音设备时自动扫描</span><input type="checkbox" checked={inbox.usbDetection} onChange={(event) => { void setInboxUsbDetection(event.target.checked).then(refreshInbox); setInbox({ ...inbox, usbDetection: event.target.checked }); }} /></label>
                  <div className="settings-actions">
                    <button type="button" className="secondary-button" onClick={() => void chooseWatchFolder()}>添加监听文件夹…</button>
                    <button type="button" className="secondary-button" onClick={() => { void rescanInbox().then(refreshInbox); }}>立即扫描</button>
                  </div>
                  {inbox.watchFolders.length > 0 ? (
                    <div className="installed-models">{inbox.watchFolders.map((folder) => (
                      <div className="model-row" key={folder.id}>
                        <span>{folder.label} · {folder.path}</span>
                        <button type="button" className="danger-text" onClick={() => { void removeInboxWatchFolder(folder.id).then(refreshInbox); }}>移除</button>
                      </div>
                    ))}</div>
                  ) : <p className="settings-empty">尚未监听任何文件夹。</p>}
                </section>
                <section className="settings-section">
                  <div className="settings-section-title"><h3>最近自动导入</h3></div>
                  {inbox.recentFiles.length === 0 && <p className="settings-empty">还没有文件进入收件箱。</p>}
                  {inbox.recentFiles.length > 0 && (
                    <div className="installed-models">{inbox.recentFiles.map((file) => (
                      <div className="model-row" key={file.id}>
                        <span>{file.fileName} · {inboxStatusLabel(file.status)}{file.errorMessage ? ` · ${file.errorMessage}` : ""}</span>
                      </div>
                    ))}</div>
                  )}
                  <p className="settings-meta">待处理 {inbox.counts.pending} · 已导入 {inbox.counts.imported} · 失败 {inbox.counts.failed}</p>
                </section>
              </div>
            ) : <p className="settings-empty">正在读取收件箱状态…</p>
          ) : tab === "hotwords" ? (
            <div className="settings-form">
              <section className="settings-section">
                <div className="settings-section-title"><h3>个人词汇库</h3><StatusLabel ok={hotwords.length > 0} /></div>
                <p className="settings-meta">热词会注入转写提示与 AI 校对，显著改善专有名词、人名、产品名的中文识别（如「回声记忆」「小能熊」不会被写成同音字）。</p>
                <div className="settings-actions hotword-input-row">
                  <input value={hotwordInput} placeholder="输入热词后回车" onChange={(event) => setHotwordInput(event.target.value)} onKeyDown={(event) => { if (event.key === "Enter" && !event.nativeEvent.isComposing) void submitHotword(); }} />
                  <button type="button" className="secondary-button" disabled={!hotwordInput.trim()} onClick={() => void submitHotword()}>添加</button>
                </div>
                {hotwords.length > 0 ? (
                  <div className="installed-models">{hotwords.map((hotword) => (
                    <div className="model-row" key={hotword.id}>
                      <span>{hotword.term}{hotword.note ? ` · ${hotword.note}` : ""}</span>
                      <button type="button" className="danger-text" onClick={() => { void removeHotword(hotword.id).then(refreshHotwords); }}>删除</button>
                    </div>
                  ))}</div>
                ) : <p className="settings-empty">还没有热词。</p>}
              </section>
              <section className="settings-section">
                <div className="settings-section-title"><h3>转写 AI 校对</h3></div>
                <label className="settings-switch-row"><span>转写完成后自动用本机模型校对（断句、标点、热词纠正）</span><input type="checkbox" checked={correctionEnabled} onChange={(event) => void toggleCorrection(event.target.checked)} /></label>
                <p className="settings-meta">校对结果写入独立文本层，原始逐字稿永不覆盖，可随时对比。也可以在记录详情页手动触发。</p>
              </section>
              <section className="settings-section">
                <div className="settings-section-title"><h3>首次启动引导</h3></div>
                <div className="settings-actions"><button type="button" className="secondary-button" onClick={() => void rerunOnboarding()}>重新运行引导向导</button></div>
                <p className="settings-meta">重新走一遍模型与收件箱配置流程。关闭设置后会自动弹出。</p>
              </section>
            </div>
          ) : tab === "output" ? (
            output ? (
              <div className="settings-form">
                <section className="settings-section">
                  <div className="settings-section-title"><h3>产出文件夹</h3><StatusLabel ok={Boolean(output.folder)} /></div>
                  <p className="settings-meta">所有生成内容（分析、逐字稿、AI 对话成稿）以标准 Markdown + YAML frontmatter 写入这个文件夹。可以指向 Obsidian 等笔记工具的目录，用你自己的体系二次管理——回声记忆只负责写出，不感知任何外部工具。</p>
                  <label><span>当前目录</span><div className="path-control"><input readOnly value={output.folder ?? "未设置（默认：~/Documents/回声记忆产出）"} /><button type="button" onClick={() => void chooseOutputFolder()}>选择</button></div></label>
                  <div className="settings-actions">
                    {output.folder && <button type="button" className="secondary-button" onClick={() => void clearOutputFolder()}>恢复默认目录</button>}
                  </div>
                </section>
                <section className="settings-section">
                  <div className="settings-section-title"><h3>自动导出</h3></div>
                  <label className="settings-switch-row"><span>分析完成后自动导出 Markdown 到产出文件夹</span><input type="checkbox" checked={output.autoExportAnalysis} onChange={(event) => void toggleAutoExport(event.target.checked)} /></label>
                  <p className="settings-meta">默认关闭。开启后每条录音分析完成即落盘一个 md 文件；也可随时在记录详情里手动导出。</p>
                </section>
                <section className="settings-section">
                  <div className="settings-section-title"><h3>最近产出</h3></div>
                  {output.recentFiles.length === 0 && <p className="settings-empty">还没有产出文件。</p>}
                  {output.recentFiles.length > 0 && (
                    <div className="installed-models">{output.recentFiles.map((file) => (
                      <div className="model-row" key={file.path}>
                        <span>{file.fileName} · {(file.size / 1024).toFixed(1)}KB</span>
                      </div>
                    ))}</div>
                  )}
                </section>
              </div>
            ) : <p className="settings-empty">正在读取产出设置…</p>
          ) : (
            editing ? <TemplateEditor template={editing} busy={busy} onCancel={() => setEditing(null)} onSaved={async () => { setEditing(null); await refresh(); }} onError={setError} /> : (
              <div className="template-list">
                <div className="template-list-heading"><span>{templates.length} 个模板</span><div className="template-heading-actions"><button type="button" className="secondary-button" onClick={() => setWizardOpen(true)}>✨ AI 生成模板</button><button type="button" className="primary-button" onClick={() => setEditing(emptyTemplate())}>新建模板</button></div></div>
                {templates.map((template) => (
                  <div className="template-row" key={template.id}>
                    <div><strong>{template.name}</strong><span>{template.description || "自定义分析模板"}</span></div>
                    <div className="template-actions">
                      <button type="button" disabled={busy} onClick={() => void duplicateTemplate(template)}>复制</button>
                      {!template.isBuiltin && <button type="button" disabled={busy} onClick={() => setEditing(template)}>编辑</button>}
                      {!template.isBuiltin && <button type="button" className="danger-text" disabled={busy} onClick={() => void removeTemplate(template)}>删除</button>}
                    </div>
                  </div>
                ))}
              </div>
            )
          )}
          {notice && <p className="inline-notice">{notice}</p>}
          {inboxError && <p className="inline-error" role="alert">{inboxError}</p>}
          {error && <p className="inline-error">{error}</p>}
        </div>
      </section>
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
}

function inboxStatusLabel(status: string) {
  if (status === "pending") return "等待处理";
  if (status === "importing") return "导入中";
  if (status === "imported") return "已导入";
  if (status === "duplicate") return "重复文件";
  if (status === "failed") return "失败";
  return status;
}

function StatusLabel({ ok }: { ok: boolean }) {
  return <span className={`settings-status ${ok ? "ready" : "missing"}`}>{ok ? "已就绪" : "未就绪"}</span>;
}

function DownloadProgress({ progress }: { progress: ModelDownloadProgress }) {
  const percent = progress.total && progress.completed !== null ? Math.min(100, Math.round(progress.completed / progress.total * 100)) : null;
  return <div className="download-progress" role="status"><div><span>{progress.status}</span><span>{percent !== null ? `${percent}%` : ""}</span></div><div className="progress-track"><span style={{ width: percent !== null ? `${percent}%` : "12%" }} /></div></div>;
}

function formatBytes(bytes: number) {
  if (bytes < 1024 * 1024) return `${Math.max(1, Math.round(bytes / 1024))}KB`;
  return `${(bytes / 1024 / 1024).toFixed(bytes >= 1024 * 1024 * 1024 ? 0 : 1)}MB`;
}

function TemplateEditor({ template, busy, onCancel, onSaved, onError }: {
  template: AnalysisTemplate;
  busy: boolean;
  onCancel: () => void;
  onSaved: () => Promise<void>;
  onError: (error: string) => void;
}) {
  const [draft, setDraft] = useState(template);
  const [saving, setSaving] = useState(false);
  async function save() {
    setSaving(true);
    onError("");
    try {
      const input = { name: draft.name, description: draft.description, focusInstructions: draft.focusInstructions, customSections: draft.customSections };
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
    setDraft({ ...draft, customSections: draft.customSections.map((section, current) => current === index ? { ...section, ...patch } : section) });
  }
  return <form className="template-editor" onSubmit={(event) => { event.preventDefault(); void save(); }}>
    <div className="editor-heading"><h3>{template.id.startsWith("new-") ? "新建模板" : "编辑模板"}</h3><button type="button" onClick={onCancel}>返回</button></div>
    <label><span>名称</span><input required maxLength={40} value={draft.name} onChange={(event) => setDraft({ ...draft, name: event.target.value })} /></label>
    <label><span>说明</span><input maxLength={120} value={draft.description} onChange={(event) => setDraft({ ...draft, description: event.target.value })} /></label>
    <label><span>分析重点</span><textarea required maxLength={800} value={draft.focusInstructions} onChange={(event) => setDraft({ ...draft, focusInstructions: event.target.value })} /></label>
    <div className="custom-section-heading"><strong>自定义栏目</strong><button type="button" disabled={draft.customSections.length >= 10} onClick={() => setDraft({ ...draft, customSections: [...draft.customSections, newSection(draft.customSections.length)] })}>添加栏目</button></div>
    {draft.customSections.map((section, index) => <div className="custom-section-row" key={`${section.key}-${index}`}>
      <input aria-label="栏目标题" placeholder="栏目标题" value={section.title} onChange={(event) => updateSection(index, { title: event.target.value, key: slugKey(event.target.value, index) })} />
      <select aria-label="栏目格式" value={section.format} onChange={(event) => updateSection(index, { format: event.target.value as "paragraph" | "list" })}><option value="list">列表</option><option value="paragraph">段落</option></select>
      <input aria-label="栏目要求" placeholder="提取要求" value={section.instruction} onChange={(event) => updateSection(index, { instruction: event.target.value })} />
      <button type="button" aria-label="删除栏目" title="删除栏目" onClick={() => setDraft({ ...draft, customSections: draft.customSections.filter((_, current) => current !== index) })}>×</button>
    </div>)}
    <div className="editor-actions"><button type="button" className="secondary-button" onClick={onCancel}>取消</button><button type="submit" className="primary-button" disabled={busy || saving || !draft.name.trim() || !draft.focusInstructions.trim()}>{saving ? "保存中…" : "保存模板"}</button></div>
  </form>;
}

function emptyTemplate(): AnalysisTemplate {
  const now = new Date().toISOString();
  return { id: `new-${Date.now()}`, name: "", description: "", focusInstructions: "", customSections: [], isBuiltin: false, createdAt: now, updatedAt: now };
}

function newSection(index: number): TemplateSection {
  return { key: `section_${index + 1}`, title: "", format: "list", instruction: "" };
}

function slugKey(title: string, index: number) {
  const key = title.trim().toLowerCase().replace(/[^a-z0-9\u4e00-\u9fff]+/g, "_").replace(/^_+|_+$/g, "");
  return key || `section_${index + 1}`;
}
