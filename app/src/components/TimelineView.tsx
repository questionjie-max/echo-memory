import { useMemo, useState } from "react";
import MemoryViewControls from "./MemoryViewControls";
import type { MemoryScope, MemorySourceReference, TimelineItem } from "../shared/types";
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

const TYPE_LABELS: Record<string, string> = {
  record: "记录",
  decision: "决策",
  task: "任务",
  viewpoint: "观点",
  event: "事件",
};

export default function TimelineView({ scope, onOpenSource, onOpenSettings }: Props) {
  const [range, setRange] = useState<MemoryRangeKey>("30d");
  const data = useMemoryViewData("timeline", scope, range);
  const items = useMemo(() => mergeItems(data.localItems, data.selectedSnapshot?.result.timelineItems ?? []), [data.localItems, data.selectedSnapshot]);
  const groups = useMemo(() => groupByDay(items), [items]);

  return (
    <section className="memory-view timeline-view">
      <MemoryViewControls
        title="时间轴"
        description="按时间回看会议、项目、决策、任务和观点；跨记录推断始终保留来源引用。"
        viewKind="timeline"
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
      {data.loading ? (
        <div className="memory-empty" role="status" aria-live="polite"><strong>正在加载时间轴…</strong></div>
      ) : data.error ? (
        <div className="memory-empty memory-error-state" role="alert"><strong>时间轴加载失败</strong><p>{data.error}</p><button type="button" className="secondary-button" onClick={() => void data.reload()}>重试</button></div>
      ) : groups.length === 0 ? <EmptyTimeline /> : (
        <div className="timeline-groups">
          {groups.map(([day, dayItems]) => <section className="timeline-day" key={day}>
            <div className="timeline-day-heading"><span>{formatMemoryDate(`${day}T00:00:00`, { month: "long", day: "numeric", weekday: "short" })}</span><span>{dayItems.length} 项</span></div>
            <div className="timeline-items">
              {dayItems.map((item) => <TimelineCard item={item} key={item.id} onOpenSource={onOpenSource} />)}
            </div>
          </section>)}
        </div>
      )}
    </section>
  );
}

function TimelineCard({ item, onOpenSource }: { item: TimelineItem; onOpenSource: (source: MemorySourceReference) => void }) {
  const source = item.sources[0];
  return <article className={`timeline-card${item.inferred ? " inferred" : ""}`}>
    <div className="timeline-card-marker" aria-hidden="true" />
    <div className="timeline-card-body">
      <div className="timeline-card-meta">
        <span className="memory-type-badge">{TYPE_LABELS[item.itemType] ?? item.itemType}</span>
        <time dateTime={item.occurredAt}>{formatMemoryDate(item.occurredAt, { hour: "2-digit", minute: "2-digit" })}</time>
        {item.projectName && <span>{item.projectName}</span>}
        {item.inferred && <span className="inferred-badge">模型推断</span>}
      </div>
      <h3>{item.title}</h3>
      {item.summary && item.summary !== item.title && <p>{item.summary}</p>}
      {item.confidence !== null && <span className="confidence-label">置信度 {Math.round(item.confidence * 100)}%</span>}
      {source && <button type="button" className="source-reference" onClick={() => onOpenSource(source)}>
        <span>查看来源</span><span>{source.quoteText || "跳转到逐字稿"}</span>
      </button>}
    </div>
  </article>;
}

function EmptyTimeline() {
  return <div className="memory-empty"><strong>当前范围还没有时间轴内容</strong><p>导入并完成转写后，记录会先以本地确定性数据出现在这里。</p></div>;
}

function mergeItems(localItems: TimelineItem[], generatedItems: TimelineItem[]) {
  const merged = new Map<string, TimelineItem>();
  for (const item of localItems) merged.set(item.id, item);
  for (const item of generatedItems) merged.set(item.id, item);
  return Array.from(merged.values()).sort((a, b) => b.occurredAt.localeCompare(a.occurredAt) || a.id.localeCompare(b.id));
}

function groupByDay(items: TimelineItem[]) {
  const groups = new Map<string, TimelineItem[]>();
  for (const item of items) {
    const day = item.occurredAt.slice(0, 10);
    const group = groups.get(day) ?? [];
    group.push(item);
    groups.set(day, group);
  }
  return Array.from(groups.entries()).sort(([a], [b]) => b.localeCompare(a));
}
