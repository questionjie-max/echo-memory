import { open } from "@tauri-apps/plugin-dialog";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useMemo, useState } from "react";
import type { InboxStatus, Project, RecordBrief, RecordStatus } from "../shared/types";
import { importAudio, listRecords, transcribeRecord, listProjects, moveRecords, deleteRecords } from "../lib/tauri";
import { formatMinutesSeconds as formatDuration, isProcessingStatus as isProcessing } from "../lib/format";
import { getInboxStatus } from "../lib/tauri";
import DocumentImportDialog from "./DocumentImportDialog";
import { PlayIcon, InboxIcon } from "./icons";

interface Props {
  projectId: string | null;
  unfiledOnly: boolean;
  onImported: () => void;
  selectedId: string | null;
  onSelect: (record: RecordBrief | null) => void;
  /** 播放钮：选中该记录并立即开始播放（仅音频记录显示播放钮）。 */
  onPlay: (record: RecordBrief) => void;
}

type WorkspaceView = "pending" | "recent";

const STATUS_LABEL: Record<RecordStatus, string> = {
  queued: "等待转写",
  preparing: "准备中",
  transcribing: "转写中",
  analyzing: "分析中",
  completed: "已处理",
  failed: "转写失败",
};

export default function RecordPanel({ projectId, unfiledOnly, onImported, selectedId, onSelect, onPlay }: Props) {
  const [records, setRecords] = useState<RecordBrief[]>([]);
  const [view, setView] = useState<WorkspaceView>("pending");
  const [inbox, setInbox] = useState<InboxStatus | null>(null);
  const [selected, setSelected] = useState<Set<string>>(() => new Set());
  const [projects, setProjects] = useState<Project[]>([]);
  const [moveTarget, setMoveTarget] = useState("");
  const [bulkBusy, setBulkBusy] = useState(false);

  useEffect(() => {
    let cancelled = false;
    const load = async () => {
      try {
        const status = await getInboxStatus();
        if (!cancelled) setInbox(status);
      } catch {
        if (!cancelled) setInbox(null);
      }
    };
    void load();
    const stopInbox = listen("inbox-update", () => void load());
    const stopProcessing = listen<{ recordId: string }>(
      "processing-progress",
      () => void refresh(),
    );
    return () => {
      cancelled = true;
      void stopInbox.then((unlisten) => unlisten());
      void stopProcessing.then((unlisten) => unlisten());
    };
  }, []);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [documentImportOpen, setDocumentImportOpen] = useState(false);
  const [documentImportChanged, setDocumentImportChanged] = useState(false);

  async function refresh() {
    try {
      setRecords(await listRecords(projectId, unfiledOnly));
    } catch (reason) {
      setError(String(reason));
    }
  }

  // 勾选状态跟着列表走：列表刷新后，已经不存在的记录要自动从勾选里去掉，
  // 否则批量操作会拿着一堆过期 id 去找后端。
  useEffect(() => {
    setSelected((current) => {
      if (current.size === 0) return current;
      const alive = new Set(records.map((record) => record.id));
      const next = new Set([...current].filter((id) => alive.has(id)));
      return next.size === current.size ? current : next;
    });
  }, [records]);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const list = await listProjects();
        if (!cancelled) setProjects(list.filter((project) => project.status === "active"));
      } catch {
        if (!cancelled) setProjects([]);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    // 切换知识库范围时重置面板状态：视图回到「待处理」、清掉勾选和提示，
    // 相当于旧版靠 key 重挂载做到的事，但不会连带清掉批量操作的结果提示。
    setView("pending");
    setSelected(new Set());
    setMoveTarget("");
    setError("");
    setNotice("");
    void refresh();
    if (!isTauri()) return;

    const unlisten = getCurrentWebviewWindow().onDragDropEvent((event) => {
      if (event.payload.type === "drop") void ingestPaths(event.payload.paths);
    }).catch((reason) => {
      setError(`无法启用拖拽导入：${String(reason)}`);
      return () => undefined;
    });
    return () => {
      void unlisten.then((stop) => stop());
    };
  }, [projectId, unfiledOnly]);

  useEffect(() => {
    if (!records.some((record) => isProcessing(record.status))) return;
    const timer = window.setInterval(() => void refresh(), 2_000);
    return () => window.clearInterval(timer);
  }, [records]);

  async function ingestPaths(paths: string[]) {
    for (const sourcePath of paths) await ingestOne(sourcePath);
  }

  async function ingestOne(sourcePath: string) {
    setBusy(true);
    setError("");
    setNotice("");
    try {
      let result = await importAudio({ sourcePath, projectId, duplicateConfirmed: false });
      if (result.duplicate) {
        const createCopy = window.confirm(`该音频已导入为“${result.title}”。是否创建一份独立副本？`);
        if (!createCopy) {
          setNotice("已保留原有录音，未创建副本。");
          return;
        }
        result = await importAudio({ sourcePath, projectId, duplicateConfirmed: true });
      }
      setNotice(`已导入“${result.title}”，正在本机准备转写。`);
      await refresh();
      await transcribeRecord(result.recordId);
      await refresh();
      onImported();
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  async function chooseFiles() {
    const selection = await open({
      multiple: true,
      directory: false,
      filters: [{ name: "音频", extensions: ["mp3", "m4a", "wav"] }],
    });
    if (!selection) return;
    await ingestPaths(Array.isArray(selection) ? selection : [selection]);
  }

  const pending = useMemo(() => records.filter(isPending), [records]);
  const visible = view === "pending" ? pending : records;

  const selectedIds = useMemo(
    () => visible.filter((record) => selected.has(record.id)).map((record) => record.id),
    [visible, selected],
  );
  const allSelected = visible.length > 0 && selectedIds.length === visible.length;
  const someSelected = selectedIds.length > 0;

  function toggleOne(recordId: string) {
    setSelected((current) => {
      const next = new Set(current);
      if (next.has(recordId)) next.delete(recordId);
      else next.add(recordId);
      return next;
    });
  }

  function toggleAll() {
    setSelected((current) => {
      const next = new Set(current);
      if (allSelected) visible.forEach((record) => next.delete(record.id));
      else visible.forEach((record) => next.add(record.id));
      return next;
    });
  }

  async function applyMove() {
    if (selectedIds.length === 0 || bulkBusy) return;
    setBulkBusy(true);
    setError("");
    setNotice("");
    try {
      const moved = await moveRecords(selectedIds, moveTarget || null);
      const target = moveTarget
        ? projects.find((project) => project.id === moveTarget)?.name ?? "知识库"
        : "未归档";
      setNotice(`已将 ${moved} 条记录移动到「${target}」。`);
      setSelected(new Set());
      await refresh();
      onImported();
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setBulkBusy(false);
    }
  }

  async function applyDelete() {
    if (selectedIds.length === 0 || bulkBusy) return;
    const confirmed = window.confirm(
      `确定删除选中的 ${selectedIds.length} 条记录吗？\n\n逐字稿、分析结果和知识索引会一起删除，原始音频文件也会从本机移除，且无法恢复。`,
    );
    if (!confirmed) return;
    setBulkBusy(true);
    setError("");
    setNotice("");
    try {
      const result = await deleteRecords(selectedIds);
      setNotice(
        result.fileCleanupFailures.length > 0
          ? `已删除 ${result.deletedCount} 条记录，但有 ${result.fileCleanupFailures.length} 个文件遗留。`
          : `已删除 ${result.deletedCount} 条记录及其音频文件。`,
      );
      setSelected(new Set());
      await refresh();
      onImported();
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
      await refresh();
    } finally {
      setBulkBusy(false);
    }
  }

  return (
    <section className="record-pane" aria-label="工作区">
      <header className="pane-heading workspace-heading">
        <div>
          <p className="pane-eyebrow">资料处理</p>
          <h2>工作区</h2>
        </div>
        <label className="select-all-toggle" title="全选或取消全选当前列表">
          <input
            type="checkbox"
            checked={allSelected}
            disabled={visible.length === 0}
            onChange={toggleAll}
            aria-label="全选当前列表"
          />
          <span>全选</span>
        </label>
        <span className="count-badge">{visible.length}</span>
      </header>

      {inbox && (inbox.watchFolders.length > 0 || inbox.usbDetection) && (
        <p className="inbox-strip" role="status">
          <InboxIcon size={13} /> 收件箱监听中
          {inbox.counts.pending > 0 ? ` · ${inbox.counts.pending} 个文件处理中` : ""}
          {inbox.counts.imported > 0 ? ` · 已自动导入 ${inbox.counts.imported}` : ""}
          {inbox.counts.failed > 0 ? ` · ${inbox.counts.failed} 个失败` : ""}
        </p>
      )}

      <div className="segmented-control" aria-label="工作区视图">
        <button type="button" className={view === "pending" ? "selected" : ""} onClick={() => setView("pending")}>待处理 <span>{pending.length}</span></button>
        <button type="button" className={view === "recent" ? "selected" : ""} onClick={() => setView("recent")}>最近 <span>{records.length}</span></button>
      </div>

      <div className="import-actions-grid">
        <div className="import-strip">
          <div>
            <strong>导入录音</strong>
            <span>MP3、M4A、WAV，也可拖入窗口</span>
          </div>
          <button type="button" className="primary-button" onClick={() => void chooseFiles()} disabled={busy}>
            {busy ? "处理中…" : "选择音频"}
          </button>
        </div>
        <div className="import-strip document-import-entry">
          <div>
            <strong>导入文档</strong>
            <span>Markdown、TXT、Word 文档</span>
          </div>
          <button type="button" className="secondary-button" onClick={() => { setDocumentImportChanged(false); setDocumentImportOpen(true); }} disabled={busy}>
            选择文档
          </button>
        </div>
      </div>

      {someSelected && (
        <div className="bulk-bar" role="group" aria-label="批量操作">
          <span className="bulk-count">已选 {selectedIds.length} 条</span>
          <label className="bulk-move">
            <span>移动到</span>
            <select
              value={moveTarget}
              onChange={(event) => setMoveTarget(event.target.value)}
              disabled={bulkBusy}
              aria-label="选择目标知识库"
            >
              <option value="">未归档</option>
              {projects.map((project) => (
                <option key={project.id} value={project.id}>{project.name}</option>
              ))}
            </select>
          </label>
          <button type="button" className="secondary-button" onClick={() => void applyMove()} disabled={bulkBusy}>
            {bulkBusy ? "处理中…" : "移动"}
          </button>
          <button type="button" className="danger-button" onClick={() => void applyDelete()} disabled={bulkBusy}>
            删除
          </button>
          <button type="button" className="link-button" onClick={() => setSelected(new Set())} disabled={bulkBusy}>
            取消选择
          </button>
        </div>
      )}

      <ul className="record-list" aria-live="polite">
        {visible.map((record) => {
          const label = statusLabel(record);
          return (
            <li key={record.id} className={selectedId === record.id ? "selected" : ""}>
              <label className="record-check" onClick={(event) => event.stopPropagation()}>
                <input
                  type="checkbox"
                  checked={selected.has(record.id)}
                  onChange={() => toggleOne(record.id)}
                  aria-label={`选择 ${record.title}`}
                />
              </label>
              {record.sourceType !== "document" && (
                <button
                  type="button"
                  className="record-play"
                  aria-label={`播放 ${record.title}`}
                  title={`播放 ${record.title}`}
                  onClick={() => onPlay(record)}
                >
                  <PlayIcon size={13} />
                </button>
              )}
              <button type="button" className="record-main" onClick={() => onSelect(record)}>
                <strong>{record.title}</strong>
                <span>{new Date(record.importedAt).toLocaleString()} · {record.sourceType === "document" ? "文字文档" : formatDuration(record.audioDurationMs, true)}</span>
                <span className="record-library">{record.projectName ?? "未归档"}</span>
              </button>
              <span className={`status-pill status-${statusTone(record)}`}>{label}</span>
            </li>
          );
        })}
        {visible.length === 0 && (
          <li className="record-empty">
            {view === "pending" ? "当前没有待处理资料。" : "尚未导入录音或文档。"}
          </li>
        )}
      </ul>

      <div className="pane-messages">
        {notice && <p className="inline-notice">{notice}</p>}
        {error && <p className="inline-error">{error}</p>}
      </div>

      <DocumentImportDialog
        open={documentImportOpen}
        projectId={projectId}
        onClose={() => {
          setDocumentImportOpen(false);
          if (documentImportChanged) {
            setDocumentImportChanged(false);
            onImported();
          }
        }}
        onImported={(record, created) => {
          setNotice(created
            ? `已导入“${record.title}”，可查看正文和分析结果。`
            : `已打开已有记录“${record.title}”，未创建重复副本。`);
          if (created) setDocumentImportChanged(true);
          void refresh();
          onSelect(record);
        }}
      />
    </section>
  );
}

function isPending(record: RecordBrief) {
  return !record.hasTranscript || !record.hasAnalysis || !record.projectId;
}

function statusLabel(record: RecordBrief) {
  if (isProcessing(record.status) || record.status === "failed") return STATUS_LABEL[record.status];
  if (!record.hasAnalysis && record.lastAnalysisError) return "分析失败";
  if (record.analysisStatus === "stale") return "分析需要更新";
  if (record.analysisStatus === "incomplete") return "分析不完整";
  if (!record.hasAnalysis) return "待分析";
  if (!record.projectId) return "待归档";
  return "已完成";
}

function statusTone(record: RecordBrief) {
  if (record.status === "failed" || (!record.hasAnalysis && record.lastAnalysisError)) return "error";
  if (isProcessing(record.status)) return "progress";
  if (isPending(record)) return "attention";
  return "done";
}
