import { useEffect, useRef, useState } from "react";
import type { DockMessage, DockMode } from "../shared/types";
import { askDock, clearDockChat, getDockStatus, saveDockMessageToOutput } from "../lib/tauri";

interface Props {
  selectedRecordId: string | null;
  selectedRecordTitle: string | null;
}

const MODES: Array<{ id: DockMode; label: string; hint: string }> = [
  { id: "summary", label: "总结当前记录", hint: "基于当前选中记录的逐字稿与分析" },
  { id: "creation", label: "文字创作", hint: "扩写、改写、风格化成稿" },
  { id: "inspire", label: "启发对话", hint: "费曼式追问，推动思考" },
  { id: "free", label: "自由聊天", hint: "无预设助手" },
];

const DOCK_STORAGE_KEY = "echo-memory-dock-state";

interface PersistedDockState {
  collapsed: boolean;
  engine: "local" | "external";
}

function loadPersisted(): PersistedDockState {
  try {
    const raw = localStorage.getItem(DOCK_STORAGE_KEY);
    if (raw) return { collapsed: false, engine: "local", ...JSON.parse(raw) };
  } catch {
    // 忽略损坏的本地状态
  }
  return { collapsed: true, engine: "local" };
}

/**
 * AI 伙伴停靠栏：所有主视图共享的顶部对话条。
 * 默认本地 Ollama 引擎；外部引擎需已配置并主动选择，选中时展示醒目提示。
 */
export default function AssistantDock({ selectedRecordId, selectedRecordTitle }: Props) {
  const [collapsed, setCollapsed] = useState(loadPersisted().collapsed);
  const [engine, setEngine] = useState<"local" | "external">(loadPersisted().engine);
  const [mode, setMode] = useState<DockMode>("free");
  const [messages, setMessages] = useState<DockMessage[]>([]);
  const [externalAvailable, setExternalAvailable] = useState(false);
  const [draft, setDraft] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const inputRef = useRef<HTMLTextAreaElement | null>(null);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const status = await getDockStatus();
        if (!cancelled) {
          setMessages(status.messages);
          setExternalAvailable(status.externalAvailable);
        }
      } catch (reason) {
        if (!cancelled) setError(String(reason));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    localStorage.setItem(DOCK_STORAGE_KEY, JSON.stringify({ collapsed, engine }));
  }, [collapsed, engine]);

  useEffect(() => {
    if (scrollRef.current) scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
  }, [messages, busy]);

  useEffect(() => {
    if (!collapsed) inputRef.current?.focus();
  }, [collapsed]);

  async function send() {
    const content = draft.trim();
    if (!content || busy) return;
    setBusy(true);
    setError(null);
    setNotice(null);
    const pendingMessage: DockMessage = {
      id: `pending-${Date.now()}`,
      chatId: "",
      role: "user",
      content,
      mode,
      createdAt: new Date().toISOString(),
    };
    setMessages((current) => [...current, pendingMessage]);
    setDraft("");
    try {
      const reply = await askDock({
        mode,
        message: content,
        engine,
        recordId: selectedRecordId,
      });
      setMessages((current) => [
        ...current.filter((message) => message.id !== pendingMessage.id),
        reply.userMessage,
        reply.assistantMessage,
      ]);
    } catch (reason) {
      setMessages((current) => current.filter((message) => message.id !== pendingMessage.id));
      setDraft(content);
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setBusy(false);
    }
  }

  async function saveMessage(message: DockMessage) {
    try {
      const title = messages.find((item) => item.role === "user")?.content.slice(0, 24) ?? "AI 对话";
      const path = await saveDockMessageToOutput(title, message.content);
      setNotice(`已保存：${path.split("/").pop()}`);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    }
  }

  async function clearAll() {
    if (!window.confirm("清空当前对话？此操作不可撤销。")) return;
    try {
      await clearDockChat();
      setMessages([]);
      setNotice(null);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    }
  }

  const activeMode = MODES.find((item) => item.id === mode);

  return (
    <section className={`assistant-dock material${collapsed ? " collapsed" : ""}`} aria-label="AI 伙伴">
      <header className="dock-header">
        <button
          type="button"
          className="dock-toggle"
          aria-expanded={!collapsed}
          onClick={() => setCollapsed((value) => !value)}
        >
          🤖 AI 伙伴{collapsed ? "（收起）" : ""}
        </button>
        {!collapsed && (
          <>
            <select
              className="dock-engine"
              value={engine}
              aria-label="对话引擎"
              onChange={(event) => {
                const next = event.target.value as "local" | "external";
                if (next === "external" && !externalAvailable) return;
                setEngine(next);
              }}
            >
              <option value="local">本地模型</option>
              <option value="external" disabled={!externalAvailable}>
                外部模型{externalAvailable ? "" : "（未配置）"}
              </option>
            </select>
            <div className="dock-modes" role="tablist" aria-label="对话模式">
              {MODES.map((item) => (
                <button
                  type="button"
                  key={item.id}
                  role="tab"
                  aria-selected={mode === item.id}
                  className={mode === item.id ? "selected" : ""}
                  onClick={() => setMode(item.id)}
                >
                  {item.label}
                </button>
              ))}
            </div>
            <span className="dock-hint" title={activeMode?.hint}>
              {mode === "summary"
                ? selectedRecordTitle
                  ? `· ${selectedRecordTitle}`
                  : "· 请先选择一条记录"
                : ""}
            </span>
            <button type="button" className="dock-clear" onClick={() => void clearAll()}>
              清空
            </button>
          </>
        )}
      </header>

      {!collapsed && (
        <>
          {engine === "external" && (
            <p className="dock-external-warning" role="alert">
              当前对话将发送到你配置的外部 AI 服务（仅文本，不含音频）。
            </p>
          )}
          <div className="dock-messages" ref={scrollRef} aria-live="polite">
            {messages.length === 0 && (
              <p className="dock-empty">
                随时对话：总结当前录音、把想法写成成稿、或让 AI 反问你推动思考。
                {activeMode ? `当前模式：${activeMode.label} —— ${activeMode.hint}。` : ""}
              </p>
            )}
            {messages.map((message) => (
              <article key={message.id} className={`dock-message ${message.role}`}>
                <p>{message.content}</p>
                {message.role === "assistant" && (
                  <button
                    type="button"
                    className="dock-save"
                    title="把这条回复保存为 Markdown 文件到产出文件夹"
                    onClick={() => void saveMessage(message)}
                  >
                    保存为文档
                  </button>
                )}
              </article>
            ))}
            {busy && (
              <p className="dock-status" role="status">
                {engine === "local" ? "本地模型思考中…" : "外部模型回复中…"}
              </p>
            )}
          </div>
          <div className="dock-composer">
            <textarea
              ref={inputRef}
              value={draft}
              rows={2}
              placeholder={`${activeMode?.label ?? "对话"}…（⌘↵ 发送）`}
              onChange={(event) => setDraft(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Enter" && (event.metaKey || event.ctrlKey) && !event.nativeEvent.isComposing) {
                  event.preventDefault();
                  void send();
                }
              }}
            />
            <button
              type="button"
              className="primary-button"
              disabled={busy || !draft.trim() || (mode === "summary" && !selectedRecordId)}
              onClick={() => void send()}
            >
              发送
            </button>
          </div>
          {error && <p className="inline-error" role="alert">{error}</p>}
          {notice && <p className="inline-notice" role="status">{notice}</p>}
        </>
      )}
    </section>
  );
}
