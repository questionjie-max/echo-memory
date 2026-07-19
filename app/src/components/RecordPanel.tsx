import { open } from "@tauri-apps/plugin-dialog";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { useEffect, useMemo, useState } from "react";
import type { RecordBrief, RecordStatus } from "../shared/types";
import { importAudio, listRecords, transcribeRecord } from "../lib/tauri";

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
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");

  async function refresh() {
    try {
      setRecords(await listRecords(projectId, unfiledOnly));
    } catch (reason) {
      setError(String(reason));
    }
  }

  useEffect(() => {
    void refresh();
    const unlisten = getCurrentWebviewWindow().onDragDropEvent((event) => {
      if (event.payload.type === "drop") void ingestPaths(event.payload.paths);
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
          <p className="pane-eyebrow">音频处理</p>
          <h2>工作区</h2>
        </div>
        <span className="count-badge">{visible.length}</span>
      </header>

      <div className="segmented-control" aria-label="工作区视图">
        <button type="button" className={view === "pending" ? "selected" : ""} onClick={() => setView("pending")}>待处理 <span>{pending.length}</span></button>
        <button type="button" className={view === "recent" ? "selected" : ""} onClick={() => setView("recent")}>最近 <span>{records.length}</span></button>
      </div>

      <div className="import-strip">
        <div>
          <strong>导入录音</strong>
          <span>MP3、M4A、WAV，也可拖入窗口</span>
        </div>
        <button type="button" className="primary-button" onClick={() => void chooseFiles()} disabled={busy}>
          {busy ? "处理中…" : "选择文件"}
        </button>
      </div>

      <ul className="record-list" aria-live="polite">
        {visible.map((record) => {
          const label = statusLabel(record);
          return (
            <li key={record.id} className={selectedId === record.id ? "selected" : ""}>
              <button type="button" className="record-main" onClick={() => onSelect(record)}>
                <strong>{record.title}</strong>
                <span>{new Date(record.importedAt).toLocaleString()} · {formatDuration(record.audioDurationMs)}</span>
                <span className="record-library">{record.projectName ?? "未归档"}</span>
              </button>
              <span className={`status-pill status-${statusTone(record)}`}>{label}</span>
            </li>
          );
        })}
        {visible.length === 0 && (
          <li className="record-empty">
            {view === "pending" ? "当前没有待处理录音。" : "尚未导入录音。"}
          </li>
        )}
      </ul>

      <div className="pane-messages">
        {notice && <p className="inline-notice">{notice}</p>}
        {error && <p className="inline-error">{error}</p>}
      </div>
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

function isProcessing(status: RecordStatus) {
  return status === "preparing" || status === "transcribing" || status === "analyzing";
}

function formatDuration(milliseconds: number) {
  const totalSeconds = Math.round(milliseconds / 1000);
  return `${Math.floor(totalSeconds / 60)}:${String(totalSeconds % 60).padStart(2, "0")}`;
}
