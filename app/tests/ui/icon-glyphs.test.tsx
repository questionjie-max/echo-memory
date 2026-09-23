/**
 * 图标字形回归防护：状态标记一律是 SVG 图标，不回退成 emoji 或符号字形。
 *
 * 背景：v0.6 把全软件的 📮🤖✨✦ 与 ✓✗○▸⌾＋ 换成了 Lucide 内联 SVG。
 * 这些组件里一部分（Pill/ListRow）在浏览器验证时被加载态挡住，必须在 jsdom 里
 * 逐状态点名验证；另外整屏违禁字符扫描防止任何一处悄悄回退。
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import { Pill, ListRow, type RowState } from "../../src/components/SettingsKit";
import SettingsPanel from "../../src/components/SettingsPanel";
import EvolutionView from "../../src/components/EvolutionView";
import DocumentImportDialog from "../../src/components/DocumentImportDialog";
import * as tauri from "../../src/lib/tauri";
import { setupSettingsMocks } from "./settings-mocks";

vi.mock("@tauri-apps/api/core", () => ({ isTauri: () => true }));
vi.mock("@tauri-apps/api/event", () => ({ listen: () => Promise.resolve(() => undefined) }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: () => Promise.resolve(null) }));
vi.mock("../../src/lib/tauri");

/** 被替换掉的 emoji 与符号字形（accent 药丸的 • 圆点是设计的一部分，不在此列）。 */
const BANNED = ["📮", "🤖", "✨", "✦", "⌾", "＋", "✓", "✗", "○", "▸"];

function assertNoBannedGlyphs(container: HTMLElement) {
  const text = container.textContent ?? "";
  for (const ch of BANNED) {
    expect(`${ch}:${text.includes(ch)}`).toBe(`${ch}:false`);
  }
}

describe("状态药丸与列表行：字形必须是 SVG 图标", () => {
  it("Pill 四档 tone 的字形位都是图标（accent 保留 • 圆点）", () => {
    const { container } = render(
      <>
        <Pill tone="ok">已就绪</Pill>
        <Pill tone="warn">待检测</Pill>
        <Pill tone="err">失败</Pill>
        <Pill tone="accent">标记</Pill>
        <Pill>普通</Pill>
      </>,
    );
    const glyphs = container.querySelectorAll(".em-pill-glyph");
    // ok/warn/err 各一个 SVG；accent 是文本 •；default 无字形位 → 共 4 个字形位
    expect(glyphs.length).toBe(4);
    const [ok, warn, err, accent] = [...glyphs] as [HTMLElement, HTMLElement, HTMLElement, HTMLElement];
    for (const g of [ok, warn, err]) {
      const svg = g.querySelector("svg");
      expect(svg).toBeTruthy();
      expect(svg?.getAttribute("viewBox")).toBe("0 0 24 24");
      expect(svg?.getAttribute("stroke")).toBe("currentColor");
      expect(svg?.getAttribute("aria-hidden")).toBe("true");
    }
    expect(accent.querySelector("svg")).toBeNull();
    expect(accent.textContent).toBe("•");
    assertNoBannedGlyphs(container);
  });

  it("ListRow 四种状态：idle/done/error 是 SVG，working 是 CSS 旋转环", () => {
    const states: RowState[] = ["idle", "done", "working", "error"];
    const { container } = render(
      <>
        {states.map((state) => (
          <ListRow key={state} state={state} title={`行-${state}`} onSelect={() => undefined} />
        ))}
      </>,
    );
    const glyphs = [...container.querySelectorAll(".em-row-glyph")] as HTMLElement[];
    expect(glyphs.length).toBe(4);
    for (const g of glyphs) {
      const state = g.className;
      if (state.includes("working")) {
        expect(g.querySelector("svg")).toBeNull();
        expect(g.textContent).toBe("");
        expect(g.className).toContain("working");
      } else {
        const svg = g.querySelector("svg");
        expect(svg).toBeTruthy();
        expect(svg?.getAttribute("viewBox")).toBe("0 0 24 24");
      }
    }
    assertNoBannedGlyphs(container);
  });
});

describe("整屏渲染不出现违禁字形", () => {
  beforeEach(() => {
    vi.resetAllMocks();
    setupSettingsMocks(tauri);
    // 演化页与 hook 需要的额外读取
    vi.mocked(tauri.listMemorySnapshots).mockResolvedValue([]);
    vi.mocked(tauri.getLocalTimeline).mockResolvedValue([]);
    vi.mocked(tauri.getMemoryGenerationJob).mockResolvedValue(null);
    vi.mocked(tauri.listMemoryFeedback).mockResolvedValue([]);
    vi.mocked(tauri.getRecord).mockResolvedValue(null);
    vi.mocked(tauri.importDocument).mockResolvedValue(null as never);
  });

  it("设置面板加载后：药丸字形全为图标，整屏无违禁字符", async () => {
    const { container } = render(<SettingsPanel open onClose={() => undefined} />);
    await screen.findByText("分析模板", { selector: "button" });
    await screen.findByText(/谁来把录音转成文字/);
    assertNoBannedGlyphs(container);
    const pillGlyphs = [...container.querySelectorAll(".em-pill-glyph")] as HTMLElement[];
    expect(pillGlyphs.length).toBeGreaterThan(0);
    for (const g of pillGlyphs) {
      const isAccentDot = g.textContent === "•" && !g.querySelector("svg");
      expect(isAccentDot || g.querySelector("svg") !== null).toBe(true);
    }
    // 折叠区 chevron 伪元素与入场动画由浏览器侧探针验证（jsdom 不算伪元素样式）
  });

  it("认知演化空状态：✦ 已换成 SparklesIcon", async () => {
    const { container } = render(
      <EvolutionView scope="all" onOpenSource={() => undefined} onOpenSettings={() => undefined} />,
    );
    const empty = await screen.findByText("认知演化需要外部 AI");
    expect(empty).toBeTruthy();
    const iconBox = container.querySelector(".memory-config-icon");
    expect(iconBox).toBeTruthy();
    const svg = iconBox?.querySelector("svg");
    expect(svg).toBeTruthy();
    expect(svg?.getAttribute("viewBox")).toBe("0 0 24 24");
    expect(iconBox?.textContent ?? "").toBe("");
    assertNoBannedGlyphs(container);
  });

  it("文档导入对话框默认态：＋/⌾ 已换成 PlusIcon/ShieldCheckIcon", async () => {
    const { container } = render(
      <DocumentImportDialog open projectId="p1" onClose={() => undefined} onImported={() => undefined} />,
    );
    await screen.findByText("选择一个文字文档");
    const pickerSvg = container.querySelector(".document-import-picker-icon svg");
    expect(pickerSvg).toBeTruthy();
    const privacySvg = container.querySelector(".document-import-privacy > span svg");
    expect(privacySvg).toBeTruthy();
    assertNoBannedGlyphs(container);
  });
});
