import { open } from "@tauri-apps/plugin-dialog";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useMemo, useState } from "react";
import type { InboxStatus, Project, RecordBrief, RecordStatus } from "../shared/types";
import {
  deleteRecords,
  importAudio,
  listArchivedRecords,
  listProjects,
  listRecords,
  moveRecords,
  setRecordArchived,
  transcribeRecord,
  updateRecordTitle,
} from "../lib/tauri";
import { formatMinutesSeconds as formatDuration, isProcessingStatus as isProcessing } from "../lib/format";
import { getInboxStatus } from "../lib/tauri";
import DocumentImportDialog from "./DocumentImportDialog";
import { InboxIcon, MoreIcon, PlayIcon } from "./icons";

interface Props {
  projectId: string | null;
  unfiledOnly: boolean;
  onImported: () => void;
  selectedId: string | null;
  onSelect: (record: RecordBrief | null) => void;
  /** 播放钮：选中该记录并立即开始播放（仅音频记录显示播放钮）。 */
  onPlay: (record: RecordBrief) => void;
}

type WorkspaceView = "pending" | "recent" | "archived";
type RecordMenuMode = "menu" | "move" | "rename";

interface RecordMenuState {
  record: RecordBrief;
  mode: RecordMenuMode;
  x: number;
  y: number;
}

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
  const [archivedRecords, setArchivedRecords] = useState<RecordBrief[]>([]);
  const [view, setView] = useState<WorkspaceView>("pending");
  const [inbox, setInbox] = useState<InboxStatus | null>(null);
  const [selected, setSelected] = useState<Set<string>>(() => new Set());
  const [projects, setProjects] = useState<Project[]>([]);
  const [moveTarget, setMoveTarget] = useState("");
  const [bulkBusy, setBulkBusy] = useState(false);
  const [recordMenu, setRecordMenu] = useState<RecordMenuState | null>(null);
  const [menuMoveTarget, setMenuMoveTarget] = useState("");
  const [menuRenameValue, setMenuRenameValue] = useState("");
  const [menuBusy, setMenuBusy] = useState(false);
  const [menuError, setMenuError] = useState("");

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
      const [active, archived] = await Promise.all([
        listRecords(projectId, unfiledOnly),
        listArchivedRecords(projectId, unfiledOnly),
      ]);
      setRecords(active);
      setArchivedRecords(archived);
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
    if (!recordMenu) return;
    const closeOnOutside = (event: MouseEvent) => {
      const target = event.target;
      if (target instanceof Element && !target.closest("[data-record-menu]")) {
        setRecordMenu(null);
      }
    };
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") setRecordMenu(null);
    };
    document.addEventListener("mousedown", closeOnOutside);
    document.addEventListener("keydown", closeOnEscape);
    return () => {
      document.removeEventListener("mousedown", closeOnOutside);
      document.removeEventListener("keydown", closeOnEscape);
    };
  }, [recordMenu]);

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
    setRecordMenu(null);
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
  const visible = view === "pending"
    ? pending
    : view === "archived"
      ? archivedRecords
      : records;

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

  function openRecordMenu(record: RecordBrief, x: number, y: number) {
    setRecordMenu({
      record,
      mode: "menu",
      x: Math.max(8, Math.min(x, window.innerWidth - 224)),
      y: Math.max(8, Math.min(y, window.innerHeight - 260)),
    });
    setMenuMoveTarget(record.projectId ?? "");
    setMenuRenameValue(record.title);
    setMenuError("");
  }

  async function runRecordAction(
    action: () => Promise<unknown>,
    successMessage: string,
    afterSuccess?: () => void,
  ) {
    if (menuBusy) return;
    setMenuBusy(true);
    setMenuError("");
    try {
      await action();
      await refresh();
      onImported();
      setNotice(successMessage);
      afterSuccess?.();
      setRecordMenu(null);
    } catch (reason) {
      setMenuError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setMenuBusy(false);
    }
  }

  function archiveFromMenu(record: RecordBrief) {
    const archived = record.archivedAt === null;
    void runRecordAction(
      () => setRecordArchived(record.id, archived),
      archived ? `已归档“${record.title}”。` : `已恢复“${record.title}”。`,
      () => {
        if (selectedId === record.id && archived) onSelect(null);
      },
    );
  }

  function moveFromMenu(record: RecordBrief) {
    void runRecordAction(
      async () => {
        await moveRecords([record.id], menuMoveTarget || null);
      },
      menuMoveTarget
        ? `已将“${record.title}”转移到所选知识库。`
        : `已将“${record.title}”移动到未归档。`,
    );
  }

  function renameFromMenu(record: RecordBrief) {
    const title = menuRenameValue.trim();
    if (!title || title === record.title) {
      setMenuError(title ? "新名称与当前名称相同。" : "名称不能为空。");
      return;
    }
    void runRecordAction(
      () => updateRecordTitle(record.id, title),
      `已重命名为“${title}”。`,
    );
  }

  function deleteFromMenu(record: RecordBrief) {
    const confirmed = window.confirm(
      `确定删除“${record.title}”吗？\n\n逐字稿、分析结果和知识索引会一起删除，原始文件也会从本机移除，且无法恢复。`,
    );
    if (!confirmed) return;
    let cleanupFailureCount = 0;
    void runRecordAction(
      async () => {
        const result = await deleteRecords([record.id]);
        cleanupFailureCount = result.fileCleanupFailures.length;
      },
      `已删除“${record.title}”及其原始文件。`,
      () => {
        if (cleanupFailureCount > 0) {
          setNotice(`已删除“${record.title}”，但有 ${cleanupFailureCount} 个文件遗留。`);
        }
        setSelected((current) => {
          const next = new Set(current);
          next.delete(record.id);
          return next;
        });
        if (selectedId === record.id) onSelect(null);
      },
    );
  }

  return (
    <section className="record-pane" aria-label="工作区">
      <header className="pane-heading workspace-heading">
        <div>
          <p className="pane-eyebrow">资料处理</p>
          <h2>工作区</h2>
        </div>
        {view !== "archived" && (
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
        )}
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
        <button type="button" className={view === "archived" ? "selected" : ""} onClick={() => setView("archived")}>归档 <span>{archivedRecords.length}</span></button>
      </div>

      {view !== "archived" && (
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
      )}

      {view !== "archived" && someSelected && (
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
            <li
              key={record.id}
              className={selectedId === record.id ? "selected" : ""}
              onContextMenu={(event) => {
                event.preventDefault();
                openRecordMenu(record, event.clientX, event.clientY);
              }}
            >
              {view !== "archived" && (
                <label className="record-check" onClick={(event) => event.stopPropagation()}>
                  <input
                    type="checkbox"
                    checked={selected.has(record.id)}
                    onChange={() => toggleOne(record.id)}
                    aria-label={`选择 ${record.title}`}
                  />
                </label>
              )}
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
              <div className="record-actions">
                <span className={`status-pill status-${view === "archived" ? "attention" : statusTone(record)}`}>
                  {view === "archived" ? "已归档" : label}
                </span>
                <button
                  type="button"
                  className="record-menu-button"
                  aria-label={`${record.title}操作`}
                  title="更多操作"
                  onClick={(event) => {
                    event.stopPropagation();
                    const rect = event.currentTarget.getBoundingClientRect();
                    openRecordMenu(record, rect.right - 216, rect.bottom + 4);
                  }}
                >
                  <MoreIcon size={16} />
                </button>
              </div>
            </li>
          );
        })}
        {visible.length === 0 && (
          <li className="record-empty">
            {view === "archived"
              ? "没有已归档资料。"
              : view === "pending"
                ? "当前没有待处理资料。"
                : "尚未导入录音或文档。"}
          </li>
        )}
      </ul>

      {recordMenu && (
        <div
          className="record-context-menu"
          data-record-menu
          role="menu"
          aria-label={`${recordMenu.record.title}操作菜单`}
          style={{ left: recordMenu.x, top: recordMenu.y }}
        >
          <div className="record-context-header">
            <strong>{recordMenu.record.title}</strong>
            <span>{recordMenu.record.archivedAt ? "已归档" : recordMenu.record.projectName ?? "未归档"}</span>
          </div>
          {recordMenu.mode === "menu" && (
            <>
              <button
                type="button"
                onClick={() => {
                  onSelect(recordMenu.record);
                  setRecordMenu(null);
                }}
              >
                打开管理
              </button>
              <button
                type="button"
                onClick={() => {
                  setRecordMenu({ ...recordMenu, mode: "move" });
                  setMenuError("");
                }}
              >
                转移知识库
              </button>
              <button
                type="button"
                onClick={() => {
                  setRecordMenu({ ...recordMenu, mode: "rename" });
                  setMenuError("");
                }}
              >
                重命名
              </button>
              <button type="button" disabled={menuBusy} onClick={() => archiveFromMenu(recordMenu.record)}>
                {recordMenu.record.archivedAt ? "恢复" : "归档"}
              </button>
              <button type="button" className="danger-text" disabled={menuBusy} onClick={() => deleteFromMenu(recordMenu.record)}>
                删除
              </button>
            </>
          )}
          {recordMenu.mode === "move" && (
            <div className="record-context-editor">
              <label>
                <span>目标知识库</span>
                <select
                  value={menuMoveTarget}
                  onChange={(event) => setMenuMoveTarget(event.target.value)}
                  disabled={menuBusy}
                  aria-label="单条记录目标知识库"
                >
                  <option value="">未归档</option>
                  {projects.map((project) => (
                    <option key={project.id} value={project.id}>{project.name}</option>
                  ))}
                </select>
              </label>
              <div className="record-context-buttons">
                <button type="button" disabled={menuBusy} onClick={() => setRecordMenu({ ...recordMenu, mode: "menu" })}>返回</button>
                <button type="button" className="primary-button" disabled={menuBusy} onClick={() => moveFromMenu(recordMenu.record)}>
                  {menuBusy ? "处理中…" : "移动"}
                </button>
              </div>
            </div>
          )}
          {recordMenu.mode === "rename" && (
            <div className="record-context-editor">
              <label>
                <span>记录名称</span>
                <input
                  value={menuRenameValue}
                  onChange={(event) => setMenuRenameValue(event.target.value)}
                  disabled={menuBusy}
                  aria-label="记录名称"
                  autoFocus
                />
              </label>
              <div className="record-context-buttons">
                <button type="button" disabled={menuBusy} onClick={() => setRecordMenu({ ...recordMenu, mode: "menu" })}>返回</button>
                <button type="button" className="primary-button" disabled={menuBusy} onClick={() => renameFromMenu(recordMenu.record)}>
                  {menuBusy ? "保存中…" : "保存"}
                </button>
              </div>
            </div>
          )}
          {menuError && <p className="record-context-error" role="alert">{menuError}</p>}
        </div>
      )}

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
  if (record.analysisStatus === "completed" && record.analysisHasQualityWarning) return "已完成 · 有质量提醒";
  if (!record.hasAnalysis) return "待分析";
  if (!record.projectId) return "待归档";
  return "已完成";
}

function statusTone(record: RecordBrief) {
  if (record.status === "failed" || (!record.hasAnalysis && record.lastAnalysisError)) return "error";
  if (isProcessing(record.status)) return "progress";
  if (record.analysisStatus === "completed" && record.analysisHasQualityWarning) return "attention";
  if (isPending(record)) return "attention";
  return "done";
}
