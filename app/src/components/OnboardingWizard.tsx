import { open } from "@tauri-apps/plugin-dialog";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useRef, useState } from "react";
import type { ModelDownloadProgress, OnboardingStatus, SuggestedWatchFolder } from "../shared/types";
import {
  addInboxWatchFolder,
  completeOnboarding,
  downloadWhisperModel,
  getLocalAiStatus,
  getOnboardingStatus,
  pullOllamaModel,
  setInboxUsbDetection,
  suggestWatchFolders,
  updateKnowledgeSettings,
} from "../lib/tauri";

interface Props {
  onFinished: () => void;
}

type WizardStep = 0 | 1 | 2 | 3 | 4 | 5;

const STEP_TITLES = ["欢迎", "环境体检", "转写模型", "分析模型", "音频收件箱", "完成"];

/**
 * 首次启动引导：隐私说明 → 环境体检 → 模型配置（支持跳过，稍后可补配）→ 收件箱。
 * 任何一步都可以跳过；跳过模型配置时产品进入“先导入先管理”的降级模式。
 */
export default function OnboardingWizard({ onFinished }: Props) {
  const [step, setStep] = useState<WizardStep>(0);
  const [status, setStatus] = useState<OnboardingStatus | null>(null);
  const [suggestions, setSuggestions] = useState<SuggestedWatchFolder[]>([]);
  const [chosenFolders, setChosenFolders] = useState<Set<string>>(new Set());
  const [usbDetection, setUsbDetection] = useState(false);
  const [whisperProgress, setWhisperProgress] = useState<string | null>(null);
  const [ollamaProgress, setOllamaProgress] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [finishing, setFinishing] = useState(false);
  const statusRequestId = useRef(0);

  async function refreshStatus() {
    const requestId = ++statusRequestId.current;
    try {
      const [nextStatus, nextSuggestions] = await Promise.all([
        getOnboardingStatus(),
        suggestWatchFolders(),
      ]);
      if (requestId !== statusRequestId.current) return;
      setStatus(nextStatus);
      setSuggestions(nextSuggestions);
      setUsbDetection(nextStatus.usbDetection);
    } catch (reason) {
      if (requestId === statusRequestId.current) setError(String(reason));
    }
  }

  useEffect(() => {
    void refreshStatus();
  }, []);

  useEffect(() => {
    if (step === 1 || step === 2 || step === 3) void refreshStatus();
  }, [step]);

  useEffect(() => {
    const stopWhisper = listen<ModelDownloadProgress>("whisper-model-download-progress", (event) => {
      const progress = event.payload;
      if (progress.status === "downloading" && progress.total) {
        const percent = Math.min(100, Math.round(((progress.completed ?? 0) / progress.total) * 100));
        setWhisperProgress(`下载中 ${percent}%（中断可续传）`);
      } else if (progress.status === "completed") {
        setWhisperProgress("已下载");
        void refreshStatus();
      } else if (progress.status === "failed") {
        setWhisperProgress(null);
        setError(progress.error ?? "下载失败");
      } else {
        setWhisperProgress(progress.status);
      }
    });
    const stopOllama = listen<ModelDownloadProgress>("model-download-progress", (event) => {
      const progress = event.payload;
      if (progress.status === "completed") {
        setOllamaProgress(null);
        void refreshStatus();
      } else if (progress.status === "failed") {
        setOllamaProgress(null);
        setError(progress.error ?? "模型下载失败");
      } else {
        setOllamaProgress(progress.status);
      }
    });
    return () => {
      void stopWhisper.then((stop) => stop());
      void stopOllama.then((stop) => stop());
    };
  }, []);

  async function downloadWhisper() {
    setError(null);
    try {
      setWhisperProgress("准备下载…");
      await downloadWhisperModel("large-v3-turbo-q5_0");
    } catch (reason) {
      setWhisperProgress(null);
      setError(String(reason));
    }
  }

  async function chooseLocalWhisperModel() {
    setError(null);
    try {
      const selected = await open({
        multiple: false,
        filters: [{ name: "Whisper GGML 模型", extensions: ["bin"] }],
      });
      if (typeof selected !== "string") return;
      const local = await getLocalAiStatus();
      await updateKnowledgeSettings({ ...local.settings, whisperModelPath: selected });
      await refreshStatus();
    } catch (reason) {
      setError(String(reason));
    }
  }

  async function pullAnalysisModel() {
    setError(null);
    try {
      if (!status) return;
      setOllamaProgress("准备下载…");
      await pullOllamaModel(status.analysisModel);
    } catch (reason) {
      setOllamaProgress(null);
      setError(String(reason));
    }
  }

  function toggleFolder(path: string) {
    setChosenFolders((current) => {
      const next = new Set(current);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  }

  async function finish() {
    setFinishing(true);
    setError(null);
    try {
      await setInboxUsbDetection(usbDetection);
      for (const path of chosenFolders) {
        await addInboxWatchFolder(path);
      }
      await completeOnboarding();
      onFinished();
    } catch (reason) {
      setError(String(reason));
    } finally {
      setFinishing(false);
    }
  }

  async function finishLater() {
    try {
      await completeOnboarding();
    } catch {
      // 即便标记失败也不阻塞使用
    }
    onFinished();
  }

  return (
    <div className="onboarding-scrim" role="presentation">
      <section className="onboarding-panel material" role="dialog" aria-modal="true" aria-label="首次启动引导">
        <header className="onboarding-header">
          <div>
            <h2>回声记忆 · 初始设置</h2>
            <p>{STEP_TITLES[step]}（{step + 1}/{STEP_TITLES.length}）</p>
          </div>
          <button type="button" className="toolbar-button" onClick={() => void finishLater()}>稍后再说</button>
        </header>
        <div className="onboarding-steps" aria-hidden="true">
          {STEP_TITLES.map((_, index) => (
            <span key={index} className={`onboarding-step-dot${index <= step ? " active" : ""}`} />
          ))}
        </div>

        {step === 0 && (
          <div className="onboarding-body">
            <h3>让 AI 记得你经历过什么</h3>
            <p>回声记忆把你散落的录音、会议和文档，变成可追溯、可检索的个人长期记忆，并通过本地 MCP 提供给你的每一个 AI 助手。</p>
            <ul className="onboarding-checklist">
              <li>✅ 原始音频、逐字稿、分析全部保留在本机</li>
              <li>✅ 外部 AI 默认关闭；即使开启也只发送所选文本，永不上传音频</li>
              <li>✅ MCP 连接默认关闭、只读、不监听网络端口</li>
            </ul>
          </div>
        )}

        {step === 1 && (
          <div className="onboarding-body">
            <h3>环境体检</h3>
            {!status ? <p role="status">正在检查…</p> : (
              <ul className="onboarding-checklist">
                <li>{status.ollamaRunning ? "✅" : "⚠️"} Ollama {status.ollamaRunning ? "服务运行中" : "未检测到服务（分析功能需要，可稍后启动）"}</li>
                <li>{status.whisperReady ? "✅" : "⚠️"} 本地转写 {status.whisperReady ? "已就绪" : "尚未配置（下一步解决）"}</li>
                <li>{status.analysisModelReady ? "✅" : "⚠️"} 分析模型 {status.analysisModelReady ? `已安装 ${status.analysisModel}` : `未安装 ${status.analysisModel}（下一步解决）`}</li>
              </ul>
            )}
            <p className="onboarding-hint">⚠️ 不影响开始使用：你可以先导入和管理录音，模型配置好后再转写。</p>
          </div>
        )}

        {step === 2 && (
          <div className="onboarding-body">
            <h3>选择转写模型</h3>
            <p className="onboarding-hint">转写完全在本机运行（内嵌 Whisper 引擎）。推荐模型 large-v3-turbo（574MB），中文准确率更好。</p>
            {status?.whisperReady ? (
              <p>✅ 已配置：{status.whisperModelPath}</p>
            ) : (
              <div className="onboarding-actions">
                <button type="button" className="primary-button" disabled={whisperProgress !== null} onClick={() => void downloadWhisper()}>
                  {whisperProgress ? whisperProgress : "下载推荐模型（574MB，支持断点续传）"}
                </button>
                <button type="button" className="toolbar-button" onClick={() => void chooseLocalWhisperModel()}>选择已有的 GGML 模型文件…</button>
              </div>
            )}
          </div>
        )}

        {step === 3 && (
          <div className="onboarding-body">
            <h3>配置分析模型</h3>
            <p className="onboarding-hint">结构化分析（摘要 / 决策 / 待办）由本机 Ollama 模型完成，需要 Ollama 服务处于运行状态。</p>
            {status?.analysisModelReady ? (
              <p>✅ 已安装 {status.analysisModel}</p>
            ) : status?.ollamaRunning ? (
              <div className="onboarding-actions">
                <button type="button" className="primary-button" disabled={ollamaProgress !== null} onClick={() => void pullAnalysisModel()}>
                  {ollamaProgress ? `下载中：${ollamaProgress}` : `下载分析模型 ${status.analysisModel}`}
                </button>
              </div>
            ) : (
              <p>⚠️ Ollama 未运行。请先安装并启动 Ollama（ollama.com），然后在设置 → 本机 AI 里下载模型。</p>
            )}
          </div>
        )}

        {step === 4 && (
          <div className="onboarding-body">
            <h3>音频收件箱</h3>
            <p className="onboarding-hint">勾选要监听的文件夹：录音笔、手机、微信传来的音频文件一出现就自动导入转写，不用手动操作。</p>
            <ul className="onboarding-suggestions">
              {suggestions.map((folder) => (
                <li key={folder.path}>
                  <label>
                    <input
                      type="checkbox"
                      checked={chosenFolders.has(folder.path)}
                      onChange={() => toggleFolder(folder.path)}
                    />
                    <span>{folder.label}<small>{folder.path}</small></span>
                  </label>
                </li>
              ))}
            </ul>
            <label className="onboarding-usb-toggle">
              <input
                type="checkbox"
                checked={usbDetection}
                onChange={(event) => setUsbDetection(event.target.checked)}
              />
              <span>插入 USB 录音设备时自动扫描导入（推荐）</span>
            </label>
          </div>
        )}

        {step === 5 && (
          <div className="onboarding-body">
            <h3>准备就绪</h3>
            <ul className="onboarding-checklist">
              <li>把音频文件拖进窗口，或放进监听文件夹，就会自动转写分析</li>
              <li>在「设置 → 收件箱 / 词汇库」里随时调整监听目录与热词</li>
              <li>一切数据都在这台 Mac 上，随时可导出</li>
            </ul>
          </div>
        )}

        {error && <p className="inline-error" role="alert">{error}</p>}

        <footer className="onboarding-footer">
          {step > 0 && (
            <button type="button" className="toolbar-button" onClick={() => setStep((current) => (current - 1) as WizardStep)}>上一步</button>
          )}
          {step < 5 ? (
            <button type="button" className="primary-button" onClick={() => setStep((current) => (current + 1) as WizardStep)}>下一步</button>
          ) : (
            <button type="button" className="primary-button" disabled={finishing} onClick={() => void finish()}>
              {finishing ? "正在完成…" : "开始使用"}
            </button>
          )}
        </footer>
      </section>
    </div>
  );
}
