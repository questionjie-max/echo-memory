import { useEffect, useState } from "react";
import { XIcon } from "./icons";
import type { ExternalAiSettings, MemoryScope, MemorySnapshot, MemoryViewKind } from "../shared/types";
import {
  formatMemoryDate,
  memoryRangeLabel,
  memoryScopeLabel,
  type MemoryRangeKey,
} from "../hooks/useMemoryViewData";

const RANGE_OPTIONS: Array<[MemoryRangeKey, string]> = [["7d", "7 天"], ["30d", "30 天"], ["90d", "90 天"], ["all", "全部"]];

interface Props {
  title: string;
  description: string;
  viewKind: MemoryViewKind;
  scope: MemoryScope;
  range: MemoryRangeKey;
  onRangeChange: (range: MemoryRangeKey) => void;
  settings: ExternalAiSettings | null;
  snapshots: MemorySnapshot[];
  selectedSnapshotId: string | null;
  onSelectSnapshot: (id: string | null) => void;
  sourceRecordCount: number;
  generating: boolean;
  onGenerate: () => Promise<unknown>;
  onCancel: () => Promise<void>;
  onOpenSettings: () => void;
}

export default function MemoryViewControls(props: Props) {
  const [confirmOpen, setConfirmOpen] = useState(false);
  useEffect(() => {
    setConfirmOpen(false);
  }, [
    props.viewKind,
    props.scope.kind,
    props.scope.projectId,
    props.range,
  ]);
  const selected = props.snapshots.find((item) => item.id === props.selectedSnapshotId) ?? null;
  const ready = Boolean(props.settings?.enabled && props.settings.hasApiKey && props.settings.privacyConsentAt);
  const estimatedLow = props.sourceRecordCount * 4_000;
  const estimatedHigh = props.sourceRecordCount * 12_000;

  return <div className="memory-view-controls">
    <header className="memory-view-header view-header">
      <div>
        <p className="pane-eyebrow">{memoryScopeLabel(props.scope)} · {memoryRangeLabel(props.range)}</p>
        <h2>{props.title}</h2>
        <p>{props.description}</p>
      </div>
      <div className="memory-header-actions">
        <div className="range-switch" role="group" aria-label="时间范围">{RANGE_OPTIONS.map(([value, label]) => <button type="button" key={value} className={props.range === value ? "selected" : ""} aria-pressed={props.range === value} onClick={() => props.onRangeChange(value)}>{label}</button>)}</div>
        {props.snapshots.length > 0 && <select aria-label="快照版本" value={props.selectedSnapshotId ?? ""} onChange={(event) => props.onSelectSnapshot(event.target.value || null)}>{props.snapshots.map((snapshot) => <option key={snapshot.id} value={snapshot.id}>v{snapshot.version} · {formatMemoryDate(snapshot.createdAt, { month: "numeric", day: "numeric", hour: "2-digit", minute: "2-digit" })} · {snapshot.model}</option>)}</select>}
        {props.generating ? <button type="button" className="secondary-button" onClick={() => void props.onCancel()}>取消生成</button> : <button type="button" className="primary-button" disabled={props.sourceRecordCount === 0 || !props.settings} onClick={() => ready ? setConfirmOpen(true) : props.onOpenSettings()}>{props.settings ? (selected ? "重新生成" : "生成") : "检查 AI 设置…"}</button>}
      </div>
    </header>
    <div className="memory-status-row" role="status" aria-live="polite" aria-atomic="true">
      <span>作用域：{memoryScopeLabel(props.scope)}</span>
      <span>来源记录：{props.sourceRecordCount}</span>
      {selected && <><span>快照 v{selected.version}</span><span>{formatMemoryDate(selected.createdAt, { month: "numeric", day: "numeric", hour: "2-digit", minute: "2-digit" })} · {selected.model}</span><StatusBadge snapshot={selected} /></>}
      {props.settings && !ready && <button type="button" className="link-button" onClick={props.onOpenSettings}>配置外部 AI</button>}
    </div>
    {selected?.qualityWarning && selected.provider !== "demo" && <SnapshotWarning kind="quality" snapshotId={selected.id} prefix="质量提示：" message={selected.qualityWarning} />}
    {selected?.errorMessage && <SnapshotWarning kind="error" snapshotId={selected.id} prefix="部分生成异常：" message={selected.errorMessage} />}
    {confirmOpen && <section className="generation-confirmation">
      <div><strong>确认外部发送范围</strong><p>将发送 {memoryScopeLabel(props.scope)}中 {props.sourceRecordCount} 条记录，范围为{memoryRangeLabel(props.range)}。</p></div>
      <dl><div><dt>预计文本量</dt><dd>{props.sourceRecordCount ? `约 ${estimatedLow.toLocaleString()}–${estimatedHigh.toLocaleString()} 字符` : "0 字符"}</dd></div><div><dt>包含</dt><dd>逐字稿全文、标题、时间、项目和现有结构化分析</dd></div><div><dt>不包含</dt><dd>原始音频文件</dd></div></dl>
      <div className="generation-actions"><button type="button" className="secondary-button" onClick={() => setConfirmOpen(false)}>返回</button><button type="button" className="primary-button" onClick={() => { setConfirmOpen(false); void props.onGenerate(); }}>确认发送并生成</button></div>
    </section>}
  </div>;
}

function StatusBadge({ snapshot }: { snapshot: MemorySnapshot }) {
  return <span className={`memory-badge ${snapshot.isStale ? "stale" : snapshot.status}`}>{snapshot.isStale ? "可能过期" : statusLabel(snapshot.status)}</span>;
}

// 快照级的质量/异常提醒：允许用户关掉当前这条；同一快照的同一条不再弹，
// 但换快照或消息内容变化后会重新出现——持久的状态信息由状态行徽章（部分结果/可能过期）兜底。
function SnapshotWarning({ kind, snapshotId, prefix, message }: { kind: "quality" | "error"; snapshotId: string; prefix: string; message: string }) {
  const storageKey = `echo-memory-warning-dismissed:${snapshotId}:${kind}`;
  const [hidden, setHidden] = useState(() => readDismissed(storageKey, message));
  useEffect(() => {
    setHidden(readDismissed(storageKey, message));
  }, [storageKey, message]);
  if (hidden) return null;
  return (
    <p className="memory-warning" role={kind === "error" ? "alert" : "status"}>
      <span className="memory-warning-copy">{prefix}{message}</span>
      <button
        type="button"
        className="memory-warning-dismiss"
        aria-label="关闭提醒"
        onClick={() => {
          try {
            localStorage.setItem(storageKey, message);
          } catch {
            // 忽略存储失败，仅本次会话内隐藏
          }
          setHidden(true);
        }}
      >
        <XIcon size={11} />
      </button>
    </p>
  );
}

function readDismissed(storageKey: string, message: string) {
  try {
    return localStorage.getItem(storageKey) === message;
  } catch {
    return false;
  }
}

function statusLabel(status: MemorySnapshot["status"]) {
  return { generating: "生成中", completed: "已完成", partial: "部分结果", failed: "失败", cancelled: "已取消" }[status];
}
