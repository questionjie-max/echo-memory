import { open } from "@tauri-apps/plugin-dialog";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useRef, useState } from "react";
import type {
  KnowledgeAnswerCitation,
  KnowledgeIndexStatus,
  KnowledgeOverview,
  KnowledgeReference,
} from "../shared/types";
import {
  createExportTicket,
  exportKnowledgeBase,
  getKnowledgeIndexStatus,
  getKnowledgeOverview,
  listProjects,
  rebuildKnowledgeIndex,
} from "../lib/tauri";
import { formatMinutesSeconds as formatTime } from "../lib/format";

interface Props {
  scope: string;
  projectId: string | null;
  unfiledOnly: boolean;
  refreshKey: number;
  onOpenCitation: (citation: KnowledgeAnswerCitation) => void;
  /** 提问入口收敛到「问知识库」视图一处：主页只留一个跳转，不再各做一套问答界面。 */
  onOpenKnowledgeChat: () => void;
}

export default function KnowledgeHome({ scope, projectId, unfiledOnly, refreshKey, onOpenCitation, onOpenKnowledgeChat }: Props) {
  const [name, setName] = useState(scope === "all" ? "全部记录" : scope === "unfiled" ? "未归档" : "知识库");
  const [overview, setOverview] = useState<KnowledgeOverview | null>(null);
  const [index, setIndex] = useState<KnowledgeIndexStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const requestId = useRef(0);
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

  useEffect(() => {
    const stop = listen("knowledge-index-update", () => void refresh());
    return () => {
      void stop.then((unlisten) => unlisten());
    };
  }, [currentScopeKey]);

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

  async function exportAll(format: "md" | "txt") {
    // 先领票据再弹目录选择对话框——后端凭票放行导出。
    const ticket = await createExportTicket();
    const destination = await open({ directory: true, multiple: false, title: `导出${name}` });
    if (typeof destination !== "string") return;
    setBusy(true);
    setError("");
    try {
      const path = await exportKnowledgeBase(projectId, unfiledOnly, destination, format, ticket);
      setNotice(`已导出到 ${path}`);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  const ready = index?.status === "completed";
  return <section className="knowledge-home" aria-label={`${name}主页`}>
    <header className="knowledge-home-header view-header">
      <div><p className="pane-eyebrow">知识库主页</p><h2>{name}</h2></div>
      <div className="knowledge-home-actions">
        <button type="button" className="primary-button" onClick={onOpenKnowledgeChat}>问知识库 →</button>
        <details className="export-menu">
          <summary>导出</summary>
          <div className="export-popover"><button type="button" onClick={() => void exportAll("md")}>Markdown 文件夹</button><button type="button" onClick={() => void exportAll("txt")}>TXT 文件夹</button></div>
        </details>
      </div>
    </header>
    <div className="knowledge-home-content">
      {error && <p className="detail-error" role="alert">{error}</p>}
      {notice && <p className="inline-notice knowledge-notice" role="status">{notice}</p>}
      {loading ? <p className="knowledge-loading" role="status">正在加载知识库概览…</p> : <div className="knowledge-metrics">
        <Metric label="录音" value={overview?.recordCount ?? 0} />
        <Metric label="逐字稿" value={overview?.transcriptCount ?? 0} />
        <Metric label="已分析" value={overview?.analyzedCount ?? 0} />
      </div>}
      {!loading && <section className="index-section">
        <div><h3>知识索引</h3><p>{indexStatusText(index)}{!ready && " · 建好后就能在「问知识库」里提问"}</p></div>
        <button type="button" className={ready ? "secondary-button" : "primary-button"} disabled={busy || index?.status === "indexing"} onClick={() => void rebuild()}>{index?.status === "indexing" ? "建立中…" : ready ? "更新索引" : "建立索引"}</button>
        {index?.status === "indexing" && <div className="index-progress"><span style={{ width: `${index.totalRecords ? Math.round(index.processedRecords / index.totalRecords * 100) : 5}%` }} /></div>}
      </section>}
      {!loading && <>
        <ReferenceSection title="最近决策" empty="还没有可验证的决策。" items={overview?.decisions ?? []} onOpen={(item) => item.segmentId && onOpenCitation({ chunkId: "overview", recordId: item.recordId, recordTitle: item.recordTitle, quoteText: item.quoteText, segmentId: item.segmentId, startMs: item.startMs ?? 0, endMs: item.endMs ?? item.startMs ?? 0 })} />
        <ReferenceSection title="最近待办" empty="还没有明确待办。" items={overview?.actionItems ?? []} onOpen={(item) => item.segmentId && onOpenCitation({ chunkId: "overview", recordId: item.recordId, recordTitle: item.recordTitle, quoteText: item.quoteText, segmentId: item.segmentId, startMs: item.startMs ?? 0, endMs: item.endMs ?? item.startMs ?? 0 })} />
      </>}
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
