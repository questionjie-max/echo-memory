import { open } from "@tauri-apps/plugin-dialog";
import { useEffect, useRef, useState } from "react";
import type { LocalAiStatus, OnboardingStatus, SuggestedWatchFolder } from "../shared/types";
import { ANALYSIS_MODEL_SUGGESTIONS, OllamaModelList, WhisperModelList } from "./ModelList";
import { Button, Card, List, ListRow, Pill, Stack, StatusPill } from "./SettingsKit";
import { useModelDownloads } from "./useModelDownloads";
import { PRIVACY_POSTURE } from "../lib/posture";
import {
  addInboxWatchFolder,
  completeOnboarding,
  getLocalAiStatus,
  getOnboardingStatus,
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
 *
 * 模型这几步复用设置里的 `ModelList`：引导和设置看到的是同一批组件、同一份状态，
 * 不会出现「引导里下完了、设置里还不认」这种两边各说各话的情况。
 * 任何一步都可以跳过；跳过模型配置时产品进入「先导入先管理」的降级模式。
 */
export default function OnboardingWizard({ onFinished }: Props) {
  const [step, setStep] = useState<WizardStep>(0);
  const [status, setStatus] = useState<OnboardingStatus | null>(null);
  const [local, setLocal] = useState<LocalAiStatus | null>(null);
  const [suggestions, setSuggestions] = useState<SuggestedWatchFolder[]>([]);
  const [chosenFolders, setChosenFolders] = useState<Set<string>>(new Set());
  const [usbDetection, setUsbDetection] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [finishing, setFinishing] = useState(false);
  const statusRequestId = useRef(0);
  const { downloads, start, cancel } = useModelDownloads((settled) => {
    if (settled.phase === "done") void refreshStatus();
    if (settled.phase === "failed") setError(settled.error ?? "下载失败");
  });

  async function refreshStatus() {
    const requestId = ++statusRequestId.current;
    try {
      const [nextStatus, nextSuggestions, nextLocal] = await Promise.all([
        getOnboardingStatus(),
        suggestWatchFolders(),
        getLocalAiStatus(),
      ]);
      if (requestId !== statusRequestId.current) return;
      setStatus(nextStatus);
      setSuggestions(nextSuggestions);
      setLocal(nextLocal);
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

  async function chooseLocalWhisperModel() {
    setError(null);
    try {
      const selected = await open({
        multiple: false,
        filters: [{ name: "Whisper GGML 模型", extensions: ["bin"] }],
      });
      if (typeof selected !== "string") return;
      const current = await getLocalAiStatus();
      await updateKnowledgeSettings({ ...current.settings, whisperModelPath: selected });
      await refreshStatus();
    } catch (reason) {
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

  const recommendedDownload = local ? downloads[local.recommendedWhisperModel.id] : undefined;
  const analysisDownload = local ? downloads[local.settings.analysisModel] : undefined;

  return (
    <div className="onboarding-scrim" role="presentation">
      <section className="onboarding-panel" role="dialog" aria-modal="true" aria-label="首次启动引导">
        <header className="onboarding-header">
          <div>
            <h2>回声记忆 · 初始设置</h2>
            <p>
              {STEP_TITLES[step]}（{step + 1}/{STEP_TITLES.length}）
            </p>
          </div>
          <Button onClick={() => void finishLater()}>稍后再说</Button>
        </header>
        <div className="onboarding-steps" aria-hidden="true">
          {STEP_TITLES.map((_, index) => (
            <span key={index} className={`onboarding-step-dot${index <= step ? " active" : ""}`} />
          ))}
        </div>

        <div className="onboarding-body">
          {step === 0 && (
            <Stack>
              <h3>让 AI 记得你经历过什么</h3>
              <p className="em-hint">
                回声记忆把你散落的录音、会议和文档，变成可追溯、可检索的个人长期记忆，
                并通过本地 MCP 提供给你的每一个 AI 助手。
              </p>
              <List>
                <ListRow static state="done" title={PRIVACY_POSTURE.onboardingFirstItem} />
                <ListRow static state="done" title="外部 AI 默认关闭；即使开启也只发送所选文本，永不上传音频" />
                <ListRow static state="done" title="MCP 连接默认关闭、只读、不监听网络端口" />
              </List>
            </Stack>
          )}

          {step === 1 && (
            <Stack>
              <h3>环境体检</h3>
              {!status || !local ? (
                <p className="em-hint" role="status">
                  正在检查…
                </p>
              ) : (
                <List>
                  <ListRow
                    static
                    state="done"
                    title="本地转写引擎"
                    badge={<StatusPill ok={local.whisperAvailable} okText="可用" badText="缺模型" />}
                    meta={local.whisperAvailable ? local.whisperModelPath ?? undefined : "下一步解决"}
                  />
                  <ListRow
                    static
                    state={local.ollamaAvailable ? "done" : "error"}
                    title="Ollama 服务"
                    badge={<StatusPill ok={local.ollamaAvailable} okText="运行中" badText="未运行" />}
                    meta={local.ollamaAvailable ? "分析功能可用的前提" : "分析功能需要它，可稍后安装"}
                  />
                  <ListRow
                    static
                    state={status.analysisModelReady ? "done" : "error"}
                    title={`分析模型 ${status.analysisModel}`}
                    badge={<StatusPill ok={status.analysisModelReady} okText="已安装" badText="未安装" />}
                    meta={status.analysisModelReady ? undefined : "下一步解决"}
                  />
                </List>
              )}
              <p className="em-hint">
                不影响开始使用：你可以先导入和管理录音，模型配置好后再转写。
              </p>
            </Stack>
          )}

          {step === 2 && (
            <Stack>
              <h3>选择转写模型</h3>
              <p className="em-hint">
                转写完全在本机运行，录音不出这台机器。推荐 large-v3-turbo：中文准确率更好，
                下载支持断点续传。下载完成后会自动设为当前模型。
              </p>
              {local ? (
                <Card title="可用模型">
                  <WhisperModelList
                    status={local}
                    download={recommendedDownload}
                    onSelect={(path) => {
                      void updateKnowledgeSettings({ ...local.settings, whisperModelPath: path })
                        .then(refreshStatus)
                        .catch((reason) => setError(String(reason)));
                    }}
                    onDownload={(modelId) => void start("whisper", modelId)}
                    onCancel={(model) => void cancel(model)}
                    onChooseFile={() => void chooseLocalWhisperModel()}
                  />
                </Card>
              ) : (
                <p className="em-hint">正在读取模型状态…</p>
              )}
            </Stack>
          )}

          {step === 3 && (
            <Stack>
              <h3>配置分析模型</h3>
              <p className="em-hint">
                结构化分析（摘要 / 决策 / 待办）默认由本机 Ollama 完成，需要 Ollama 服务处于运行状态。
                也可以稍后在 设置 → AI 模型 里改成外部 API。
              </p>
              {local && !local.ollamaAvailable && (
                <p className="em-note warn">
                  Ollama 未运行。先安装并启动 Ollama（ollama.com），再回到这一步。
                </p>
              )}
              {local && (
                <Card title="分析模型">
                  <OllamaModelList
                    models={local.ollamaModels}
                    selected={local.settings.analysisModel}
                    suggestions={ANALYSIS_MODEL_SUGGESTIONS}
                    download={analysisDownload}
                    available={local.ollamaAvailable}
                    role="analysis"
                    onSelect={(name) => {
                      void updateKnowledgeSettings({ ...local.settings, analysisModel: name })
                        .then(refreshStatus)
                        .catch((reason) => setError(String(reason)));
                    }}
                    onDownload={(name) => void start("ollama", name)}
                    onCancel={(model) => void cancel(model)}
                  />
                </Card>
              )}
            </Stack>
          )}

          {step === 4 && (
            <Stack>
              <h3>音频收件箱</h3>
              <p className="em-hint">
                勾选要监听的文件夹：录音笔、手机、微信传来的音频文件一出现就自动导入转写，不用手动操作。
              </p>
              <List>
                {suggestions.map((folder) => (
                  <ListRow
                    key={folder.path}
                    state={chosenFolders.has(folder.path) ? "done" : "idle"}
                    title={folder.label}
                    meta={folder.path}
                    badge={chosenFolders.has(folder.path) ? <Pill tone="accent">已勾选</Pill> : undefined}
                    onSelect={() => toggleFolder(folder.path)}
                  />
                ))}
              </List>
              <label className="em-check-row">
                <input
                  type="checkbox"
                  checked={usbDetection}
                  onChange={(event) => setUsbDetection(event.target.checked)}
                />
                <span>插入 USB 录音设备时自动扫描导入（推荐）</span>
              </label>
            </Stack>
          )}

          {step === 5 && (
            <Stack>
              <h3>准备就绪</h3>
              <List>
                <ListRow static state="done" title="把音频文件拖进窗口，或放进监听文件夹，就会自动转写分析" />
                <ListRow static state="done" title="在「设置 → 收件箱 / 词汇库」里随时调整监听目录与热词" />
                <ListRow static state="done" title="需要跨记录的记忆生成时，再去 设置 → AI 模型 配置外部 API" />
                <ListRow static state="done" title="一切数据都在这台 Mac 上，随时可导出" />
              </List>
            </Stack>
          )}
        </div>

        {error && (
          <p className="em-note err" role="alert">
            {error}
          </p>
        )}

        <footer className="onboarding-footer">
          {step > 0 && (
            <Button onClick={() => setStep((current) => (current - 1) as WizardStep)}>上一步</Button>
          )}
          {step < 5 ? (
            <Button variant="primary" onClick={() => setStep((current) => (current + 1) as WizardStep)}>
              下一步
            </Button>
          ) : (
            <Button variant="primary" disabled={finishing} onClick={() => void finish()}>
              {finishing ? "正在完成…" : "开始使用"}
            </Button>
          )}
        </footer>
      </section>
    </div>
  );
}
