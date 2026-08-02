import { open } from "@tauri-apps/plugin-dialog";
import { useEffect, useRef, useState } from "react";
import type {
  KnowledgeAnswer,
  KnowledgeAnswerCitation,
  KnowledgeIndexStatus,
  KnowledgeOverview,
  KnowledgeReference,
} from "../shared/types";
import {
  askKnowledgeBase,
  exportKnowledgeBase,
  getKnowledgeIndexStatus,
  getKnowledgeOverview,
  listProjects,
  rebuildKnowledgeIndex,
} from "../lib/tauri";

interface Props {
  scope: string;
  projectId: string | null;
  unfiledOnly: boolean;
  refreshKey: number;
  onOpenCitation: (citation: KnowledgeAnswerCitation) => void;
}

type HomeTab = "overview" | "ask";

interface ConversationItem {
  question: string;
  answer: KnowledgeAnswer;
}

export default function KnowledgeHome({ scope, projectId, unfiledOnly, refreshKey, onOpenCitation }: Props) {
  const [name, setName] = useState(scope === "all" ? "全部记录" : scope === "unfiled" ? "未归档" : "知识库");
  const [overview, setOverview] = useState<KnowledgeOverview | null>(null);
  const [index, setIndex] = useState<KnowledgeIndexStatus | null>(null);
  const [tab, setTab] = useState<HomeTab>("overview");
  const [question, setQuestion] = useState("");
  const [conversation, setConversation] = useState<ConversationItem[]>([]);
  const [busy, setBusy] = useState(false);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const requestId = useRef(0);
  const answerRequestId = useRef(0);
  const refreshKeyRef = useRef(refreshKey);
  const currentScopeKey = `${scope}:${projectId ?? ""}:${unfiledOnly ? "unfiled" : "all"}`;
  const activeScopeRef = useRef(currentScopeKey);
  activeScopeRef.current = currentScopeKey;

  async function refresh(showLoading = false) {
    const refreshId = ++requestId.current;
    const refreshScopeKey = currentScopeKey;
    if (showLoading) setLoading(true);
    try {
      const [summary, status, projects] = await Promise.all([
        getKnowledgeOverview(projectId, unfiledOnly),
        getKnowledgeIndexStatus(projectId, unfiledOnly),
        listProjects(),
      ]);
      if (refreshId !== requestId.current || refreshScopeKey !== activeScopeRef.current) return;
      setOverview(summary);
      setIndex(status);
      setName(scope === "all" ? "全部记录" : scope === "unfiled" ? "未归档" : projects.find((project) => project.id === scope)?.name ?? "知识库");
    } catch (reason) {
      if (refreshId !== requestId.current || refreshScopeKey !== activeScopeRef.current) return;
      setError(String(reason));
    } finally {
      if (refreshId === requestId.current && refreshScopeKey === activeScopeRef.current) setLoading(false);
    }
  }

  useEffect(() => {
    answerRequestId.current += 1;
    setConversation([]);
    setQuestion("");
    setBusy(false);
    setTab("overview");
    setOverview(null);
    setIndex(null);
    setLoading(true);
    setError("");
    setNotice("");
    void refresh(true);
  }, [currentScopeKey]);

  useEffect(() => {
    if (refreshKeyRef.current === refreshKey) return;
    refreshKeyRef.current = refreshKey;
    void refresh(true);
  }, [refreshKey]);

  useEffect(() => {
    if (index?.status !== "indexing") return;
    let cancelled = false;
    let timer: number | undefined;
    const poll = async () => {
      await refresh();
      if (!cancelled && activeScopeRef.current === currentScopeKey) timer = window.setTimeout(() => void poll(), 1_000);
    };
    timer = window.setTimeout(() => void poll(), 1_000);
    return () => {
      cancelled = true;
      if (timer !== undefined) window.clearTimeout(timer);
    };
  }, [index?.status, currentScopeKey]);

  async function rebuild() {
    const rebuildId = ++requestId.current;
    const rebuildScopeKey = currentScopeKey;
    setBusy(true);
    setError("");
    setNotice("");
    try {
      await rebuildKnowledgeIndex(projectId, unfiledOnly);
      if (rebuildId !== requestId.current || rebuildScopeKey !== activeScopeRef.current) return;
      setIndex((current) => current ? { ...current, status: "indexing", lastError: null } : {
        scopeKey: rebuildScopeKey,
        status: "indexing",
        totalRecords: 0,
        processedRecords: 0,
        chunkCount: 0,
        embeddingModel: "",
        lastError: null,
        updatedAt: new Date().toISOString(),
      });
      setNotice("正在本机建立知识索引");
      window.setTimeout(() => {
        if (rebuildId === requestId.current && rebuildScopeKey === activeScopeRef.current) void refresh();
      }, 400);
    } catch (reason) {
      if (rebuildId !== requestId.current || rebuildScopeKey !== activeScopeRef.current) return;
      setError(String(reason));
    } finally {
      if (rebuildId !== requestId.current || rebuildScopeKey !== activeScopeRef.current) return;
      setBusy(false);
    }
  }

  async function ask() {
    const trimmed = question.trim();
    if (!trimmed) return;
    const askId = ++answerRequestId.current;
    const askScopeKey = currentScopeKey;
    setBusy(true);
    setError("");
    try {
      const answer = await askKnowledgeBase(trimmed, projectId, unfiledOnly);
      if (askId !== answerRequestId.current || askScopeKey !== activeScopeRef.current) return;
      setConversation((items) => [...items, { question: trimmed, answer }]);
      setQuestion("");
    } catch (reason) {
      if (askId !== answerRequestId.current || askScopeKey !== activeScopeRef.current) return;
      setError(String(reason));
    } finally {
      if (askId === answerRequestId.current && askScopeKey === activeScopeRef.current) setBusy(false);
    }
  }

  async function exportAll(format: "md" | "txt") {
    const destination = await open({ directory: true, multiple: false, title: `导出${name}` });
    if (typeof destination !== "string") return;
    setBusy(true);
    setError("");
    try {
      const path = await exportKnowledgeBase(projectId, unfiledOnly, destination, format);
      setNotice(`已导出到 ${path}`);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  const ready = index?.status === "completed";
  return <section className="knowledge-home" aria-label={`${name}主页`}>
    <header className="knowledge-home-header material">
      <div><p className="pane-eyebrow">知识库主页</p><h2>{name}</h2></div>
      <details className="export-menu">
        <summary>导出</summary>
        <div className="export-popover material"><button type="button" onClick={() => void exportAll("md")}>Markdown 文件夹</button><button type="button" onClick={() => void exportAll("txt")}>TXT 文件夹</button></div>
      </details>
    </header>
    <div className="knowledge-home-tabs" role="tablist" aria-label="知识库视图">
      <button id="knowledge-overview-tab" type="button" role="tab" aria-selected={tab === "overview"} aria-controls="knowledge-overview-panel" className={tab === "overview" ? "selected" : ""} onClick={() => setTab("overview")}>概览</button>
      <button id="knowledge-ask-tab" type="button" role="tab" aria-selected={tab === "ask"} aria-controls="knowledge-ask-panel" className={tab === "ask" ? "selected" : ""} onClick={() => setTab("ask")}>问知识库</button>
    </div>
    {error && <p className="detail-error" role="alert">{error}</p>}
    {notice && <p className="inline-notice knowledge-notice" role="status">{notice}</p>}
    <div className="knowledge-home-content">
      {tab === "overview" ? <div id="knowledge-overview-panel" role="tabpanel" aria-labelledby="knowledge-overview-tab">
        {loading ? <p className="knowledge-loading" role="status">正在加载知识库概览…</p> : <div className="knowledge-metrics">
          <Metric label="录音" value={overview?.recordCount ?? 0} />
          <Metric label="逐字稿" value={overview?.transcriptCount ?? 0} />
          <Metric label="已分析" value={overview?.analyzedCount ?? 0} />
        </div>}
        {!loading && <section className="index-section">
          <div><h3>知识索引</h3><p>{indexStatusText(index)}</p></div>
          <button type="button" className={ready ? "secondary-button" : "primary-button"} disabled={busy || index?.status === "indexing"} onClick={() => void rebuild()}>{index?.status === "indexing" ? "建立中…" : ready ? "更新索引" : "建立索引"}</button>
          {index?.status === "indexing" && <div className="index-progress"><span style={{ width: `${index.totalRecords ? Math.round(index.processedRecords / index.totalRecords * 100) : 5}%` }} /></div>}
        </section>}
        {!loading && <>
          <ReferenceSection title="最近决策" empty="还没有可验证的决策。" items={overview?.decisions ?? []} onOpen={(item) => item.segmentId && onOpenCitation({ chunkId: "overview", recordId: item.recordId, recordTitle: item.recordTitle, quoteText: item.quoteText, segmentId: item.segmentId, startMs: item.startMs ?? 0, endMs: item.endMs ?? item.startMs ?? 0 })} />
          <ReferenceSection title="最近待办" empty="还没有明确待办。" items={overview?.actionItems ?? []} onOpen={(item) => item.segmentId && onOpenCitation({ chunkId: "overview", recordId: item.recordId, recordTitle: item.recordTitle, quoteText: item.quoteText, segmentId: item.segmentId, startMs: item.startMs ?? 0, endMs: item.endMs ?? item.startMs ?? 0 })} />
        </>}
      </div> : <div id="knowledge-ask-panel" role="tabpanel" aria-labelledby="knowledge-ask-tab" className="knowledge-chat">
        <div className="conversation">
          {conversation.length === 0 && <div className="chat-empty"><h3>问{name}</h3><p>{ready ? "" : "建立知识索引后即可提问。"}</p></div>}
          {conversation.map((item, index) => <article className="conversation-item" key={`${item.question}-${index}`}><p className="question-bubble">{item.question}</p><div className="answer-block"><p>{item.answer.answer}</p>{item.answer.citations.map((citation) => <button type="button" key={citation.chunkId} onClick={() => onOpenCitation(citation)}><strong>{citation.recordTitle} · {formatTime(citation.startMs)}</strong><span>{citation.quoteText}</span></button>)}</div></article>)}
        </div>
        <form className="ask-form" onSubmit={(event) => { event.preventDefault(); void ask(); }}><textarea value={question} maxLength={500} disabled={!ready || busy} placeholder={ready ? "输入问题" : "知识索引尚未就绪"} aria-label="向知识库提问" onChange={(event) => setQuestion(event.target.value)} onKeyDown={(event) => { if (event.key === "Enter" && !event.shiftKey) { event.preventDefault(); void ask(); } }} /><button type="submit" className="primary-button" disabled={!ready || busy || !question.trim()}>{busy ? "查找中…" : "提问"}</button></form>
      </div>}
    </div>
  </section>;
}

function Metric({ label, value }: { label: string; value: number }) { return <div><strong>{value}</strong><span>{label}</span></div>; }

function indexStatusText(status: KnowledgeIndexStatus | null) {
  if (!status || status.status === "not_built") return "尚未建立";
  if (status.status === "stale") return "录音内容已变化，需要更新";
  if (status.status === "indexing") return `${status.processedRecords} / ${status.totalRecords} 条录音`;
  if (status.status === "failed") return status.lastError ?? "建立失败";
  return `${status.chunkCount} 个知识片段 · ${status.embeddingModel}`;
}

function ReferenceSection({ title, empty, items, onOpen }: { title: string; empty: string; items: KnowledgeReference[]; onOpen: (item: KnowledgeReference) => void }) {
  return <section className="overview-section"><h3>{title}</h3>{items.length ? <div className="overview-reference-list">{items.map((item, index) => <button type="button" disabled={!item.segmentId} key={`${item.recordId}-${index}`} onClick={() => onOpen(item)}><strong>{item.text}</strong><span>{item.recordTitle}{item.startMs !== null ? ` · ${formatTime(item.startMs)}` : ""}</span></button>)}</div> : <p>{empty}</p>}</section>;
}

function formatTime(ms: number) { const seconds = Math.floor(ms / 1000); return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`; }
