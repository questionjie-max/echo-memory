import { open } from "@tauri-apps/plugin-dialog";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useMemo, useState } from "react";
import type { DiskSpaceStatus, InboxStatus, Project, RecordBrief, RecordStatus } from "../shared/types";
import {
  deleteRecords,
  getDiskSpaceStatus,
  importAudio,
  listArchivedRecords,
  listProjects,
  listRecords,
  moveRecords,
  setRecordArchived,
  transcribeRecord,
  updateRecordTitle,
} from "../lib/tauri";
import { formatBytes, formatMinutesSeconds as formatDuration, isProcessingStatus as isProcessing } from "../lib/format";
import { getInboxStatus } from "../lib/tauri";
import DocumentImportDialog from "./DocumentImportDialog";
import ChatImportDialog from "./ChatImportDialog";
import { InboxIcon, ChevronDownIcon, MoreIcon, PlayIcon } from "./icons";

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
type ContentKind = "audio" | "document" | "chat";

/**
 * 三种输入方式即三类内容：录音 / 文档 / AI 对话。
 * 工作区列表按这三类分组，组头可点击折叠，组头右侧就是这一类的导入入口——
 * 折叠的既是内容也是入口，点开哪一类就看到哪一类。
 */
const CONTENT_KINDS: ContentKind[] = ["audio", "document", "chat"];
const CONTENT_KIND_META: Record<
  ContentKind,
  { label: string; hint: string; action: string; primary: boolean }
> = {
  audio: {
    label: "录音",
    hint: "MP3、M4A、WAV、AAC，也可拖入窗口",
    action: "选择音频",
    primary: true,
  },
  document: {
    label: "文档",
    hint: "Markdown、TXT、Word 文档",
    action: "选择文档",
    primary: false,
  },
  chat: {
    label: "AI 对话",
    hint: "ChatGPT / Claude 导出、对话式 Markdown",
    action: "选择导出",
    primary: false,
  },
};

/** 记录的内容归类：音频导入 ⇒ 录音；文档与 AI 对话各有自己的类目词。 */
function contentKind(record: RecordBrief): ContentKind {
  if (record.sourceType === "document") return "document";
  if (record.sourceType === "chat") return "chat";
  return "audio";
}

const GROUP_COLLAPSE_KEY = "workspace.group-collapse";

/** 分组折叠记忆：存「已收起」的类目数组；解析失败按全部展开处理。 */
function readCollapsedKinds(): Set<ContentKind> {
  try {
    const raw = window.localStorage.getItem(GROUP_COLLAPSE_KEY);
    if (!raw) return new Set();
    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed)) return new Set();
    const kinds = parsed.filter(
      (kind): kind is ContentKind =>
        typeof kind === "string" && (CONTENT_KINDS as string[]).includes(kind),
    );
    return new Set(kinds);
  } catch {
    return new Set();
  }
}

function writeCollapsedKinds(collapsed: Set<ContentKind>) {
  try {
    window.localStorage.setItem(GROUP_COLLAPSE_KEY, JSON.stringify([...collapsed]));
  } catch {
    // 存储不可用只影响下次打开的记忆，不影响本次折叠
  }
}

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
  const [diskSpace, setDiskSpace] = useState<DiskSpaceStatus | null>(null);
  const [collapsedKinds, setCollapsedKinds] = useState<Set<ContentKind>>(() => readCollapsedKinds());
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

  // 磁盘空间提醒：进页面查一次、每分钟刷新；只提示不阻塞（低空间时导入/转写会明确报错）。
  useEffect(() => {
    let cancelled = false;
    const loadDiskSpace = async () => {
      try {
        const status = await getDiskSpaceStatus();
        if (!cancelled) setDiskSpace(status);
      } catch {
        if (!cancelled) setDiskSpace(null);
      }
    };
    void loadDiskSpace();
    const timer = window.setInterval(() => void loadDiskSpace(), 60_000);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, []);

  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [documentImportOpen, setDocumentImportOpen] = useState(false);
  const [documentImportChanged, setDocumentImportChanged] = useState(false);
  const [chatImportOpen, setChatImportOpen] = useState(false);
  const [chatImportChanged, setChatImportChanged] = useState(false);

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
      filters: [{ name: "音频", extensions: ["mp3", "m4a", "wav", "aac"] }],
    });
    if (!selection) return;
    await ingestPaths(Array.isArray(selection) ? selection : [selection]);
  }

  const pending = useMemo(() => records.filter(isPending), [records]);
  const viewRecords = view === "pending"
    ? pending
    : view === "archived"
      ? archivedRecords
      : records;
  // 待处理视图只可能有录音与文档两类待办内容：对话导入即可检索，不进这个队列。
  const groupKinds = useMemo(
    () => (view === "pending" ? CONTENT_KINDS.filter((kind) => kind !== "chat") : CONTENT_KINDS),
    [view],
  );
  const groups = useMemo(
    () =>
      groupKinds.map((kind) => ({
        kind,
        items: viewRecords.filter((record) => contentKind(record) === kind),
      })),
    [groupKinds, viewRecords],
  );
  const visible = viewRecords;

  function toggleKind(kind: ContentKind) {
    setCollapsedKinds((current) => {
      const next = new Set(current);
      if (next.has(kind)) next.delete(kind);
      else next.add(kind);
      writeCollapsedKinds(next);
      return next;
    });
  }

  function startImport(kind: ContentKind) {
    if (kind === "audio") {
      void chooseFiles();
      return;
    }
    if (kind === "document") {
      setDocumentImportChanged(false);
      setDocumentImportOpen(true);
      return;
    }
    setChatImportChanged(false);
    setChatImportOpen(true);
  }

  /** 记录行：分组只改变它挂在哪个 section 下，行内的勾选/播放/菜单/右键行为完全不变。 */
  function renderRow(record: RecordBrief) {
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
        {record.sourceType !== "document" && record.sourceType !== "chat" && (
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
          <span>{new Date(record.importedAt).toLocaleString()} · {record.sourceType === "document" ? "文字文档" : record.sourceType === "chat" ? "AI 对话" : formatDuration(record.audioDurationMs, true)}</span>
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
  }

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

      {diskSpace && diskSpace.availableBytes < diskSpace.lowSpaceThresholdBytes && (
        <p className="space-warning" role="alert">
          磁盘可用空间仅剩 {formatBytes(diskSpace.availableBytes)}：导入、转写或模型下载可能失败，请先释放空间。
        </p>
      )}

      <div className="segmented-control" aria-label="工作区视图">
        <button type="button" className={view === "pending" ? "selected" : ""} onClick={() => setView("pending")}>待处理 <span>{pending.length}</span></button>
        <button type="button" className={view === "recent" ? "selected" : ""} onClick={() => setView("recent")}>最近 <span>{records.length}</span></button>
        <button type="button" className={view === "archived" ? "selected" : ""} onClick={() => setView("archived")}>归档 <span>{archivedRecords.length}</span></button>
      </div>

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

      <div className="record-list" aria-live="polite">
        {view === "archived" && visible.length === 0 ? (
          <p className="record-empty">没有已归档资料。</p>
        ) : (
          groups.map(({ kind, items }) => {
            const meta = CONTENT_KIND_META[kind];
            const collapsed = collapsedKinds.has(kind);
            return (
              <section className="record-group" key={kind} aria-label={`${meta.label}分组`}>
                <div className="record-group-header">
                  <button
                    type="button"
                    className="record-group-toggle"
                    aria-expanded={!collapsed}
                    aria-label={`${meta.label}分组（${items.length} 条）`}
                    onClick={() => toggleKind(kind)}
                  >
                    <span className="record-group-chevron" aria-hidden="true"><ChevronDownIcon size={12} /></span>
                    <strong>{meta.label}</strong>
                    <span className="record-group-count">{items.length}</span>
                    <span className="record-group-hint">{meta.hint}</span>
                  </button>
                  {view !== "archived" && (
                    <button
                      type="button"
                      className={meta.primary ? "primary-button" : "secondary-button"}
                      onClick={() => startImport(kind)}
                      disabled={busy}
                    >
                      {busy && kind === "audio" ? "处理中…" : meta.action}
                    </button>
                  )}
                </div>
                {!collapsed && (
                  <ul className="record-group-items">
                    {items.length === 0 ? (
                      <li className="record-empty">
                        {view === "archived" ? "这一类暂无归档内容。" : "这一类暂无内容，可从上方导入。"}
                      </li>
                    ) : (
                      items.map((record) => renderRow(record))
                    )}
                  </ul>
                )}
              </section>
            );
          })
        )}
      </div>

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

      <ChatImportDialog
        open={chatImportOpen}
        projectId={projectId}
        onClose={() => {
          setChatImportOpen(false);
          if (chatImportChanged) {
            setChatImportChanged(false);
            onImported();
          }
        }}
        onImported={() => {
          setNotice("AI 对话已导入，分析正在后台进行。");
          setChatImportChanged(true);
          void refresh();
        }}
      />
    </section>
  );
}

function isPending(record: RecordBrief) {
  // AI 对话导入即入库可检索，没有「待处理」语义；留在待处理里会把这个队列彻底淹没。
  if (record.sourceType === "chat") return false;
  return !record.hasTranscript || !record.hasAnalysis || !record.projectId;
}

function statusLabel(record: RecordBrief) {
  if (isProcessing(record.status) || record.status === "failed") return STATUS_LABEL[record.status];
  if (record.sourceType === "chat") return "对话档案";
  if (!record.hasAnalysis && record.lastAnalysisError) return "分析失败";
  if (record.analysisStatus === "stale") return "分析需要更新";
  if (record.analysisStatus === "incomplete") return "分析不完整";
  if (record.analysisStatus === "completed" && record.analysisHasQualityWarning) return "已完成 · 有质量提醒";
  if (!record.hasAnalysis) return "待分析";
  if (!record.projectId) return "待归档";
  return "已完成";
}

function statusTone(record: RecordBrief) {
  if (record.status === "failed") return "error";
  if (isProcessing(record.status)) return "progress";
  // 对话档案是类目词而非待办状态：不用错误/待办色，与「已完成」同色。
  if (record.sourceType === "chat") return "done";
  if (!record.hasAnalysis && record.lastAnalysisError) return "error";
  if (record.analysisStatus === "completed" && record.analysisHasQualityWarning) return "attention";
  if (isPending(record)) return "attention";
  return "done";
}
