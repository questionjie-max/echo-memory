import type { MemorySourceReference, TimelineItem } from "../shared/types";

export interface GrowthTimelineInsight {
  id: string;
  itemType: string;
  title: string;
  summary: string;
  inferred: boolean;
  confidence: number | null;
  sources: MemorySourceReference[];
}

export interface GrowthTimelineRecord {
  recordId: string;
  occurredAt: string;
  title: string;
  summary: string;
  projectId: string | null;
  projectName: string | null;
  inferred: boolean;
  source: MemorySourceReference;
  insights: GrowthTimelineInsight[];
}

export interface GrowthTimelineProject {
  key: string;
  projectId: string | null;
  name: string;
  records: GrowthTimelineRecord[];
}

export interface GrowthTimelineDay {
  key: string;
  recordCount: number;
  projectCount: number;
  unfiledRecordCount: number;
  projects: GrowthTimelineProject[];
}

interface RecordAccumulator {
  recordId: string;
  root: TimelineItem | null;
  fallback: TimelineItem;
  insightByKey: Map<string, TimelineItem>;
}

export function buildGrowthTimelineDays(
  localItems: TimelineItem[],
  generatedItems: TimelineItem[],
): GrowthTimelineDay[] {
  const records = new Map<string, RecordAccumulator>();

  for (const item of mergeTimelineItems(localItems, generatedItems)) {
    const source = primarySource(item);
    if (!source || !localDayKey(item.occurredAt)) continue;

    const current = records.get(source.recordId) ?? {
      recordId: source.recordId,
      root: null,
      fallback: item,
      insightByKey: new Map<string, TimelineItem>(),
    };

    if (item.itemType === "record") {
      if (!current.root || rootPriority(item) > rootPriority(current.root)) {
        current.root = item;
      }
    } else {
      const key = insightKey(item);
      const existing = current.insightByKey.get(key);
      if (!existing || rootPriority(item) > rootPriority(existing)) {
        current.insightByKey.set(key, item);
      }
    }
    records.set(source.recordId, current);
  }

  const days = new Map<string, GrowthTimelineRecord[]>();
  for (const accumulator of records.values()) {
    const root = accumulator.root ?? accumulator.fallback;
    const dayKey = localDayKey(root.occurredAt);
    const source = sourceForRecord(root, accumulator.recordId);
    if (!dayKey || !source) continue;

    const record: GrowthTimelineRecord = {
      recordId: accumulator.recordId,
      occurredAt: root.occurredAt,
      title: root.itemType === "record" ? root.title : "记录详情",
      summary:
        root.itemType === "record"
          ? root.summary
          : "这条记录包含 AI 提取的结构化内容。",
      projectId: root.projectId,
      projectName: root.projectName,
      inferred: root.inferred,
      source,
      insights: Array.from(accumulator.insightByKey.values())
        .map((item) => ({
          id: item.id,
          itemType: item.itemType,
          title: item.title,
          summary: item.summary,
          inferred: item.inferred,
          confidence: item.confidence,
          sources: uniqueSources(item.sources),
        }))
        .sort(compareInsights),
    };
    days.set(dayKey, [...(days.get(dayKey) ?? []), record]);
  }

  return Array.from(days.entries())
    .sort(([left], [right]) => right.localeCompare(left))
    .map(([key, dayRecords]) => buildDay(key, dayRecords));
}

export function localDayKey(value: string): string | null {
  const match = /^(\d{4})-(\d{2})-(\d{2})(?:$|T)/.exec(value);
  if (!match) return null;

  const [, yearText, monthText, dayText] = match;
  const year = Number(yearText);
  const month = Number(monthText);
  const day = Number(dayText);
  if (
    month < 1 ||
    month > 12 ||
    day < 1 ||
    day > new Date(Date.UTC(year, month, 0)).getUTCDate()
  ) {
    return null;
  }

  if (value.length === 10) return value;

  const date = new Date(value);
  if (!Number.isFinite(date.getTime())) return null;
  return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}-${String(date.getDate()).padStart(2, "0")}`;
}

function mergeTimelineItems(
  localItems: TimelineItem[],
  generatedItems: TimelineItem[],
) {
  const merged = new Map<string, TimelineItem>();
  for (const item of generatedItems) merged.set(item.id, item);
  for (const item of localItems) merged.set(item.id, item);
  return Array.from(merged.values());
}

function buildDay(
  key: string,
  records: GrowthTimelineRecord[],
): GrowthTimelineDay {
  const projects = new Map<string, GrowthTimelineProject>();
  for (const record of records.sort(compareRecords)) {
    const projectKey = record.projectId
      ? `id:${record.projectId}`
      : record.projectName
        ? `name:${record.projectName}`
        : "unfiled";
    const project = projects.get(projectKey) ?? {
      key: projectKey,
      projectId: record.projectId,
      name: record.projectName || "未归档",
      records: [],
    };
    project.records.push(record);
    projects.set(projectKey, project);
  }

  const groups = Array.from(projects.values()).sort((left, right) => {
    if (left.key === "unfiled") return 1;
    if (right.key === "unfiled") return -1;
    return left.name.localeCompare(right.name, "zh-CN");
  });

  return {
    key,
    recordCount: records.length,
    projectCount: groups.filter((project) => project.key !== "unfiled").length,
    unfiledRecordCount:
      groups.find((project) => project.key === "unfiled")?.records.length ?? 0,
    projects: groups,
  };
}

function primarySource(item: TimelineItem) {
  return item.sources.find((source) => Boolean(source.recordId)) ?? null;
}

function sourceForRecord(item: TimelineItem, recordId: string) {
  return (
    item.sources.find((source) => source.recordId === recordId) ??
    primarySource(item)
  );
}

function uniqueSources(sources: MemorySourceReference[]) {
  const unique = new Map<string, MemorySourceReference>();
  for (const source of sources) {
    if (!source.recordId) continue;
    const key = `${source.recordId}:${source.segmentId ?? ""}:${source.startMs ?? ""}:${source.endMs ?? ""}`;
    if (!unique.has(key)) unique.set(key, source);
  }
  return Array.from(unique.values());
}

function rootPriority(item: TimelineItem) {
  return (item.itemType === "record" ? 2 : 0) + (item.inferred ? 0 : 1);
}

function insightKey(item: TimelineItem) {
  return `${item.itemType}:${item.title.trim().toLocaleLowerCase("zh-CN")}`;
}

function compareRecords(left: GrowthTimelineRecord, right: GrowthTimelineRecord) {
  return (
    new Date(right.occurredAt).getTime() - new Date(left.occurredAt).getTime() ||
    left.recordId.localeCompare(right.recordId)
  );
}

function compareInsights(
  left: GrowthTimelineInsight,
  right: GrowthTimelineInsight,
) {
  return (
    insightOrder(left.itemType) - insightOrder(right.itemType) ||
    left.title.localeCompare(right.title, "zh-CN")
  );
}

function insightOrder(itemType: string) {
  return { decision: 0, task: 1, viewpoint: 2, event: 3 }[itemType] ?? 4;
}
