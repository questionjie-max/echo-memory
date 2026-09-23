import { useEffect, useRef, useState } from "react";
import { SparklesIcon } from "./icons";
import MemoryViewControls from "./MemoryViewControls";
import type { EvolutionItem, MemoryFeedback, MemoryScope, MemorySourceReference } from "../shared/types";
import { listMemoryFeedback, updateMemoryFeedback } from "../lib/tauri";
import {
  formatMemoryDate,
  useMemoryViewData,
  type MemoryRangeKey,
} from "../hooks/useMemoryViewData";

interface Props {
  scope: MemoryScope;
  onOpenSource: (source: MemorySourceReference) => void;
  onOpenSettings: () => void;
}

export default function EvolutionView({ scope, onOpenSource, onOpenSettings }: Props) {
  const [range, setRange] = useState<MemoryRangeKey>("90d");
  const [feedback, setFeedback] = useState<MemoryFeedback[]>([]);
  const [feedbackBusy, setFeedbackBusy] = useState<Set<string>>(() => new Set());
  const [feedbackError, setFeedbackError] = useState("");
  const [feedbackStatus, setFeedbackStatus] = useState("");
  const feedbackRequestId = useRef(0);
  const activeSnapshotId = useRef<string | null>(null);
  const data = useMemoryViewData("evolution", scope, range);
  activeSnapshotId.current = data.selectedSnapshot?.id ?? null;
  const items = data.selectedSnapshot?.result.evolutionItems ?? [];
  const groups = groupByTopic(items);

  useEffect(() => {
    const snapshotId = data.selectedSnapshot?.id ?? null;
    const requestId = ++feedbackRequestId.current;
    setFeedback([]);
    setFeedbackError("");
    setFeedbackStatus("");
    setFeedbackBusy(new Set());
    if (!snapshotId) return;

    void listMemoryFeedback(snapshotId)
      .then((items) => {
        if (requestId !== feedbackRequestId.current || activeSnapshotId.current !== snapshotId) return;
        setFeedback(items);
      })
      .catch((reason) => {
        if (requestId !== feedbackRequestId.current || activeSnapshotId.current !== snapshotId) return;
        setFeedbackError(`反馈读取失败：${errorMessage(reason)}`);
      });

    return () => {
      if (requestId === feedbackRequestId.current) feedbackRequestId.current += 1;
    };
  }, [data.selectedSnapshot?.id]);

  async function decide(item: EvolutionItem, decision: "confirmed" | "rejected") {
    if (!data.selectedSnapshot) return;
    const snapshotId = data.selectedSnapshot.id;
    const busyKey = `${snapshotId}:${item.id}`;
    const note = window.prompt(decision === "confirmed" ? "可选：补充确认备注" : "可选：说明驳回原因", feedback.find((entry) => entry.itemId === item.id)?.note ?? "");
    if (note === null) return;
    setFeedbackBusy((current) => new Set(current).add(busyKey));
    setFeedbackError("");
    setFeedbackStatus("正在保存反馈…");
    try {
      const saved = await updateMemoryFeedback(snapshotId, item.id, decision, note);
      if (activeSnapshotId.current !== snapshotId) return;
      setFeedback((current) => [saved, ...current.filter((entry) => entry.itemId !== item.id)]);
      setFeedbackStatus(decision === "confirmed" ? "反馈已确认。" : "反馈已驳回。");
    } catch (reason) {
      if (activeSnapshotId.current === snapshotId) {
        setFeedbackStatus("");
        setFeedbackError(`反馈保存失败：${errorMessage(reason)}`);
      }
    } finally {
      setFeedbackBusy((current) => {
        const next = new Set(current);
        next.delete(busyKey);
        return next;
      });
    }
  }

  return <section className="memory-view evolution-view">
    <MemoryViewControls
      title="认知演化"
      description="按主题查看观点的新增、补充、修正、推翻、合并和验证过程。"
      viewKind="evolution"
      scope={scope}
      range={range}
      onRangeChange={setRange}
      settings={data.settings}
      snapshots={data.snapshots}
      selectedSnapshotId={data.selectedSnapshotId}
      onSelectSnapshot={data.setSelectedSnapshotId}
      sourceRecordCount={data.sourceRecordCount}
      generating={data.generating}
      onGenerate={data.generate}
      onCancel={data.cancel}
      onOpenSettings={onOpenSettings}
    />
    {feedbackStatus && <p className="inline-notice memory-notice" role="status" aria-live="polite">{feedbackStatus}</p>}
    {data.loading ? <div className="memory-empty" role="status" aria-live="polite"><strong>正在加载认知演化…</strong></div> : data.error || feedbackError ? <div className="memory-empty memory-error-state" role="alert"><strong>认知演化加载失败</strong><p>{[data.error, feedbackError].filter(Boolean).join("；")}</p><button type="button" className="secondary-button" onClick={() => { setFeedbackError(""); void data.reload(); }}>重试</button></div> : !data.selectedSnapshot ? (data.settings && !isExternalAiReady(data.settings) ? <ConfiguredEmpty onOpenSettings={onOpenSettings} /> : <NoSnapshotEmpty />) : <div className="evolution-content">
      {groups.length === 0 ? <div className="memory-empty"><strong>当前快照还没有可展示的观点变化</strong><p>生成快照后，模型会在可匹配来源的范围内提取观点演化。</p></div> : groups.map(([topic, topicItems]) => <section className="evolution-topic" key={topic}><div className="evolution-topic-heading"><h3>{topic}</h3><span>{topicItems.length} 次变化</span></div><div className="evolution-chain">{topicItems.map((item) => <EvolutionCard key={item.id} item={item} feedback={feedback.find((entry) => entry.itemId === item.id)} busy={feedbackBusy.has(`${data.selectedSnapshot?.id}:${item.id}`)} onDecide={decide} onOpenSource={onOpenSource} />)}</div></section>)}
      <Watchlist title="长期未推进的问题" items={data.selectedSnapshot.result.dormantQuestions} empty="没有识别到长期未推进的问题。" />
      <Watchlist title="持续有任务但缺少里程碑变化的项目" items={data.selectedSnapshot.result.stalledProjects} empty="没有识别到停滞项目。" />
    </div>}
  </section>;
}

function EvolutionCard({ item, feedback, busy, onDecide, onOpenSource }: { item: EvolutionItem; feedback?: MemoryFeedback; busy: boolean; onDecide: (item: EvolutionItem, decision: "confirmed" | "rejected") => Promise<void>; onOpenSource: (source: MemorySourceReference) => void }) {
  return <article className={`evolution-card${item.inferred ? " inferred" : ""}`} aria-busy={busy}><div className="evolution-card-heading"><span className={`change-type change-${item.changeType}`}>{item.changeType}</span><time dateTime={item.occurredAt}>{formatMemoryDate(item.occurredAt, { month: "numeric", day: "numeric", hour: "2-digit", minute: "2-digit" })}</time>{item.inferred && <span className="inferred-badge">待确认推断</span>}{feedback && <span className={`feedback-badge ${feedback.decision}`}>{feedback.decision === "confirmed" ? "已确认" : "已驳回"}</span>}</div><div className="evolution-before-after"><div><span>之前</span><p>{item.beforeText || "—"}</p></div><div className="evolution-arrow">→</div><div><span>之后</span><p>{item.afterText || "—"}</p></div></div>{item.reason && <p className="evolution-reason"><strong>变化原因：</strong>{item.reason}</p>}{item.confidence !== null && <span className="confidence-label">置信度 {Math.round(item.confidence * 100)}%</span>}<div className="evolution-card-footer"><div className="source-list">{item.sources.map((source, index) => <button type="button" className="source-reference" key={`${source.recordId}-${source.segmentId}-${index}`} onClick={() => onOpenSource(source)}><span>证据 {index + 1}</span><span>{source.quoteText || "跳转到逐字稿"}</span></button>)}</div><div className="feedback-actions"><button type="button" className="secondary-button" disabled={busy} onClick={() => void onDecide(item, "rejected")}>驳回</button><button type="button" className="primary-button" disabled={busy} onClick={() => void onDecide(item, "confirmed")}>确认</button></div></div>{feedback?.note && <p className="feedback-note">备注：{feedback.note}</p>}</article>;
}

function NoSnapshotEmpty() {
  return <div className="memory-empty"><strong>还没有认知演化快照</strong><p>生成快照后，模型会在可匹配来源的范围内提取观点演化。</p></div>;
}

function isExternalAiReady(settings: { enabled: boolean; hasApiKey: boolean; privacyConsentAt: string | null }) {
  return settings.enabled && settings.hasApiKey && Boolean(settings.privacyConsentAt);
}

function ConfiguredEmpty({ onOpenSettings }: { onOpenSettings: () => void }) {
  return <div className="memory-config-empty"><div className="memory-config-icon"><SparklesIcon size={22} /></div><h3>认知演化需要外部 AI</h3><p>启用后，模型只分析你确认发送的文本；音频文件永不上传。</p><button type="button" className="primary-button" onClick={onOpenSettings}>打开外部 AI 设置</button></div>;
}

function Watchlist({ title, items, empty }: { title: string; items: string[]; empty: string }) {
  return <section className="evolution-watchlist"><div className="evolution-topic-heading"><h3>{title}</h3><span>{items.length} 项</span></div>{items.length ? <ul>{items.map((item) => <li key={item}>{item}</li>)}</ul> : <p>{empty}</p>}</section>;
}

function groupByTopic(items: EvolutionItem[]) {
  const groups = new Map<string, EvolutionItem[]>();
  for (const item of items) groups.set(item.topic || "未分类主题", [...(groups.get(item.topic || "未分类主题") ?? []), item]);
  return Array.from(groups.entries()).sort(([a], [b]) => a.localeCompare(b));
}

function errorMessage(reason: unknown) {
  return reason instanceof Error ? reason.message : String(reason);
}
