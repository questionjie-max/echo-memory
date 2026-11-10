import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import App from "../../src/App";

vi.mock("@tauri-apps/api/core", () => ({
  isTauri: () => false,
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(() => Promise.resolve(() => undefined)),
}));
vi.mock("../../src/lib/tauri", () => ({
  getOnboardingStatus: vi.fn(() => Promise.resolve({ completed: true })),
  getRecord: vi.fn(),
}));
vi.mock("../../src/lib/dialogs", () => ({
  notify: vi.fn(() => Promise.resolve()),
}));
vi.mock("../../src/components/AppToolbar", () => ({
  default: ({ onSelectView }: { onSelectView: (view: string) => void }) => (
    <button type="button" onClick={() => onSelectView("library")}>首页</button>
  ),
}));
vi.mock("../../src/components/ProjectPanel", () => ({
  default: () => <aside>项目侧栏</aside>,
}));
vi.mock("../../src/components/AssistantDock", () => ({
  default: () => <div>随行助手</div>,
}));
vi.mock("../../src/components/RecordPanel", () => ({
  default: ({ onSelect }: { onSelect: (record: unknown) => void }) => (
    <button
      type="button"
      onClick={() => onSelect({
        id: "record-1",
        title: "已选记录",
        projectId: null,
        projectName: null,
        sourceType: "import",
        audioHash: "hash",
        audioDurationMs: 1,
        importedAt: "2026-01-01T00:00:00Z",
        status: "completed",
        hasTranscript: true,
        hasAnalysis: false,
        analysisStatus: null,
        analysisHasQualityWarning: false,
        lastAnalysisError: null,
        analysisTemplateId: null,
        processingStage: null,
        progressCurrent: 0,
        progressTotal: 0,
        archivedAt: null,
      })}
    >
      打开记录
    </button>
  ),
}));
vi.mock("../../src/components/RecordDetail", () => ({
  default: () => <div>记录详情</div>,
}));
vi.mock("../../src/components/KnowledgeHome", () => ({
  default: () => <div>电脑设备直录</div>,
}));
vi.mock("../../src/components/SettingsPanel", () => ({
  default: () => null,
}));
vi.mock("../../src/components/OnboardingWizard", () => ({
  default: () => null,
}));
vi.mock("../../src/components/KnowledgeChatView", () => ({ default: () => null }));
vi.mock("../../src/components/GrowthView", () => ({ default: () => null }));
vi.mock("../../src/components/EvolutionView", () => ({ default: () => null }));
vi.mock("../../src/components/ActionDashboard", () => ({ default: () => null }));

beforeEach(() => {
  vi.clearAllMocks();
});

describe("主导航", () => {
  it("从记录详情点击首页会回到知识库主页，而不是继续停在详情", async () => {
    render(<App />);

    fireEvent.click(await screen.findByRole("button", { name: "打开记录" }));
    expect(screen.getByText("记录详情")).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "首页" }));
    expect(screen.getByText("电脑设备直录")).toBeTruthy();
    expect(screen.queryByText("记录详情")).toBeNull();
  });
});
