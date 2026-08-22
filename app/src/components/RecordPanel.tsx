import { open } from "@tauri-apps/plugin-dialog";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useMemo, useState } from "react";
import type { InboxStatus, RecordBrief, RecordStatus } from "../shared/types";
import { importAudio, listRecords, transcribeRecord } from "../lib/tauri";
import { formatMinutesSeconds as formatDuration, isProcessingStatus as isProcessing } from "../lib/format";
import { getInboxStatus } from "../lib/tauri";
import DocumentImportDialog from "./DocumentImportDialog";

interface Props {
  projectId: string | null;
  unfiledOnly: boolean;
  onImported: () => void;
  selectedId: string | null;
  onSelect: (record: RecordBrief | null) => void;
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

export default function RecordPanel({ projectId, unfiledOnly, onImported, selectedId, onSelect }: Props) {
  const [records, setRecords] = useState<RecordBrief[]>([]);
  const [view, setView] = useState<WorkspaceView>("pending");
  const [inbox, setInbox] = useState<InboxStatus | null>(null);

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
    const stop = listen("inbox-update", () => void load());
    return () => {
      cancelled = true;
      void stop.then((unlisten) => unlisten());
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

  useEffect(() => {
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

  return (
    <section className="record-pane" aria-label="工作区">
      <header className="pane-heading workspace-heading">
        <div>
          <p className="pane-eyebrow">资料处理</p>
          <h2>工作区</h2>
        </div>
        <span className="count-badge">{visible.length}</span>
      </header>

      {inbox && (inbox.watchFolders.length > 0 || inbox.usbDetection) && (
        <p className="inbox-strip" role="status">
          📮 收件箱监听中
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

      <ul className="record-list" aria-live="polite">
        {visible.map((record) => {
          const label = statusLabel(record);
          return (
            <li key={record.id} className={selectedId === record.id ? "selected" : ""}>
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
