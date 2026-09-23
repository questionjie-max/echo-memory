import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import GrowthView from "../../src/components/GrowthView";
import * as tauri from "../../src/lib/tauri";
import type { ExternalAiSettings, MemoryScope, TimelineItem } from "../../src/shared/types";
import { externalAiSettingsFixture } from "./settings-mocks";

vi.mock("../../src/lib/tauri");

const scope: MemoryScope = { kind: "all", projectId: null };

function timelineFixture(overrides: Partial<TimelineItem> = {}): TimelineItem {
  return {
    id: "timeline-1",
    occurredAt: "2026-09-23T09:30:00.000Z",
    itemType: "record",
    title: "产品评审",
    summary: "确认先完成质量门禁。",
    projectId: "p1",
    projectName: "产品研发",
    inferred: false,
    confidence: 1,
    sources: [
      {
        recordId: "r1",
        segmentId: "s1",
        startMs: 0,
        endMs: 1000,
        quoteText: "先完成质量门禁。",
      },
    ],
    ...overrides,
  };
}

function renderGrowth() {
  return render(
    <GrowthView
      scope={scope}
      onOpenSource={() => undefined}
      onOpenSettings={() => undefined}
    />,
  );
}

function settings(): ExternalAiSettings {
  return externalAiSettingsFixture({
    enabled: true,
    hasApiKey: true,
    privacyConsentAt: "2026-09-23T10:00:00.000Z",
  });
}

beforeEach(() => {
  vi.resetAllMocks();
  vi.mocked(tauri.getExternalAiSettings).mockResolvedValue(settings());
  vi.mocked(tauri.getLocalTimeline).mockResolvedValue([]);
  vi.mocked(tauri.listMemorySnapshots).mockResolvedValue([]);
});

describe("成长轨迹", () => {
  it("数据读取期间显示加载态", async () => {
    vi.mocked(tauri.getLocalTimeline).mockReturnValue(new Promise(() => undefined));
    vi.mocked(tauri.listMemorySnapshots).mockReturnValue(new Promise(() => undefined));
    renderGrowth();

    expect(await screen.findByText("正在整理成长轨迹…")).toBeTruthy();
  });

  it("无数据时显示空轨迹", async () => {
    renderGrowth();

    expect(await screen.findByText("当前范围还没有记录")).toBeTruthy();
  });

  it("全部数据源失败时显示错误并可重试", async () => {
    vi.mocked(tauri.getExternalAiSettings).mockRejectedValue(new Error("设置不可用"));
    vi.mocked(tauri.getLocalTimeline).mockRejectedValue(new Error("本地时间轴不可用"));
    vi.mocked(tauri.listMemorySnapshots).mockRejectedValue(new Error("快照不可用"));
    renderGrowth();

    await screen.findByText("成长轨迹加载失败");
    expect(screen.getByRole("alert").textContent).toContain("本地时间轴不可用");
    const before = vi.mocked(tauri.getLocalTimeline).mock.calls.length;
    fireEvent.click(screen.getByRole("button", { name: "重试" }));
    await waitFor(() =>
      expect(vi.mocked(tauri.getLocalTimeline).mock.calls.length).toBeGreaterThan(before),
    );
  });

  it("部分数据源失败时保留已加载记录并显示警告", async () => {
    vi.mocked(tauri.listMemorySnapshots).mockRejectedValue(new Error("快照不可用"));
    vi.mocked(tauri.getLocalTimeline).mockResolvedValue([timelineFixture()]);
    renderGrowth();

    fireEvent.click(await screen.findByRole("button", { name: /1 条记录/ }));
    await screen.findByText("产品评审");
    expect(screen.getByRole("alert").textContent).toContain("部分数据读取失败");
  });

  it("按天节点可展开并打开源记录", async () => {
    vi.mocked(tauri.getLocalTimeline).mockResolvedValue([timelineFixture()]);
    const onOpenSource = vi.fn();
    render(
      <GrowthView
        scope={scope}
        onOpenSource={onOpenSource}
        onOpenSettings={() => undefined}
      />,
    );

    const trigger = await screen.findByRole("button", { name: /1 条记录/ });
    fireEvent.click(trigger);
    expect(trigger.getAttribute("aria-expanded")).toBe("true");
    await screen.findByText("产品评审");
    fireEvent.click(screen.getByRole("button", { name: "打开记录" }));
    expect(onOpenSource).toHaveBeenCalledWith(
      expect.objectContaining({ recordId: "r1", segmentId: "s1" }),
    );
  });
});
