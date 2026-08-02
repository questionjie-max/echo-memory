import assert from "node:assert/strict";
import test from "node:test";
import { buildGrowthTimelineDays, localDayKey } from "../src/components/growthTimeline.ts";
import type { TimelineItem } from "../src/shared/types.ts";

function item(overrides: Partial<TimelineItem> & Pick<TimelineItem, "id" | "itemType" | "title" | "occurredAt" | "sources">): TimelineItem {
  return {
    summary: overrides.title,
    projectId: null,
    projectName: null,
    inferred: false,
    confidence: null,
    ...overrides,
  };
}

function source(recordId: string) {
  return { recordId, segmentId: null, startMs: null, endMs: null, quoteText: "" };
}

test("同一录音的结论、待办和观点只计为一条记录", () => {
  const items = [
    item({ id: "r1", itemType: "record", title: "周会", occurredAt: "2026-08-02T09:00:00+08:00", sources: [source("record-1")] }),
    item({ id: "d1", itemType: "decision", title: "下周发布", occurredAt: "2026-08-02T09:00:00+08:00", sources: [source("record-1")] }),
    item({ id: "t1", itemType: "task", title: "补齐测试", occurredAt: "2026-08-02T09:00:00+08:00", sources: [source("record-1")] }),
    item({ id: "v1", itemType: "viewpoint", title: "先稳定再扩展", occurredAt: "2026-08-02T09:00:00+08:00", sources: [source("record-1")] }),
  ];
  const [day] = buildGrowthTimelineDays(items, []);
  assert.equal(day.recordCount, 1);
  assert.equal(day.projects[0].records[0].insights.length, 3);
});

test("同一天的多条记录合并为一个日期节点并按项目分组", () => {
  const items = [
    item({ id: "r1", itemType: "record", title: "项目会", occurredAt: "2026-08-02T09:00:00+08:00", projectId: "p1", projectName: "发布项目", sources: [source("record-1")] }),
    item({ id: "r2", itemType: "record", title: "访谈", occurredAt: "2026-08-02T15:00:00+08:00", projectId: "p2", projectName: "用户研究", sources: [source("record-2")] }),
    item({ id: "r3", itemType: "record", title: "随手记", occurredAt: "2026-08-02T18:00:00+08:00", sources: [source("record-3")] }),
  ];
  const [day] = buildGrowthTimelineDays(items, []);
  assert.equal(day.key, localDayKey("2026-08-02T09:00:00+08:00"));
  assert.equal(day.recordCount, 3);
  assert.equal(day.projectCount, 2);
  assert.equal(day.unfiledRecordCount, 1);
  assert.deepEqual(day.projects.map((project) => project.name), ["发布项目", "用户研究", "未归档"]);
});

test("日期节点按最近日期优先排列", () => {
  const days = buildGrowthTimelineDays([
    item({ id: "older", itemType: "record", title: "旧记录", occurredAt: "2026-07-30T10:00:00+08:00", sources: [source("old")] }),
    item({ id: "newer", itemType: "record", title: "新记录", occurredAt: "2026-08-02T10:00:00+08:00", sources: [source("new")] }),
  ], []);
  assert.deepEqual(days.map((day) => day.key), ["2026-08-02", "2026-07-30"]);
});

test("AI 快照不会重复本地记录或重复相同洞察", () => {
  const local = [
    item({ id: "local-record", itemType: "record", title: "本地标题", occurredAt: "2026-08-02T10:00:00+08:00", sources: [source("record-1")] }),
    item({ id: "local-task", itemType: "task", title: "完成原型", occurredAt: "2026-08-02T10:00:00+08:00", sources: [source("record-1")] }),
  ];
  const generated = [
    item({ id: "ai-record", itemType: "record", title: "AI 标题", occurredAt: "2026-08-02T10:00:00+08:00", inferred: true, sources: [source("record-1")] }),
    item({ id: "ai-task", itemType: "task", title: "完成原型", occurredAt: "2026-08-02T10:00:00+08:00", inferred: true, sources: [source("record-1")] }),
  ];
  const [day] = buildGrowthTimelineDays(local, generated);
  assert.equal(day.recordCount, 1);
  assert.equal(day.projects[0].records[0].title, "本地标题");
  assert.equal(day.projects[0].records[0].insights.length, 1);
  assert.equal(day.projects[0].records[0].insights[0].inferred, false);
});

test("无效日期和没有记录来源的数据不会生成虚假节点", () => {
  const days = buildGrowthTimelineDays([
    item({ id: "bad-date", itemType: "record", title: "错误日期", occurredAt: "invalid", sources: [source("record-1")] }),
    item({ id: "no-source", itemType: "record", title: "无来源", occurredAt: "2026-08-02T10:00:00+08:00", sources: [] }),
  ], []);
  assert.deepEqual(days, []);
});

test("按本地时区而不是 UTC 字符串切片归入日期", () => {
  const value = "2026-08-01T18:30:00Z";
  const date = new Date(value);
  const expected = `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}-${String(date.getDate()).padStart(2, "0")}`;
  assert.equal(localDayKey(value), expected);
});

test("不存在的日历日期不会被自动归一化", () => {
  assert.equal(localDayKey("2026-02-30T10:00:00+08:00"), null);
});

test("纯日期保持原日期，不经过 UTC 时区换算", () => {
  assert.equal(localDayKey("2026-08-02"), "2026-08-02");
});


test("跨记录洞察保留全部引用且不增加记录计数", () => {
  const items = [
    item({ id: "record-1", itemType: "record", title: "产品会", occurredAt: "2026-08-02T09:00:00+08:00", sources: [source("r1")] }),
    item({ id: "record-2", itemType: "record", title: "复盘会", occurredAt: "2026-08-02T15:00:00+08:00", sources: [source("r2")] }),
    item({ id: "shared", itemType: "decision", title: "统一发布节奏", occurredAt: "2026-08-02T15:00:00+08:00", sources: [source("r1"), source("r2")] }),
  ];
  const [day] = buildGrowthTimelineDays(items, []);
  const insight = day.projects.flatMap((project) => project.records).find((record) => record.recordId === "r1")?.insights[0];
  assert.equal(day.recordCount, 2);
  assert.deepEqual(insight?.sources.map((entry) => entry.recordId), ["r1", "r2"]);
});
