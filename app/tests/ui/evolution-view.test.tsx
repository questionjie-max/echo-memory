import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import EvolutionView from "../../src/components/EvolutionView";
import * as tauri from "../../src/lib/tauri";
import type { EvolutionItem, ExternalAiSettings, MemoryScope, MemorySnapshot, MemorySourceReference } from "../../src/shared/types";
import { externalAiSettingsFixture } from "./settings-mocks";

vi.mock("../../src/lib/tauri");

const scope: MemoryScope = { kind: "all", projectId: null };

function source(recordId: string, quoteText: string): MemorySourceReference {
  return { recordId, segmentId: null, startMs: null, endMs: null, quoteText };
}

function evolutionItem(overrides: Partial<EvolutionItem> & Pick<EvolutionItem, "id" | "changeType" | "beforeText" | "afterText" | "reason">): EvolutionItem {
  return {
    topic: "供应商付款流程",
    occurredAt: "2026-09-23T09:30:00.000Z",
    inferred: false,
    confidence: null,
    sources: [],
    ...overrides,
  };
}

function snapshotFixture(items: EvolutionItem[]): MemorySnapshot {
  return {
    id: "snapshot-1",
    viewKind: "evolution",
    scope: { kind: "all", projectId: null },
    rangeStart: null,
    rangeEnd: null,
    status: "completed",
    provider: "external",
    model: "deepseek-v4-flash",
    sourceRecordIds: ["r1", "r2", "r3"],
    requestHash: "hash-1",
    result: {
      timelineItems: [],
      nodes: [],
      edges: [],
      evolutionItems: items,
      dormantQuestions: [],
      stalledProjects: [],
    },
    qualityWarning: null,
    errorMessage: null,
    isStale: false,
    version: 1,
    createdAt: "2026-09-23T09:30:00.000Z",
    updatedAt: "2026-09-23T09:30:00.000Z",
  };
}

function settings(): ExternalAiSettings {
  return externalAiSettingsFixture({
    enabled: true,
    hasApiKey: true,
    privacyConsentAt: "2026-09-23T10:00:00.000Z",
  });
}

function renderEvolution() {
  return render(<EvolutionView scope={scope} onOpenSource={() => undefined} onOpenSettings={() => undefined} />);
}

beforeEach(() => {
  vi.resetAllMocks();
  vi.mocked(tauri.getExternalAiSettings).mockResolvedValue(settings());
  vi.mocked(tauri.getLocalTimeline).mockResolvedValue([]);
  vi.mocked(tauri.listMemorySnapshots).mockResolvedValue([]);
  vi.mocked(tauri.listMemoryFeedback).mockResolvedValue([]);
  vi.mocked(tauri.updateMemoryFeedback).mockResolvedValue({
    id: "f1",
    snapshotId: "snapshot-1",
    itemId: "e1",
    decision: "confirmed",
    note: "",
    createdAt: "2026-09-23T10:00:00.000Z",
    updatedAt: "2026-09-23T10:00:00.000Z",
  });
});

describe("认知演化", () => {
  it("英文枚举渲染为中文标签，不把原文抛给界面", async () => {
    vi.mocked(tauri.listMemorySnapshots).mockResolvedValue([
      snapshotFixture([evolutionItem({
        id: "e1",
        changeType: "added",
        beforeText: "",
        afterText: "开始担心供应商付款周期过长。",
        reason: "",
      })]),
    ]);
    renderEvolution();

    expect(await screen.findByText("新增")).toBeTruthy();
    expect(screen.queryByText("added")).toBeNull();
  });

  it("新增类不显示空壳「之前」栏", async () => {
    vi.mocked(tauri.listMemorySnapshots).mockResolvedValue([
      snapshotFixture([evolutionItem({
        id: "e1",
        changeType: "added",
        beforeText: "",
        afterText: "开始担心供应商付款周期过长。",
        reason: "",
      })]),
    ]);
    renderEvolution();

    await screen.findByText("新增");
    expect(screen.queryByText("之前")).toBeNull();
    expect(screen.getByText("之后")).toBeTruthy();
  });

  it("修正类显示前后对比", async () => {
    vi.mocked(tauri.listMemorySnapshots).mockResolvedValue([
      snapshotFixture([evolutionItem({
        id: "e1",
        changeType: "revised",
        beforeText: "倾向先用现有模板过渡。",
        afterText: "决定引入 OCR，不再手工录入。",
        reason: "新录音里发现模板方案在发票量上来后会崩。",
      })]),
    ]);
    renderEvolution();

    expect(await screen.findByText("修正")).toBeTruthy();
    expect(screen.getByText("之前")).toBeTruthy();
    expect(screen.getByText("倾向先用现有模板过渡。")).toBeTruthy();
    // reason 与 beforeText/afterText/引文均不重复，应完整渲染
    expect(screen.getByText(/为什么变：/)).toBeTruthy();
    expect(screen.getByText(/新录音里发现模板方案在发票量上来后会崩/)).toBeTruthy();
  });

  it("reason 复述观点原文时隐藏，避免同义反复", async () => {
    vi.mocked(tauri.listMemorySnapshots).mockResolvedValue([
      snapshotFixture([evolutionItem({
        id: "e1",
        changeType: "added",
        beforeText: "",
        afterText: "开始担心供应商付款周期过长。",
        reason: "开始担心供应商付款周期过长。",
      })]),
    ]);
    renderEvolution();

    await screen.findByText("新增");
    expect(screen.queryByText(/为什么变：/)).toBeNull();
  });

  it("统计摘要反映记录数、变化数与待确认数", async () => {
    vi.mocked(tauri.listMemorySnapshots).mockResolvedValue([
      snapshotFixture([
        evolutionItem({ id: "e1", changeType: "added", beforeText: "", afterText: "观点一", reason: "", inferred: true }),
        evolutionItem({ id: "e2", changeType: "revised", beforeText: "旧观点", afterText: "观点二", reason: "因为有了新证据", inferred: false, topic: "另一个主题" }),
      ]),
    ]);
    renderEvolution();

    const summary = (await screen.findByText(/基于 \d+ 条记录/)).textContent ?? "";
    expect(summary).toContain("基于 3 条记录");
    expect(summary).toContain("2 个主题");
    expect(summary).toContain("2 次变化");
    expect(summary).toContain("其中 1 条待确认");
  });

  it("确认反馈后徽章与状态刷新", async () => {
    vi.mocked(tauri.listMemorySnapshots).mockResolvedValue([
      snapshotFixture([evolutionItem({
        id: "e1",
        changeType: "added",
        beforeText: "",
        afterText: "开始担心供应商付款周期过长。",
        reason: "",
        inferred: true,
      })]),
    ]);
    // 备注弹窗返回空串（用户直接确认）
    vi.spyOn(window, "prompt").mockReturnValue("");
    renderEvolution();

    await screen.findByText("新增");
    fireEvent.click(screen.getByRole("button", { name: "确认" }));
    expect(await screen.findByText("已确认")).toBeTruthy();
    expect(vi.mocked(tauri.updateMemoryFeedback)).toHaveBeenCalledWith(
      "snapshot-1",
      "e1",
      "confirmed",
      "",
    );
  });

  it("证据按钮跳转到原文", async () => {
    const onOpenSource = vi.fn();
    vi.mocked(tauri.listMemorySnapshots).mockResolvedValue([
      snapshotFixture([evolutionItem({
        id: "e1",
        changeType: "added",
        beforeText: "",
        afterText: "开始担心供应商付款周期过长。",
        reason: "",
        sources: [source("r1", "这个付款周期实在太长了")],
      })]),
    ]);
    render(<EvolutionView scope={scope} onOpenSource={onOpenSource} onOpenSettings={() => undefined} />);

    fireEvent.click(await screen.findByRole("button", { name: /证据 1/ }));
    expect(onOpenSource).toHaveBeenCalledWith(expect.objectContaining({ recordId: "r1" }));
  });

  it("批量确认一次提交全部待确认条目", async () => {
    vi.mocked(tauri.listMemorySnapshots).mockResolvedValue([
      snapshotFixture([
        evolutionItem({ id: "e1", changeType: "added", beforeText: "", afterText: "观点一", reason: "", inferred: true }),
        evolutionItem({ id: "e2", changeType: "added", beforeText: "", afterText: "观点二", reason: "", inferred: true }),
      ]),
    ]);
    vi.mocked(tauri.updateMemoryFeedbackBatch).mockResolvedValue([
      { id: "f1", snapshotId: "snapshot-1", itemId: "e1", decision: "confirmed", note: "", createdAt: "2026-09-23T10:00:00.000Z", updatedAt: "2026-09-23T10:00:00.000Z" },
      { id: "f2", snapshotId: "snapshot-1", itemId: "e2", decision: "confirmed", note: "", createdAt: "2026-09-23T10:00:00.000Z", updatedAt: "2026-09-23T10:00:00.000Z" },
    ]);
    renderEvolution();

    const button = await screen.findByRole("button", { name: "全部确认（2）" });
    fireEvent.click(button);
    await waitFor(() =>
      expect(vi.mocked(tauri.updateMemoryFeedbackBatch)).toHaveBeenCalledWith(
        "snapshot-1",
        [
          { itemId: "e1", decision: "confirmed", note: "" },
          { itemId: "e2", decision: "confirmed", note: "" },
        ],
      ),
    );
    expect(await screen.findByText("已确认 2 条推断。")).toBeTruthy();
    // 确认后无待确认项，批量入口消失
    await waitFor(() => expect(screen.queryByRole("button", { name: /全部确认/ })).toBeNull());
  });
});
