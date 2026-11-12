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

describe("AI 模型：转写与分析是两张独立配置卡", () => {
  it("默认两张卡都在本机，不出现第三方凭据字段", async () => {
    await renderPanel();
    const transcription = card("转写");
    const analysis = card("分析");

    expect(
      within(transcription).getByRole("button", { name: "本机" }).getAttribute("aria-pressed"),
    ).toBe("true");
    expect(
      within(analysis).getByRole("button", { name: "本机" }).getAttribute("aria-pressed"),
    ).toBe("true");
    expect(within(transcription).queryByLabelText("转写服务商")).toBeNull();
    expect(within(analysis).queryByLabelText("分析服务商")).toBeNull();
    expect(within(transcription).queryByLabelText(/API Key/)).toBeNull();
    expect(within(analysis).queryByLabelText(/API Key/)).toBeNull();
  });

  it("只切转写时分析仍留在本机，内置服务商不显示接口地址", async () => {
    await renderPanel();
    const transcription = card("转写");
    fireEvent.click(within(transcription).getByRole("button", { name: "第三方" }));

    expect(await within(transcription).findByLabelText("转写服务商")).toBeTruthy();
    expect(within(transcription).getByLabelText("百炼 API Key")).toBeTruthy();
    expect(within(transcription).getByText(/会把原始音频上传给/)).toBeTruthy();
    expect(within(transcription).queryByDisplayValue("https://dashscope.aliyuncs.com/compatible-mode/v1")).toBeNull();
    expect(within(card("分析")).queryByLabelText("分析服务商")).toBeNull();
    await waitFor(() =>
      expect(tauri.updateExternalAiSettings).toHaveBeenCalledWith(
        expect.objectContaining({ transcriptionMode: "external" }),
      ),
    );
  });

  it("内置服务商自动选中目录推荐模型", async () => {
    vi.mocked(tauri.getExternalAiSettings).mockResolvedValue(
      externalAiSettingsFixture({
        transcriptionMode: "external",
        transcriptionProvider: "zhipu",
        transcriptionBaseUrl: "https://open.bigmodel.cn/api/paas/v4",
        transcriptionModel: "",
      }),
    );
    await renderPanel();

    const transcription = card("转写");
    const model = (await within(transcription).findByLabelText("转写模型")) as HTMLSelectElement;
    expect(model.value).toBe("glm-asr-2512");

    fireEvent.change(within(transcription).getByLabelText("转写服务商"), {
      target: { value: "dashscope" },
    });
    expect((within(transcription).getByLabelText("转写模型") as HTMLSelectElement).value).toBe(
      "qwen3-asr-flash",
    );
  });

  it("腾讯云动态显示三项凭据，豆包与 OpenAI 各显示单 Key", async () => {
    vi.mocked(tauri.getExternalAiSettings).mockResolvedValue(
      externalAiSettingsFixture({
        transcriptionMode: "external",
        transcriptionProvider: "tencent",
        transcriptionBaseUrl: "https://asr.cloud.tencent.com/asr/flash/v1",
        transcriptionModel: "16k_zh_en",
      }),
    );
    await renderPanel();
    const transcription = card("转写");
    expect(await within(transcription).findByLabelText("AppID")).toBeTruthy();
    expect(within(transcription).getByLabelText("SecretId")).toBeTruthy();
    expect(within(transcription).getByLabelText("SecretKey")).toBeTruthy();

    fireEvent.change(within(transcription).getByLabelText("转写服务商"), {
      target: { value: "doubao" },
    });
    expect(within(transcription).getByLabelText("豆包语音 API Key")).toBeTruthy();
    expect(within(transcription).queryByLabelText("AppID")).toBeNull();

    fireEvent.change(within(transcription).getByLabelText("转写服务商"), {
      target: { value: "openai" },
    });
    expect(within(transcription).getByLabelText("OpenAI API Key")).toBeTruthy();
    expect((within(transcription).getByLabelText(/模型 ID/) as HTMLInputElement).value).toBe(
      "whisper-1",
    );
  });

  it("已保存腾讯云凭据后，只改一个字段不能保存半套凭据", async () => {
    vi.mocked(tauri.getExternalAiSettings).mockResolvedValue(
      externalAiSettingsFixture({
        transcriptionMode: "external",
        transcriptionProvider: "tencent",
        transcriptionBaseUrl: "https://asr.cloud.tencent.com/asr/flash/v1",
        transcriptionModel: "16k_zh_en",
        transcriptionHasApiKey: true,
        audioUploadConsentAt: "2026-09-23T00:00:00Z",
      }),
    );
    await renderPanel();
    const transcription = card("转写");
    const save = await within(transcription).findByRole("button", {
      name: "保存并验证",
    });
    expect(save.hasAttribute("disabled")).toBe(false);

    fireEvent.click(within(transcription).getByRole("button", { name: "修改" }));
    fireEvent.change(within(transcription).getByLabelText(/SecretKey/), {
      target: { value: "new-secret-key" },
    });
    expect(save.hasAttribute("disabled")).toBe(true);

    fireEvent.change(within(transcription).getByLabelText(/SecretId/), {
      target: { value: "new-secret-id" },
    });
    expect(save.hasAttribute("disabled")).toBe(true);

    fireEvent.change(within(transcription).getByLabelText(/AppID/), {
      target: { value: "123456789" },
    });
    expect(save.hasAttribute("disabled")).toBe(false);
  });

  it("切换服务商或高级改模型都会清空隐私同意", async () => {
    vi.mocked(tauri.getExternalAiSettings).mockResolvedValue(
      externalAiSettingsFixture({
        transcriptionMode: "external",
        transcriptionProvider: "zhipu",
        transcriptionBaseUrl: "https://open.bigmodel.cn/api/paas/v4",
        transcriptionModel: "glm-asr-2512",
      }),
    );
    await renderPanel();
    const transcription = card("转写");
    const consent = (await within(transcription).findByRole("checkbox")) as HTMLInputElement;

    fireEvent.click(consent);
    expect(consent.checked).toBe(true);
    fireEvent.click(within(transcription).getByRole("button", { name: "高级" }));
    fireEvent.change(within(transcription).getByLabelText(/手动模型 ID/), {
      target: { value: "glm-asr-private" },
    });
    expect(consent.checked).toBe(false);

    fireEvent.click(consent);
    expect(consent.checked).toBe(true);
    fireEvent.change(within(transcription).getByLabelText("转写服务商"), {
      target: { value: "dashscope" },
    });
    expect(consent.checked).toBe(false);
  });

  it("自定义兼容接口才显示地址和手动模型输入", async () => {
    // 测试用假凭据：仅供断言的占位值，不是真密钥（常量引用，避免被安全扫描误判）。
    const customFixture = "mock-custom-value";
    vi.mocked(tauri.getExternalAiSettings).mockResolvedValue(
      externalAiSettingsFixture({
        transcriptionMode: "external",
        transcriptionProvider: "zhipu",
        transcriptionBaseUrl: "https://open.bigmodel.cn/api/paas/v4",
        transcriptionModel: "glm-asr-2512",
      }),
    );
    await renderPanel();
    const transcription = card("转写");
    fireEvent.change(await within(transcription).findByLabelText("转写服务商"), {
      target: { value: "custom" },
    });

    const url = within(transcription).getByLabelText(/转写接口地址/) as HTMLInputElement;
    const model = within(transcription).getByLabelText(/模型 ID/) as HTMLInputElement;
    expect(url.value).toBe("");
    expect(model.value).toBe("");

    fireEvent.change(url, { target: { value: "https://relay.example/v1" } });
    fireEvent.change(model, { target: { value: "whisper-large-v3" } });
    fireEvent.change(within(transcription).getByLabelText("API Key"), {
      target: { value: customFixture },
    });
    fireEvent.click(within(transcription).getByRole("checkbox"));
    // 仅保存路径（跳过验证）：凭据照常落盘。
    fireEvent.click(within(transcription).getByRole("button", { name: "仅保存" }));

    await waitFor(() =>
      expect(tauri.saveExternalCredentials).toHaveBeenCalledWith(
        "transcription",
        "custom",
        { apiKey: customFixture },
      ),
    );
    expect(tauri.updateExternalAiSettings).toHaveBeenCalledWith(
      expect.objectContaining({
        transcriptionProvider: "custom",
        transcriptionBaseUrl: "https://relay.example/v1",
        transcriptionModel: "whisper-large-v3",
      }),
    );
  });

  it("已保存的 Key 默认只显示已粘贴状态，显式点击才回显，修改时要求重新填写", async () => {
    vi.mocked(tauri.getExternalAiSettings).mockResolvedValue(
      externalAiSettingsFixture({
        transcriptionMode: "external",
        transcriptionProvider: "zhipu",
        transcriptionBaseUrl: "https://open.bigmodel.cn/api/paas/v4",
        transcriptionModel: "glm-asr-2512",
        transcriptionHasApiKey: true,
      }),
    );
    await renderPanel();
    const transcription = card("转写");

    expect(await within(transcription).findByText("已保存 1 项凭据")).toBeTruthy();
    expect(tauri.revealExternalCredentials).not.toHaveBeenCalled();

    fireEvent.click(within(transcription).getByRole("button", { name: "显示" }));
    await waitFor(() =>
      expect(tauri.revealExternalCredentials).toHaveBeenCalledWith(
        "transcription",
        "zhipu",
      ),
    );
    const input = (await within(transcription).findByLabelText(
      "智谱 API Key",
    )) as HTMLInputElement;
    expect(input.value).toBe("mock-saved-value");
    expect(input.readOnly).toBe(true);

    fireEvent.click(within(transcription).getByRole("button", { name: "隐藏" }));
    expect(within(transcription).queryByLabelText("智谱 API Key")).toBeNull();

    fireEvent.click(within(transcription).getByRole("button", { name: "修改" }));
    const replace = within(transcription).getByLabelText(/智谱 API Key/) as HTMLInputElement;
    expect(replace.value).toBe("");
    expect(replace.readOnly).toBe(false);
  });

  it("已保存的第三方转写可发送一段测试音频做真实连通验证", async () => {
    vi.mocked(tauri.getExternalAiSettings).mockResolvedValue(
      externalAiSettingsFixture({
        transcriptionMode: "external",
        transcriptionProvider: "zhipu",
        transcriptionBaseUrl: "https://open.bigmodel.cn/api/paas/v4",
        transcriptionModel: "glm-asr-2512",
        transcriptionHasApiKey: true,
        audioUploadConsentAt: "2026-09-23T00:00:00Z",
      }),
    );
    await renderPanel();
    const transcription = card("转写");

    expect(
      await within(transcription).findByText(/约 1 秒的静音测试音/),
    ).toBeTruthy();
    fireEvent.click(within(transcription).getByRole("button", { name: "验证连通" }));

    await waitFor(() => expect(tauri.verifySavedProvider).toHaveBeenCalledWith("transcription"));
    expect(tauri.verifySavedProvider).not.toHaveBeenCalledWith("analysis");
  });

  it("OpenAI 可按当前 Key 拉取转写模型并用新结果更新模型输入", async () => {
    vi.mocked(tauri.getExternalAiSettings).mockResolvedValue(
      externalAiSettingsFixture({
        transcriptionMode: "external",
        transcriptionProvider: "openai",
        transcriptionBaseUrl: "https://api.openai.com/v1",
        transcriptionModel: "whisper-1",
        transcriptionHasApiKey: true,
        audioUploadConsentAt: "2026-09-23T00:00:00Z",
      }),
    );
    await renderPanel();
    const transcription = card("转写");

    fireEvent.click(
      await within(transcription).findByRole("button", { name: "拉取可用模型" }),
    );
    await waitFor(() =>
      expect(tauri.listExternalModels).toHaveBeenCalledWith(
        "transcription",
        "openai",
        "https://api.openai.com/v1",
        null,
      ),
    );

    // 拉取结果进入模型输入框的 datalist 候选（单一主输入控件，不再出现第二个下拉）。
    await waitFor(() => {
      const option = transcription.querySelector(
        'datalist option[value="gpt-4o-transcribe"]',
      );
      expect(option).not.toBeNull();
    });
    fireEvent.change(within(transcription).getByLabelText(/模型 ID/), {
      target: { value: "gpt-4o-transcribe" },
    });
    expect(
      (within(transcription).getByLabelText(/模型 ID/) as HTMLInputElement).value,
    ).toBe("gpt-4o-transcribe");
    expect(
      (within(transcription).getByRole("checkbox") as HTMLInputElement).checked,
    ).toBe(false);
  });

  it("阿里云转写填写 Key 后即可拉取并选择厂商模型", async () => {
    vi.mocked(tauri.listExternalModels).mockResolvedValue({
      chatCandidates: ["qwen3-asr-flash", "qwen-audio-asr-latest"],
      otherModels: [],
      allUnclassified: false,
    });
    await renderPanel();
    const transcription = card("转写");
    fireEvent.click(within(transcription).getByRole("button", { name: "第三方" }));

    fireEvent.change(await within(transcription).findByLabelText(/百炼 API Key/), {
      target: { value: "sk-dashscope" },
    });
    const pull = within(transcription).getByRole("button", { name: "拉取可用模型" });
    expect(pull.hasAttribute("disabled")).toBe(false);
    fireEvent.click(pull);

    await waitFor(() =>
      expect(tauri.listExternalModels).toHaveBeenCalledWith(
        "transcription",
        "dashscope",
        "https://dashscope.aliyuncs.com/compatible-mode/v1",
        "sk-dashscope",
      ),
    );
    expect(
      await within(transcription).findByRole("option", {
        name: "qwen-audio-asr-latest",
      }),
    ).toBeTruthy();
  });

  it("转写填写 Key 后可直接保存并验证：先草稿诊断（不落盘），通过才凭票据提交", async () => {
    // 假凭据 fixture：只用于断言诊断走的是草稿值，不是真实 Key。
    const draftKey = ["draft", "key", "fixture"].join("-");
    await renderPanel();
    const transcription = card("转写");
    fireEvent.click(within(transcription).getByRole("button", { name: "第三方" }));

    fireEvent.change(await within(transcription).findByLabelText(/百炼 API Key/), {
      target: { value: draftKey },
    });
    fireEvent.click(within(transcription).getByRole("checkbox"));
    const saveAndVerify = within(transcription).getByRole("button", { name: "保存并验证" });
    expect(saveAndVerify.hasAttribute("disabled")).toBe(false);
    fireEvent.click(saveAndVerify);

    // 先诊断草稿：新 Key 只走内存，不写凭据槽位。
    await waitFor(() => expect(tauri.diagnoseExternalProvider).toHaveBeenCalledOnce());
    expect(tauri.diagnoseExternalProvider).toHaveBeenCalledWith(
      expect.objectContaining({
        target: "transcription",
        providerId: "dashscope",
        credentials: { apiKey: draftKey },
      }),
    );
    // 诊断通过（默认 mock）→ 凭票据提交；旧的先落盘再测路径不该被走到。
    await waitFor(() => expect(tauri.saveAndVerifyProvider).toHaveBeenCalledOnce());
    expect(tauri.saveAndVerifyProvider).toHaveBeenCalledWith(
      expect.objectContaining({
        target: "transcription",
        verificationToken: "test-verification-token",
      }),
    );
    expect(tauri.saveExternalCredentials).not.toHaveBeenCalled();
  });

  it("分析表单可用刚填写但尚未保存的 Key 拉取模型", async () => {
    await renderPanel();
    const analysis = card("分析");
    fireEvent.click(within(analysis).getByRole("button", { name: "第三方" }));

    fireEvent.change(
      await within(analysis).findByLabelText(/OpenAI API Key/),
      { target: { value: "sk-entered-before-save" } },
    );
    const pull = within(analysis).getByRole("button", { name: "拉取可用模型" });
    expect(pull.hasAttribute("disabled")).toBe(false);
    fireEvent.click(pull);

    await waitFor(() =>
      expect(tauri.listExternalModels).toHaveBeenCalledWith(
        "analysis",
        "openai",
        "https://api.openai.com/v1",
        "sk-entered-before-save",
      ),
    );
    expect(tauri.saveExternalCredentials).not.toHaveBeenCalled();
  });

  it("没有模型列表接口的服务商明确提示只能使用内置或手动模型", async () => {
    vi.mocked(tauri.getExternalAiSettings).mockResolvedValue(
      externalAiSettingsFixture({
        transcriptionMode: "external",
        transcriptionProvider: "doubao",
        transcriptionBaseUrl:
          "https://openspeech.bytedance.com/api/v3/auc/bigmodel/recognize/flash",
        transcriptionModel: "volc.bigasr.auc_turbo",
        transcriptionHasApiKey: true,
      }),
    );
    await renderPanel();
    const transcription = card("转写");

    expect(
      await within(transcription).findByText(
        "该服务商不提供模型列表接口，只能使用内置推荐模型或手动模型 ID。",
      ),
    ).toBeTruthy();
    expect(
      within(transcription).queryByRole("button", { name: "拉取可用模型" }),
    ).toBeNull();
  });

  it("另一张卡已有同服务商凭据时可在本卡复用，秘密不经过输入框", async () => {
    vi.mocked(tauri.getExternalAiSettings).mockResolvedValue(
      externalAiSettingsFixture({
        analysisProvider: "zhipu",
        baseUrl: "https://open.bigmodel.cn/api/paas/v4",
        model: "glm-4-flash",
        transcriptionProvider: "zhipu",
        transcriptionBaseUrl: "https://open.bigmodel.cn/api/paas/v4",
        transcriptionModel: "glm-asr-2512",
        transcriptionHasApiKey: true,
      }),
    );
    await renderPanel();
    const analysis = card("分析");
    fireEvent.click(within(analysis).getByRole("button", { name: "第三方" }));

    fireEvent.click(await within(analysis).findByRole("button", { name: "复用已配置凭据" }));
    await waitFor(() => expect(tauri.reuseExternalCredentials).toHaveBeenCalledWith("analysis"));
    expect(within(analysis).queryByDisplayValue("sk-")).toBeNull();
  });

  it("未勾同意不能保存；保存后只承诺首次转写验证", async () => {
    let current = externalAiSettingsFixture({
      transcriptionMode: "external",
      transcriptionProvider: "zhipu",
      transcriptionBaseUrl: "https://open.bigmodel.cn/api/paas/v4",
      transcriptionModel: "glm-asr-2512",
    });
    vi.mocked(tauri.getExternalAiSettings).mockImplementation(() => Promise.resolve(current));
    vi.mocked(tauri.updateExternalAiSettings).mockImplementation((next) => {
      current = next;
      return Promise.resolve(current);
    });
    vi.mocked(tauri.saveExternalCredentials).mockImplementation(() => {
      current = { ...current, transcriptionHasApiKey: true };
      return Promise.resolve(current);
    });
    await renderPanel();
    const transcription = card("转写");

    fireEvent.change(await within(transcription).findByLabelText("智谱 API Key"), {
      target: { value: "zhipu-secret" },
    });
    const save = within(transcription).getByRole("button", { name: "保存并验证" });
    expect(save.hasAttribute("disabled")).toBe(true);

    fireEvent.click(within(transcription).getByRole("checkbox"));
    expect(save.hasAttribute("disabled")).toBe(false);
    fireEvent.click(save);

    expect(await within(transcription).findByText("已保存 1 项凭据")).toBeTruthy();
    // 保存后没有健康快照：状态条必须落在「已配置，未验证」，不得自称已验证或已连接。
    expect(await within(transcription).findByText("已配置，未验证")).toBeTruthy();
    expect(within(transcription).queryByText(/已连接/)).toBeNull();
    expect(within(transcription).queryByText("已验证")).toBeNull();
  });
});

describe("AI 模型：转写引擎", () => {
  it("没装 whisperX 时给出安装方法，而不是把用户切到一个跑不了的引擎", async () => {
    await renderPanel();
    fireEvent.click(within(card("转写")).getByText(/whisperX/));

    // v1.4 包3：安装引导用人话（复制命令到终端 + 装不上也不影响默认转写），不再裸抛 pip
    expect(await screen.findByText(/复制到「终端」App 里运行/)).toBeTruthy();
    expect(screen.queryByText(/pip install whisperx/)).toBeNull();
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

  it("已装 whisperX 时切过去立刻生效，且标注实验特性", async () => {
    vi.mocked(tauri.getTranscriptionEngineStatus).mockResolvedValue(
      engineStatusFixture({ whisperxAvailable: true, whisperxPath: "/usr/local/bin/whisperx" }),
    );
    await renderPanel();
    // 块6：开关文案带「实验」标注（默认关 + 显性实验身份）。
    // 引导区的实验标题在「未安装」用例里断言（已安装时引导不展开）。
    expect(within(card("转写")).getByText("whisperX（实验）")).toBeTruthy();
    fireEvent.click(within(card("转写")).getByText("whisperX（实验）"));

    await waitFor(() => expect(tauri.setTranscriptionEngine).toHaveBeenCalledWith("whisperx"));
  });
});
