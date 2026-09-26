/**
 * 「AI 模型」tab 的行为测试。
 *
 * 这里盯的是这次重构真正承诺的东西：
 *  1. 点模型行就切换，不存在「设为当前」再按「保存设置」两道门；
 *  2. 本地和外部是同一个决策的两个选项，切过去才出现凭据字段；
 *  3. 下载状态长在模型那一行上，百分比和字节数是真实的；
 *  4. 下载可以取消，没装的引擎不会把用户推进去。
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import SettingsPanel from "../../src/components/SettingsPanel";
import * as tauri from "../../src/lib/tauri";
import {
  downloadProgressFixture,
  engineStatusFixture,
  externalAiSettingsFixture,
  localAiStatusFixture,
  setupSettingsMocks,
} from "./settings-mocks";

const listeners = vi.hoisted(() => ({
  handlers: {} as Record<string, (event: { payload: unknown }) => void>,
}));

vi.mock("@tauri-apps/api/core", () => ({ isTauri: () => true }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: (name: string, handler: (event: { payload: unknown }) => void) => {
    listeners.handlers[name] = handler;
    return Promise.resolve(() => undefined);
  },
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: () => Promise.resolve(null) }));
vi.mock("../../src/lib/tauri");

async function renderPanel() {
  render(<SettingsPanel open onClose={() => undefined} />);
  await screen.findByText("AI 模型", { selector: "button" });
  // 等第一屏数据落地，否则查询会落在「正在读取…」上。
  await screen.findByText("转写", { selector: "h3" });
}

/** 按卡片标题取到那张卡片，避免不同卡片里的同名按钮互相干扰。 */
function card(title: string): HTMLElement {
  const heading = screen.getByText(title, { selector: "h3" });
  return heading.closest("section") as HTMLElement;
}

function emit(name: string, payload: unknown) {
  act(() => {
    listeners.handlers[name]?.({ payload });
  });
}

beforeEach(() => {
  vi.resetAllMocks();
  listeners.handlers = {};
  setupSettingsMocks(tauri);
});

describe("AI 模型：模型列表即选择器", () => {
  it("点已安装的模型行就直接保存，不需要再按保存按钮", async () => {
    vi.mocked(tauri.getLocalAiStatus).mockResolvedValue(
      localAiStatusFixture({
        whisperAvailable: true,
        whisperModelPath: "/models/ggml-large-v3-turbo-q5_0.bin",
        whisperModels: [
          { id: "ggml-large-v3-turbo-q5_0", path: "/models/ggml-large-v3-turbo-q5_0.bin", size: 574_041_195 },
          { id: "ggml-small", path: "/models/ggml-small.bin", size: 487_601_967 },
        ],
      }),
    );
    await renderPanel();

    fireEvent.click(await screen.findByText("ggml-small"));

    await waitFor(() =>
      expect(tauri.updateKnowledgeSettings).toHaveBeenCalledWith(
        expect.objectContaining({ whisperModelPath: "/models/ggml-small.bin" }),
      ),
    );
    // 旧的「保存设置」按钮必须消失，否则又回到两道门。
    expect(screen.queryByText("保存设置", { selector: "button" })).toBeNull();
    expect(screen.queryByText("保存外部 AI 设置", { selector: "button" })).toBeNull();
  });

  it("推荐模型装好之后就不该再出现下载行", async () => {
    // 已安装模型的 id 是文件名去掉后缀（ggml-large-v3-turbo-q5_0），推荐模型的 id 不是
    // 同一个字符串（large-v3-turbo-q5_0）。拿 id 直接比对的话，装好了还会挂着一行「下载」。
    vi.mocked(tauri.getLocalAiStatus).mockResolvedValue(
      localAiStatusFixture({
        whisperAvailable: true,
        whisperModelPath: "/models/ggml-large-v3-turbo-q5_0.bin",
        whisperModels: [
          { id: "ggml-large-v3-turbo-q5_0", path: "/models/ggml-large-v3-turbo-q5_0.bin", size: 574_041_195 },
        ],
      }),
    );
    await renderPanel();

    const transcription = card("转写");
    // 同一个模型只能出现一次，且用同一个可读名字。
    expect(within(transcription).getAllByText("large-v3-turbo（q5_0）").length).toBe(1);
    expect(within(transcription).queryByText("下载", { selector: "button" })).toBeNull();
    expect(within(transcription).queryByText("继续下载", { selector: "button" })).toBeNull();
  });

  it("还没下载推荐模型时给出下载入口和真实体积", async () => {
    await renderPanel();
    const transcription = card("转写");
    expect(within(transcription).getByText("large-v3-turbo（q5_0）")).toBeTruthy();
    expect(within(transcription).getByText(/547MB/)).toBeTruthy();
    expect(within(transcription).getByText("下载", { selector: "button" })).toBeTruthy();
    // 体积和校验值来自后端，界面上不该再出现硬编码的哈希。
    expect(screen.queryByText(/SHA-256/)).toBeNull();
  });

  it("中断过就显示「继续下载」并说明已下多少", async () => {
    vi.mocked(tauri.getLocalAiStatus).mockResolvedValue(
      localAiStatusFixture({ pendingWhisperDownload: { model: "large-v3-turbo-q5_0", bytes: 104_857_600 } }),
    );
    await renderPanel();
    const transcription = card("转写");
    expect(within(transcription).getByText("继续下载", { selector: "button" })).toBeTruthy();
    expect(within(transcription).getByText(/已下载 100MB/)).toBeTruthy();
  });

  it("本地模型不需要凭据，界面上明说这一点", async () => {
    await renderPanel();
    expect(within(card("分析")).getByText(/本地模型不需要 API Key 和接口地址/)).toBeTruthy();
  });

  it("分析模型列表里不出现嵌入模型 —— 拿它写分析只会出垃圾", async () => {
    vi.mocked(tauri.getLocalAiStatus).mockResolvedValue(
      localAiStatusFixture({
        ollamaModels: [
          { name: "qwen2.5:7b", size: 4_683_073_536 },
          { name: "qwen3-embedding:0.6b", size: 639_213_568 },
          { name: "llama3.2:3b", size: 2_019_392_256 },
        ],
      }),
    );
    await renderPanel();

    const analysis = card("分析");
    expect(within(analysis).getByText("qwen2.5:7b")).toBeTruthy();
    expect(within(analysis).getByText("llama3.2:3b")).toBeTruthy();
    expect(within(analysis).queryByText("qwen3-embedding:0.6b")).toBeNull();
    // 嵌入模型仍然出现在知识索引里 —— 那才是它该在的地方。
    expect(within(card("知识索引")).getByText("qwen3-embedding:0.6b")).toBeTruthy();
  });

  it("知识索引只列嵌入模型 —— 拿对话模型算向量会静默污染整份索引", async () => {
    vi.mocked(tauri.getLocalAiStatus).mockResolvedValue(
      localAiStatusFixture({
        ollamaModels: [
          { name: "qwen2.5:7b", size: 4_683_073_536 },
          { name: "qwen3-embedding:0.6b", size: 639_213_568 },
          { name: "llama3.2:3b", size: 2_019_392_256 },
        ],
      }),
    );
    await renderPanel();

    const index = card("知识索引");
    expect(within(index).getByText("qwen3-embedding:0.6b")).toBeTruthy();
    expect(within(index).queryByText("llama3.2:3b")).toBeNull();
    expect(within(index).queryByText("qwen2.5:7b")).toBeNull();
  });

  it("用户自定义的嵌入模型即使名字里没有 embed 也不会从列表里消失", async () => {
    vi.mocked(tauri.getLocalAiStatus).mockResolvedValue(
      localAiStatusFixture({
        ollamaModels: [
          { name: "my-custom-vectors", size: 100_000_000 },
          { name: "llama3.2:3b", size: 2_019_392_256 },
        ],
        settings: {
          transcriptionLanguage: "zh",
          whisperModelPath: "",
          analysisModel: "llama3.2:3b",
          embeddingModel: "my-custom-vectors",
        },
      }),
    );
    await renderPanel();

    const index = card("知识索引");
    expect(within(index).getByText("my-custom-vectors")).toBeTruthy();
    expect(within(index).queryByText("llama3.2:3b")).toBeNull();
  });
});

describe("AI 模型：下载状态长在模型行上", () => {
  it("收到进度事件时显示百分比和字节数，并可取消", async () => {
    await renderPanel();
    fireEvent.click(within(card("转写")).getByText("下载", { selector: "button" }));
    await waitFor(() => expect(tauri.downloadWhisperModel).toHaveBeenCalledWith("large-v3-turbo-q5_0"));

    emit("whisper-model-download-progress", downloadProgressFixture({ completed: 104_857_600, total: 574_041_195 }));

    expect(await screen.findByText("18%")).toBeTruthy();
    expect(screen.getByText("100MB / 547MB")).toBeTruthy();

    fireEvent.click(screen.getByText("取消", { selector: "button" }));
    await waitFor(() => expect(tauri.cancelModelDownload).toHaveBeenCalledWith("large-v3-turbo-q5_0"));
  });

  it("下载完成后重新拉状态，让新模型立刻出现在列表里", async () => {
    await renderPanel();
    fireEvent.click(within(card("转写")).getByText("下载", { selector: "button" }));
    const before = vi.mocked(tauri.getLocalAiStatus).mock.calls.length;

    emit("whisper-model-download-progress", downloadProgressFixture({ status: "completed" }));

    await waitFor(() => expect(vi.mocked(tauri.getLocalAiStatus).mock.calls.length).toBeGreaterThan(before));
  });

  it("Ollama 的英文状态不会原样丢给用户", async () => {
    await renderPanel();
    fireEvent.click(within(card("分析")).getAllByText("下载", { selector: "button" })[0]);
    await waitFor(() => expect(tauri.pullOllamaModel).toHaveBeenCalledWith("qwen2.5:7b"));

    emit("model-download-progress", {
      model: "qwen2.5:7b",
      status: "pulling manifest",
      completed: null,
      total: null,
      error: null,
    });

    expect(await screen.findByText("正在准备")).toBeTruthy();
    expect(screen.queryByText("pulling manifest")).toBeNull();
  });
});

describe("AI 模型：本地与外部是同一个决策", () => {
  it("默认在本机渠道，看不到 API Key 字段", async () => {
    await renderPanel();
    const channel = card("录音处理通道");
    expect(within(channel).getByText("本机处理", { selector: "button" })).toBeTruthy();
    expect(within(card("分析")).queryByText("接口地址")).toBeNull();
    expect(within(card("转写")).queryByText("语音转文字 API Key")).toBeNull();
  });

  it("切到外部渠道才出现凭据字段和隐私说明", async () => {
    await renderPanel();
    fireEvent.click(within(card("录音处理通道")).getByText("第三方处理", { selector: "button" }));

    expect(await within(card("转写")).findByText("接口地址")).toBeTruthy();
    expect(within(card("转写")).getByText("语音转文字 API Key")).toBeTruthy();
    expect(within(card("转写")).getByText(/原始音频会上传/)).toBeTruthy();
    expect(within(card("分析")).getByText("API Key")).toBeTruthy();
    expect(within(card("分析")).getAllByText(/逐字稿全文/).length).toBeGreaterThan(0);
    expect(await waitFor(() =>
      expect(tauri.updateExternalAiSettings).toHaveBeenCalledWith(
        expect.objectContaining({ processingMode: "external" }),
      ),
    ));
  });

  it("第三方服务未选择时仍可进入配置，状态保持待配置", async () => {
    vi.mocked(tauri.getExternalAiSettings).mockResolvedValue(
      externalAiSettingsFixture({ transcriptionProvider: "none" }),
    );
    await renderPanel();
    fireEvent.click(within(card("录音处理通道")).getByText("第三方处理", { selector: "button" }));

    const transcription = card("转写");
    const providerLabel = await within(transcription).findByText("服务类型");
    const provider = providerLabel.closest("label")?.querySelector("select");
    expect(provider).toBeTruthy();
    expect((provider as HTMLSelectElement).value).toBe("");
    expect(within(transcription).getByText("请选择语音转文字服务")).toBeTruthy();
    expect(within(transcription).getByText("语音转文字 API Key")).toBeTruthy();
    expect(within(transcription).getByText("待配置")).toBeTruthy();
    expect(screen.queryByText(/第三方处理需要选择语音转文字服务/)).toBeNull();
  });

  it("第三方转写缺少地址或模型时不谎报可用", async () => {
    vi.mocked(tauri.getExternalAiSettings).mockResolvedValue(
      externalAiSettingsFixture({
        processingMode: "external",
        transcriptionBaseUrl: "",
        transcriptionModel: "",
        transcriptionHasApiKey: true,
        audioUploadConsentAt: "2026-09-26T00:00:00Z",
      }),
    );
    await renderPanel();

    const transcription = card("转写");
    expect(within(transcription).getByText("待配置")).toBeTruthy();
    expect(within(transcription).queryByText("可用")).toBeNull();
    expect(screen.getByText(/第三方通道待完成：第三方转写/)).toBeTruthy();
  });

  it("第三方摘要显示实际转写和理解链路，不再误报本机 Ollama", async () => {
    vi.mocked(tauri.getExternalAiSettings).mockResolvedValue(
      externalAiSettingsFixture({
        processingMode: "external",
        enabled: true,
        hasApiKey: true,
        privacyConsentAt: "2026-09-26T00:00:00Z",
        transcriptionHasApiKey: true,
        audioUploadConsentAt: "2026-09-26T00:00:00Z",
      }),
    );
    await renderPanel();

    expect(
      screen.getByText(
        /转写 openai-compatible · qwen3-asr-flash ｜ 分析 外部模型 · gpt-4.1-mini/,
      ),
    ).toBeTruthy();
    expect(screen.queryByText(/本机 Ollama/)).toBeNull();
  });

  it("地址在失焦时才写库，敲到一半不会保存", async () => {
    await renderPanel();
    fireEvent.click(within(card("录音处理通道")).getByText("第三方处理", { selector: "button" }));
    const input = await screen.findByDisplayValue("https://api.openai.com/v1");
    const callsAfterChannelSwitch = vi.mocked(tauri.updateExternalAiSettings).mock.calls.length;

    fireEvent.change(input, { target: { value: "https://api.op" } });
    await new Promise((resolve) => setTimeout(resolve, 700));
    expect(vi.mocked(tauri.updateExternalAiSettings).mock.calls.length).toBe(callsAfterChannelSwitch);

    fireEvent.change(input, { target: { value: "https://api.example.com/v1" } });
    fireEvent.blur(input);

    await waitFor(() =>
      expect(tauri.updateExternalAiSettings).toHaveBeenCalledWith(
        expect.objectContaining({ baseUrl: "https://api.example.com/v1" }),
      ),
    );
  });
});

describe("AI 模型：转写引擎", () => {
  it("没装 whisperX 时给出安装方法，而不是把用户切到一个跑不了的引擎", async () => {
    await renderPanel();
    fireEvent.click(within(card("转写")).getByText(/whisperX/));

    expect(await screen.findByText(/pip install whisperx/)).toBeTruthy();
    expect(screen.getByText("1. 安装并检测 WhisperX")).toBeTruthy();
    expect(screen.getByText("2. 授权三个模型")).toBeTruthy();
    expect(screen.getByText("3. 保存 HuggingFace Token")).toBeTruthy();
    expect(screen.getAllByText("未完成").length).toBeGreaterThanOrEqual(2);
    expect(
      within(card("转写")).getAllByText("重新检测", { selector: "button" }).length,
    ).toBeGreaterThan(0);
    expect(tauri.getTranscriptionEngineStatus).toHaveBeenCalled();
    expect(tauri.setTranscriptionEngine).not.toHaveBeenCalled();
  });

  it("重新检测会刷新转写引擎状态", async () => {
    await renderPanel();
    fireEvent.click(within(card("转写")).getByText(/whisperX/));
    await screen.findByText("1. 安装并检测 WhisperX");
    const before = vi.mocked(tauri.getTranscriptionEngineStatus).mock.calls.length;

    fireEvent.click(
      within(card("转写")).getByRole("button", { name: "重新检测" }),
    );

    await waitFor(() =>
      expect(vi.mocked(tauri.getTranscriptionEngineStatus).mock.calls.length).toBeGreaterThan(before),
    );
  });

  it("HuggingFace Token 用密码框保存，保存后清空并显示完成状态", async () => {
    vi.mocked(tauri.getTranscriptionEngineStatus)
      .mockResolvedValueOnce(engineStatusFixture())
      .mockResolvedValue(engineStatusFixture({ hfTokenSet: true }));
    await renderPanel();
    fireEvent.click(within(card("转写")).getByText(/whisperX/));
    const tokenLabel = await screen.findByText("HuggingFace Token");
    const tokenInput = tokenLabel.closest("label")?.querySelector("input");
    expect(tokenInput).toBeTruthy();
    expect((tokenInput as HTMLInputElement).type).toBe("password");

    fireEvent.change(tokenInput as HTMLInputElement, { target: { value: "hf_secret" } });
    fireEvent.click(screen.getByRole("button", { name: "保存" }));

    await waitFor(() => expect(tauri.setHfToken).toHaveBeenCalledWith("hf_secret"));
    await waitFor(() =>
      expect((tokenInput as HTMLInputElement).value).toBe(""),
    );
    expect(screen.queryByText("hf_secret")).toBeNull();
    const tokenStep = screen
      .getByText("3. 保存 HuggingFace Token")
      .closest("li") as HTMLElement;
    expect(await within(tokenStep).findByText("已完成")).toBeTruthy();
  });

  it("已装 whisperX 时切换立刻生效，且不显示「没找到」的警告", async () => {
    vi.mocked(tauri.getTranscriptionEngineStatus).mockResolvedValue(
      engineStatusFixture({ engine: "whisperx", whisperxAvailable: true, whisperxPath: "/usr/local/bin/whisperx" }),
    );
    await renderPanel();
    // 已经在新引擎上，说明状态被尊重，同时安装步骤明确显示已检测。
    expect(screen.queryByText(/未检测到 whisperx/)).toBeNull();
    expect(screen.getByText(/已检测到 whisperx/)).toBeTruthy();

    fireEvent.click(within(card("转写")).getByText("内嵌引擎"));
    await waitFor(() => expect(tauri.setTranscriptionEngine).toHaveBeenCalledWith("embedded"));
  });

  it("已装 whisperX 时切过去立刻生效", async () => {
    vi.mocked(tauri.getTranscriptionEngineStatus).mockResolvedValue(
      engineStatusFixture({ whisperxAvailable: true, whisperxPath: "/usr/local/bin/whisperx" }),
    );
    await renderPanel();
    fireEvent.click(within(card("转写")).getByText("whisperX"));

    await waitFor(() => expect(tauri.setTranscriptionEngine).toHaveBeenCalledWith("whisperx"));
  });
});
