import { useEffect, useMemo, useState } from "react";
import MemoryViewControls from "./MemoryViewControls";
import type {
  MemoryScope,
  MemorySourceReference,
} from "../shared/types";
import {
  formatMemoryDate,
  useMemoryViewData,
  type MemoryRangeKey,
} from "../hooks/useMemoryViewData";
import {
  buildGrowthTimelineDays,
  type GrowthTimelineDay,
  type GrowthTimelineInsight,
  type GrowthTimelineRecord,
} from "./growthTimeline";

interface Props {
  scope: MemoryScope;
  onOpenSource: (source: MemorySourceReference) => void;
  onOpenSettings: () => void;
}

const INSIGHT_LABELS: Record<string, string> = {
  decision: "结论",
  task: "待办",
  viewpoint: "观点",
  event: "事件",
  question: "问题",
};

export default function GrowthView({
  scope,
  onOpenSource,
  onOpenSettings,
}: Props) {
  const [range, setRange] = useState<MemoryRangeKey>("30d");
  const [expandedDay, setExpandedDay] = useState<string | null>(null);
  const data = useMemoryViewData("timeline", scope, range);
  const days = useMemo(
    () =>
      buildGrowthTimelineDays(
        data.localItems,
        data.selectedSnapshot?.result.timelineItems ?? [],
      ),
    [data.localItems, data.selectedSnapshot],
  );

  useEffect(() => {
    setExpandedDay(null);
  }, [scope.kind, scope.projectId, range, data.selectedSnapshot?.id]);

  return (
    <section className="memory-view growth-view">
      <MemoryViewControls
        title="成长轨迹"
        description="一天一个节点：先看当天记录数量，点击后再展开项目和记录。"
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
        <div className="memory-empty" role="status" aria-live="polite">
          <strong>正在整理成长轨迹…</strong>
        </div>
      ) : days.length === 0 && data.error ? (
        <LoadFailure message={data.error} onRetry={() => void data.reload()} />
      ) : days.length === 0 ? (
        <EmptyGrowthTimeline />
      ) : (
        <div className="growth-timeline-shell">
          {data.error && (
            <div className="growth-inline-warning" role="alert">
              <span>部分数据读取失败，当前先展示已加载的记录。</span>
              <button type="button" onClick={() => void data.reload()}>
                重试
              </button>
            </div>
          )}
          <div className="growth-timeline" aria-label="按天排列的成长轨迹">
            {days.map((day) => (
              <GrowthDayNode
                key={day.key}
                day={day}
                expanded={expandedDay === day.key}
                onToggle={() =>
                  setExpandedDay((current) =>
                    current === day.key ? null : day.key,
                  )
                }
                onOpenSource={onOpenSource}
              />
            ))}
          </div>
        </div>
      )}
    </section>
  );
}

function GrowthDayNode({
  day,
  expanded,
  onToggle,
  onOpenSource,
}: {
  day: GrowthTimelineDay;
  expanded: boolean;
  onToggle: () => void;
  onOpenSource: (source: MemorySourceReference) => void;
}) {
  const detailsId = `growth-day-${day.key}`;
  const visibleProjects = day.projects.slice(0, 3);
  const remainingProjects = day.projects.length - visibleProjects.length;

  return (
    <article className={`growth-day-node${expanded ? " expanded" : ""}`}>
      <button
        type="button"
        className="growth-day-trigger"
        aria-expanded={expanded}
        aria-controls={detailsId}
        onClick={onToggle}
      >
        <span className="growth-day-marker" aria-hidden="true" />
        <span className="growth-day-date">
          <time dateTime={day.key}>
            {formatMemoryDate(`${day.key}T12:00:00`, {
              month: "long",
              day: "numeric",
              weekday: "short",
            })}
          </time>
          <strong>{day.recordCount} 条记录</strong>
        </span>
        <span className="growth-day-summary">
          <span>
            {day.projectCount > 0
              ? `${day.projectCount} 个项目`
              : "没有归入项目"}
            {day.unfiledRecordCount > 0
              ? ` · ${day.unfiledRecordCount} 条未归档`
              : ""}
          </span>
          <span className="growth-project-preview" aria-hidden="true">
            {visibleProjects.map((project) => (
              <span key={project.key}>{project.name}</span>
            ))}
            {remainingProjects > 0 && <span>+{remainingProjects}</span>}
          </span>
        </span>
        <span className="growth-day-chevron" aria-hidden="true">
          {expanded ? "−" : "+"}
        </span>
      </button>

      {expanded && (
        <div className="growth-day-details" id={detailsId}>
          {day.projects.map((project) => (
            <section className="growth-project-group" key={project.key}>
              <div className="growth-project-heading">
                <h3>{project.name}</h3>
                <span>{project.records.length} 条</span>
              </div>
              <div className="growth-record-list">
                {project.records.map((record) => (
                  <GrowthRecordCard
                    key={record.recordId}
                    record={record}
                    onOpenSource={onOpenSource}
                  />
                ))}
              </div>
            </section>
          ))}
        </div>
      )}
    </article>
  );
}

function GrowthRecordCard({
  record,
  onOpenSource,
}: {
  record: GrowthTimelineRecord;
  onOpenSource: (source: MemorySourceReference) => void;
}) {
  return (
    <article className="growth-record-card">
      <div className="growth-record-heading">
        <time dateTime={record.occurredAt}>
          {formatMemoryDate(record.occurredAt, {
            hour: "2-digit",
            minute: "2-digit",
          })}
        </time>
        <h4>{record.title}</h4>
        {record.inferred && <span className="inferred-badge">模型推断</span>}
      </div>
      {record.summary && record.summary !== record.title && (
        <p>{record.summary}</p>
      )}
      <div className="growth-record-actions">
        <button
          type="button"
          className="secondary-button"
          onClick={() => onOpenSource(record.source)}
        >
          打开记录
        </button>
        {record.insights.length > 0 && (
          <details className="growth-insights">
            <summary>
              查看 {record.insights.length} 项分析
            </summary>
            <div className="growth-insight-list">
              {record.insights.map((insight) => (
                <GrowthInsight
                  key={insight.id}
                  insight={insight}
                  onOpenSource={onOpenSource}
                />
              ))}
            </div>
          </details>
        )}
      </div>
    </article>
  );
}

function GrowthInsight({
  insight,
  onOpenSource,
}: {
  insight: GrowthTimelineInsight;
  onOpenSource: (source: MemorySourceReference) => void;
}) {
  return (
    <div className="growth-insight">
      <span className="memory-type-badge">
        {INSIGHT_LABELS[insight.itemType] ?? insight.itemType}
      </span>
      <div>
        <strong>{insight.title}</strong>
        {insight.summary && insight.summary !== insight.title && (
          <p>{insight.summary}</p>
        )}
      </div>
      {insight.sources.length > 0 && (
        <div className="growth-insight-sources">
          {insight.sources.map((source, index) => (
            <button
              type="button"
              className="link-button"
              key={`${source.recordId}:${source.segmentId ?? ""}:${source.startMs ?? ""}:${index}`}
              onClick={() => onOpenSource(source)}
            >
              {insight.sources.length === 1 ? "查看引用" : `引用 ${index + 1}`}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

function LoadFailure({
  message,
  onRetry,
}: {
  message: string;
  onRetry: () => void;
}) {
  return (
    <div className="memory-empty memory-error-state" role="alert">
      <strong>成长轨迹加载失败</strong>
      <p>{message}</p>
      <button type="button" className="secondary-button" onClick={onRetry}>
        重试
      </button>
    </div>
  );
}

function EmptyGrowthTimeline() {
  return (
    <div className="memory-empty">
      <strong>当前范围还没有记录</strong>
      <p>导入录音或文档后，这里会按天显示当天发生了几件事。</p>
    </div>
  );
}
