import { open } from "@tauri-apps/plugin-dialog";
import { useEffect, useState } from "react";
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
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");

  async function refresh() {
    try {
      const [summary, status, projects] = await Promise.all([
        getKnowledgeOverview(projectId, unfiledOnly),
        getKnowledgeIndexStatus(projectId, unfiledOnly),
        listProjects(),
      ]);
      setOverview(summary);
      setIndex(status);
      setName(scope === "all" ? "全部记录" : scope === "unfiled" ? "未归档" : projects.find((project) => project.id === scope)?.name ?? "知识库");
    } catch (reason) {
      setError(String(reason));
    }
  }

  useEffect(() => {
    setConversation([]);
    setTab("overview");
    setError("");
    setNotice("");
    void refresh();
  }, [scope, refreshKey]);

  useEffect(() => {
    if (index?.status !== "indexing") return;
    const timer = window.setInterval(() => void refresh(), 1_000);
    return () => window.clearInterval(timer);
  }, [index?.status, scope]);

  async function rebuild() {
    setBusy(true);
    setError("");
    setNotice("");
    try {
      await rebuildKnowledgeIndex(projectId, unfiledOnly);
      setIndex((current) => current ? { ...current, status: "indexing", lastError: null } : current);
      setNotice("正在本机建立知识索引");
      window.setTimeout(() => void refresh(), 400);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  async function ask() {
    const trimmed = question.trim();
    if (!trimmed) return;
    setBusy(true);
    setError("");
    try {
      const answer = await askKnowledgeBase(trimmed, projectId, unfiledOnly);
      setConversation((items) => [...items, { question: trimmed, answer }]);
      setQuestion("");
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
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
    <div className="knowledge-home-tabs" role="tablist">
      <button type="button" className={tab === "overview" ? "selected" : ""} onClick={() => setTab("overview")}>概览</button>
      <button type="button" className={tab === "ask" ? "selected" : ""} onClick={() => setTab("ask")}>问知识库</button>
    </div>
    {error && <p className="detail-error">{error}</p>}
    {notice && <p className="inline-notice knowledge-notice">{notice}</p>}
    <div className="knowledge-home-content">
      {tab === "overview" ? <>
        <div className="knowledge-metrics">
          <Metric label="录音" value={overview?.recordCount ?? 0} />
          <Metric label="逐字稿" value={overview?.transcriptCount ?? 0} />
          <Metric label="已分析" value={overview?.analyzedCount ?? 0} />
        </div>
        <section className="index-section">
          <div><h3>知识索引</h3><p>{indexStatusText(index)}</p></div>
          <button type="button" className={ready ? "secondary-button" : "primary-button"} disabled={busy || index?.status === "indexing"} onClick={() => void rebuild()}>{index?.status === "indexing" ? "建立中…" : ready ? "更新索引" : "建立索引"}</button>
          {index?.status === "indexing" && <div className="index-progress"><span style={{ width: `${index.totalRecords ? Math.round(index.processedRecords / index.totalRecords * 100) : 5}%` }} /></div>}
        </section>
        <ReferenceSection title="最近决策" empty="还没有可验证的决策。" items={overview?.decisions ?? []} onOpen={(item) => item.segmentId && onOpenCitation({ chunkId: "overview", recordId: item.recordId, recordTitle: item.recordTitle, quoteText: item.quoteText, segmentId: item.segmentId, startMs: item.startMs ?? 0, endMs: item.endMs ?? item.startMs ?? 0 })} />
        <ReferenceSection title="最近待办" empty="还没有明确待办。" items={overview?.actionItems ?? []} onOpen={(item) => item.segmentId && onOpenCitation({ chunkId: "overview", recordId: item.recordId, recordTitle: item.recordTitle, quoteText: item.quoteText, segmentId: item.segmentId, startMs: item.startMs ?? 0, endMs: item.endMs ?? item.startMs ?? 0 })} />
      </> : <div className="knowledge-chat">
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
