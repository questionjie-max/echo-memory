import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import { askKnowledgeBase, getKnowledgeIndexStatus, listProjects, rebuildKnowledgeIndex } from "../lib/tauri";
import type { KnowledgeAnswer, KnowledgeAnswerCitation, KnowledgeIndexStatus } from "../shared/types";
import "./knowledge-chat.css";

export interface KnowledgeChatViewProps {
  scope: string;
  projectId: string | null;
  unfiledOnly: boolean;
  refreshKey: number;
  onOpenCitation: (citation: KnowledgeAnswerCitation) => void;
  onOpenSettings: () => void;
}

type TurnStatus = "pending" | "completed" | "failed";

interface ConversationTurn {
  id: number;
  question: string;
  answer: KnowledgeAnswer | null;
  status: TurnStatus;
  error: string;
}

const MAX_REQUEST_CHARS = 500;
const SUGGESTED_QUESTIONS = [
  "最近有哪些重要会议结论？",
  "有哪些尚未完成的行动项？",
  "围绕当前范围做过哪些关键决策？",
  "帮我梳理重要讨论的时间线。",
];

export default function KnowledgeChatView({ scope, projectId, unfiledOnly, refreshKey, onOpenCitation, onOpenSettings }: KnowledgeChatViewProps) {
  const fallbackScopeName = scope === "all" ? "全部资料" : unfiledOnly || scope === "unfiled" ? "未归档资料" : "当前项目";
  const scopeRequestKey = `${scope}:${projectId ?? "all"}:${unfiledOnly ? "unfiled" : "filed"}`;
  const [scopeName, setScopeName] = useState(fallbackScopeName);
  const [indexStatus, setIndexStatus] = useState<KnowledgeIndexStatus | null>(null);
  const [indexLoading, setIndexLoading] = useState(true);
  const [indexError, setIndexError] = useState("");
  const [rebuilding, setRebuilding] = useState(false);
  const [draft, setDraft] = useState("");
  const [turns, setTurns] = useState<ConversationTurn[]>([]);
  const [pendingTurnId, setPendingTurnId] = useState<number | null>(null);
  const activeScopeRef = useRef(scopeRequestKey);
  const metadataRequestRef = useRef(0);
  const answerRequestRef = useRef(0);
  const refreshKeyRef = useRef(refreshKey);
  const turnIdRef = useRef(0);
  const conversationEndRef = useRef<HTMLDivElement | null>(null);
  activeScopeRef.current = scopeRequestKey;

  async function loadScopeData(includeProjects: boolean, showLoading: boolean) {
    const targetScope = scopeRequestKey;
    const requestId = ++metadataRequestRef.current;
    if (showLoading) setIndexLoading(true);
    setIndexError("");
    const [statusResult, projectsResult] = await Promise.allSettled([
      getKnowledgeIndexStatus(projectId, unfiledOnly),
      includeProjects ? listProjects() : Promise.resolve(null),
    ]);
    if (activeScopeRef.current !== targetScope || metadataRequestRef.current !== requestId) return;
    if (statusResult.status === "fulfilled") setIndexStatus(statusResult.value);
    else setIndexError(toErrorMessage(statusResult.reason));
    if (projectsResult.status === "fulfilled" && projectsResult.value) {
      const project = projectId ? projectsResult.value.find((item) => item.id === projectId) : null;
      setScopeName(project?.name ?? fallbackScopeName);
    }
    setIndexLoading(false);
  }

  useEffect(() => {
    answerRequestRef.current += 1;
    setScopeName(fallbackScopeName);
    setIndexStatus(null);
    setIndexError("");
    setTurns([]);
    setDraft("");
    setPendingTurnId(null);
    void loadScopeData(true, true);
  }, [scopeRequestKey]);

  useEffect(() => {
    if (refreshKeyRef.current === refreshKey) return;
    refreshKeyRef.current = refreshKey;
    void loadScopeData(true, true);
  }, [refreshKey]);

  useEffect(() => {
    if (indexStatus?.status !== "indexing") return;
    let cancelled = false;
    let timer: number | undefined;
    const poll = async () => {
      await loadScopeData(false, false);
      if (!cancelled && activeScopeRef.current === scopeRequestKey) timer = window.setTimeout(() => void poll(), 1_000);
    };
    timer = window.setTimeout(() => void poll(), 1_000);
    return () => {
      cancelled = true;
      if (timer !== undefined) window.clearTimeout(timer);
    };
  }, [indexStatus?.status, scopeRequestKey]);

  useEffect(() => {
    conversationEndRef.current?.scrollIntoView({ behavior: "smooth", block: "end" });
  }, [turns, pendingTurnId]);

  const indexReady = indexStatus?.status === "completed" && indexStatus.chunkCount > 0;
  const inputDisabled = !indexReady || pendingTurnId !== null;

  async function handleRebuildIndex() {
    if (rebuilding || indexStatus?.status === "indexing") return;
    const targetScope = scopeRequestKey;
    setRebuilding(true);
    setIndexError("");
    try {
      await rebuildKnowledgeIndex(projectId, unfiledOnly);
      if (activeScopeRef.current !== targetScope) return;
      setIndexStatus((current) => current ? { ...current, status: "indexing", processedRecords: 0, lastError: null } : current);
      window.setTimeout(() => void loadScopeData(false, false), 400);
    } catch (reason) {
      if (activeScopeRef.current === targetScope) setIndexError(toErrorMessage(reason));
    } finally {
      if (activeScopeRef.current === targetScope) setRebuilding(false);
    }
  }

  function handleClearConversation() {
    answerRequestRef.current += 1;
    setTurns([]);
    setDraft("");
    setPendingTurnId(null);
  }

  async function submitQuestion(question: string, retryTurnId?: number) {
    const trimmed = question.trim();
    if (!trimmed || !indexReady || pendingTurnId !== null) return;
    const targetScope = scopeRequestKey;
    const requestId = ++answerRequestRef.current;
    const turnId = retryTurnId ?? ++turnIdRef.current;
    const previousTurns = turns.filter((turn) => turn.id < turnId && turn.status === "completed" && turn.answer);
    const requestQuestion = buildContextualQuestion(trimmed, previousTurns);

    if (retryTurnId === undefined) {
      setTurns((items) => [...items, { id: turnId, question: trimmed, answer: null, status: "pending", error: "" }]);
      setDraft("");
    } else {
      setTurns((items) => items.map((turn) => turn.id === turnId ? { ...turn, answer: null, status: "pending", error: "" } : turn));
    }
    setPendingTurnId(turnId);

    try {
      const answer = await askKnowledgeBase(requestQuestion, projectId, unfiledOnly);
      if (activeScopeRef.current !== targetScope || answerRequestRef.current !== requestId) return;
      setTurns((items) => items.map((turn) => turn.id === turnId ? { ...turn, answer, status: "completed", error: "" } : turn));
    } catch (reason) {
      if (activeScopeRef.current !== targetScope || answerRequestRef.current !== requestId) return;
      setTurns((items) => items.map((turn) => turn.id === turnId ? { ...turn, status: "failed", error: toErrorMessage(reason) } : turn));
    } finally {
      if (activeScopeRef.current === targetScope && answerRequestRef.current === requestId) setPendingTurnId(null);
    }
  }

  function handleComposerKeyDown(event: KeyboardEvent<HTMLTextAreaElement>) {
    if (event.key !== "Enter" || event.shiftKey || event.nativeEvent.isComposing) return;
    event.preventDefault();
    void submitQuestion(draft);
  }

  return <section className="knowledge-chat-view" aria-label={`与${scopeName}对话`}>
    <header className="knowledge-chat-header view-header">
      <div className="knowledge-chat-title-group">
        <p className="knowledge-chat-eyebrow">问知识库</p>
        <h2>问你的知识库</h2>
        <p>{scopeDescription(scopeName, scope, unfiledOnly)}</p>
      </div>
      <div className="knowledge-chat-header-actions">
        {turns.length > 0 && <button type="button" className="knowledge-chat-quiet-button" onClick={handleClearConversation}>清空对话</button>}
        <button type="button" className="knowledge-chat-quiet-button" onClick={onOpenSettings}>AI 设置</button>
      </div>
    </header>

    <IndexStatusBar status={indexStatus} loading={indexLoading} rebuilding={rebuilding} error={indexError} onRetry={() => void loadScopeData(true, true)} onRebuild={() => void handleRebuildIndex()} />

    <div className={`knowledge-chat-main ${indexReady ? "ready" : ""}`}>
      {indexLoading && !indexStatus ? <LoadingState /> : indexError && !indexStatus ? <MetadataErrorState error={indexError} onRetry={() => void loadScopeData(true, true)} onOpenSettings={onOpenSettings} /> : !indexReady ? <IndexGuide status={indexStatus} rebuilding={rebuilding} onRebuild={() => void handleRebuildIndex()} onOpenSettings={onOpenSettings} /> : <>
        <div className="knowledge-chat-conversation" aria-live="polite" aria-busy={pendingTurnId !== null}>
          {turns.length === 0 ? <ChatEmptyState scopeName={scopeName} onAsk={(question) => void submitQuestion(question)} /> : turns.map((turn) => <ConversationTurnView key={turn.id} turn={turn} onRetry={() => void submitQuestion(turn.question, turn.id)} onOpenCitation={onOpenCitation} />)}
          <div ref={conversationEndRef} />
        </div>
        <form className="knowledge-chat-composer" onSubmit={(event) => { event.preventDefault(); void submitQuestion(draft); }}>
          <label htmlFor="knowledge-chat-question">向 {scopeName} 提问</label>
          <div className="knowledge-chat-input-row">
            <textarea id="knowledge-chat-question" value={draft} maxLength={MAX_REQUEST_CHARS} rows={2} disabled={inputDisabled} placeholder="例如：上次讨论这个问题时，最终结论是什么？" onChange={(event) => setDraft(event.target.value)} onKeyDown={handleComposerKeyDown} />
            <button type="submit" className="knowledge-chat-send-button" disabled={inputDisabled || draft.trim().length < 2}>{pendingTurnId !== null ? "处理中" : "发送"}</button>
          </div>
          <div className="knowledge-chat-composer-meta"><span>Enter 发送，Shift + Enter 换行</span><span>{characterCount(draft)}/{MAX_REQUEST_CHARS}</span></div>
        </form>
      </>}
    </div>
  </section>;
}

function IndexStatusBar({ status, loading, rebuilding, error, onRetry, onRebuild }: { status: KnowledgeIndexStatus | null; loading: boolean; rebuilding: boolean; error: string; onRetry: () => void; onRebuild: () => void }) {
  const tone = status?.status ?? (loading ? "indexing" : "failed");
  return <div className="knowledge-chat-index-bar" role="status">
    <span className={`knowledge-chat-status-dot ${tone}`} aria-hidden="true" />
    <div className="knowledge-chat-index-copy"><strong>{indexStatusTitle(status, loading)}</strong><span>{indexStatusDetail(status, loading)}</span></div>
    {status?.status === "indexing" && <div className="knowledge-chat-progress" aria-label={indexStatusDetail(status, false)}><span style={{ width: `${indexProgress(status)}%` }} /></div>}
    {error && <span className="knowledge-chat-index-error" title={error}>状态更新失败</span>}
    {error && <button type="button" className="knowledge-chat-inline-button" onClick={onRetry}>重试</button>}
    {status && ["not_built", "stale", "failed"].includes(status.status) && <button type="button" className="knowledge-chat-inline-button" disabled={rebuilding} onClick={onRebuild}>{rebuilding ? "正在启动" : status.status === "stale" ? "更新索引" : "建立索引"}</button>}
  </div>;
}

function LoadingState() {
  return <div className="knowledge-chat-centered-state" aria-live="polite"><span className="knowledge-chat-loader" aria-hidden="true" /><h3>正在读取知识库状态</h3><p>正在确认当前范围是否可以开始对话。</p></div>;
}

function MetadataErrorState({ error, onRetry, onOpenSettings }: { error: string; onRetry: () => void; onOpenSettings: () => void }) {
  return <div className="knowledge-chat-centered-state knowledge-chat-error-state"><span className="knowledge-chat-state-mark" aria-hidden="true">!</span><h3>暂时无法读取知识库</h3><p>{error}</p><div className="knowledge-chat-state-actions"><button type="button" className="knowledge-chat-primary-button" onClick={onRetry}>重新加载</button><button type="button" className="knowledge-chat-secondary-button" onClick={onOpenSettings}>检查 AI 设置</button></div></div>;
}

function IndexGuide({ status, rebuilding, onRebuild, onOpenSettings }: { status: KnowledgeIndexStatus | null; rebuilding: boolean; onRebuild: () => void; onOpenSettings: () => void }) {
  const indexing = status?.status === "indexing";
  const emptyCompletedIndex = status?.status === "completed" && status.chunkCount === 0;
  return <div className="knowledge-chat-centered-state knowledge-chat-index-guide">
    <span className="knowledge-chat-state-mark" aria-hidden="true">AI</span>
    <h3>{indexing ? "正在建立知识索引" : emptyCompletedIndex ? "当前范围没有可检索内容" : status?.status === "failed" ? "知识索引建立失败" : "建立索引后即可开始对话"}</h3>
    <p>{indexing ? `${status.processedRecords}/${status.totalRecords} 条内容已处理，完成后会自动开放对话。` : emptyCompletedIndex ? "请先导入包含文字内容的录音或文档，随后更新索引。" : status?.lastError || "索引会在本机整理知识内容，让 AI 能够检索原文并提供可验证引用。"}</p>
    <div className="knowledge-chat-state-actions">{!emptyCompletedIndex && <button type="button" className="knowledge-chat-primary-button" disabled={indexing || rebuilding} onClick={onRebuild}>{indexing ? "建立中" : rebuilding ? "正在启动" : status?.status === "stale" ? "更新索引" : "建立索引"}</button>}<button type="button" className="knowledge-chat-secondary-button" onClick={onOpenSettings}>检查 AI 设置</button></div>
    <small>索引与问答均在当前配置的本地 AI 环境中运行。</small>
  </div>;
}

function ChatEmptyState({ scopeName, onAsk }: { scopeName: string; onAsk: (question: string) => void }) {
  return <div className="knowledge-chat-empty"><div className="knowledge-chat-empty-copy"><span className="knowledge-chat-state-mark" aria-hidden="true">AI</span><h3>从 {scopeName} 中查找答案</h3><p>我会先检索相关原文，再给出回答和可打开的引用。答案不足时会明确说明，不会用猜测代替证据。</p></div><div className="knowledge-chat-suggestions" aria-label="建议问题">{SUGGESTED_QUESTIONS.map((question) => <button type="button" key={question} onClick={() => onAsk(question)}>{question}</button>)}</div></div>;
}

function ConversationTurnView({ turn, onRetry, onOpenCitation }: { turn: ConversationTurn; onRetry: () => void; onOpenCitation: (citation: KnowledgeAnswerCitation) => void }) {
  return <article className="knowledge-chat-turn">
    <div className="knowledge-chat-user-message"><span>你</span><p>{turn.question}</p></div>
    <div className={`knowledge-chat-assistant-message ${turn.status}`}><span className="knowledge-chat-assistant-label">AI</span>
      {turn.status === "pending" && <div className="knowledge-chat-thinking"><span className="knowledge-chat-loader" aria-hidden="true" /><p>正在检索知识库并整理答案…</p></div>}
      {turn.status === "failed" && <div className="knowledge-chat-turn-error"><strong>本次回答失败</strong><p>{turn.error}</p><button type="button" onClick={onRetry}>重试这个问题</button></div>}
      {turn.status === "completed" && turn.answer && <><div className={`knowledge-chat-answer ${turn.answer.insufficientEvidence ? "insufficient" : ""}`}>{turn.answer.insufficientEvidence && <strong>现有资料不足以确认</strong>}<p>{turn.answer.answer}</p></div>{turn.answer.citations.length > 0 && <div className="knowledge-chat-citations"><h4>回答依据</h4><div>{turn.answer.citations.map((citation, index) => <button type="button" key={`${citation.chunkId}-${index}`} onClick={() => onOpenCitation(citation)}><span className="knowledge-chat-citation-index">{index + 1}</span><span className="knowledge-chat-citation-copy"><strong>{citation.recordTitle}</strong><small>{citation.quoteText}</small></span><span className="knowledge-chat-citation-action">查看原文</span></button>)}</div></div>}</>}
    </div>
  </article>;
}

function scopeDescription(scopeName: string, scope: string, unfiledOnly: boolean) {
  if (scope === "all") return "从全部已建立索引的录音与文档中检索，并用原文引用回答。";
  if (unfiledOnly || scope === "unfiled") return "仅从未归档的录音与文档中检索，不会混入项目资料。";
  return `仅从“${scopeName}”中的录音与文档检索，回答不会跨越当前项目。`;
}

function indexStatusTitle(status: KnowledgeIndexStatus | null, loading: boolean) {
  if (loading && !status) return "正在检查索引";
  if (!status) return "索引状态未知";
  return { not_built: "知识索引未建立", stale: "知识索引需要更新", indexing: "正在建立知识索引", completed: status.chunkCount > 0 ? "知识索引已就绪" : "索引中暂无内容", failed: "知识索引建立失败" }[status.status];
}

function indexStatusDetail(status: KnowledgeIndexStatus | null, loading: boolean) {
  if (loading && !status) return "正在读取当前范围";
  if (!status) return "请重新加载索引状态";
  if (status.status === "indexing") return `${status.processedRecords}/${status.totalRecords} 条内容已处理`;
  if (status.status === "completed") return `${status.chunkCount} 个知识片段 · ${status.totalRecords} 条内容`;
  if (status.status === "failed") return status.lastError || "请重试或检查本地 AI 设置";
  if (status.status === "stale") return `${status.totalRecords} 条内容发生变化，更新后可继续提问`;
  return "建立后即可基于当前范围进行问答";
}

function indexProgress(status: KnowledgeIndexStatus) {
  if (status.totalRecords <= 0) return 8;
  return Math.max(4, Math.min(100, Math.round(status.processedRecords / status.totalRecords * 100)));
}

function buildContextualQuestion(question: string, turns: ConversationTurn[]) {
  const currentQuestion = takeCharacters(question.trim(), MAX_REQUEST_CHARS);
  const currentBlock = `当前问题：${currentQuestion}`;
  const contextHeader = "最近对话：\n";
  const remaining = MAX_REQUEST_CHARS - characterCount(currentBlock) - characterCount(contextHeader) - 1;
  if (remaining < 24) return currentQuestion;
  const selectedLines: string[] = [];
  let used = 0;
  for (const turn of turns.slice(-4).reverse()) {
    if (!turn.answer) continue;
    const line = `问：${takeCharacters(turn.question.replace(/\s+/g, " "), 54)} 答：${takeCharacters(turn.answer.answer.replace(/\s+/g, " "), 90)}`;
    const lineLength = characterCount(line) + (selectedLines.length > 0 ? 1 : 0);
    if (used + lineLength > remaining) continue;
    selectedLines.unshift(line);
    used += lineLength;
  }
  if (selectedLines.length === 0) return currentQuestion;
  return `${contextHeader}${selectedLines.join("\n")}\n${currentBlock}`;
}

function characterCount(value: string) { return Array.from(value).length; }
function takeCharacters(value: string, limit: number) { return Array.from(value).slice(0, limit).join(""); }
function toErrorMessage(reason: unknown) {
  const message = reason instanceof Error ? reason.message : String(reason);
  return message.replace(/^Error:\s*/i, "") || "发生未知错误，请稍后重试。";
}
