/**
 * 模型列表：设置面板与引导向导共用。
 *
 * 「列表即选择器」——点整行就切换当前模型，不需要先选中再按保存；
 * 下载状态长在模型自己那一行上，有半成品时按钮变「继续下载」。
 */
import type { LocalAiStatus, LocalModelInfo, RecommendedModel } from "../shared/types";
import { Button, ListRow, List, Pill, Progress, type RowState } from "./SettingsKit";
import { PlusIcon } from "./icons";
import { formatBytes, type DownloadState } from "./useModelDownloads";

/** 推荐的分析模型。不给假尺寸：真实体积只有拉取时后端才拿得到。 */
export const ANALYSIS_MODEL_SUGGESTIONS = [
  { name: "qwen2.5:7b", hint: "默认选择，中文分析更稳" },
  { name: "qwen3.5:4b", hint: "更小更快，机器配置吃紧时用它" },
];

function baseName(path: string): string {
  return path.split("/").pop() ?? path;
}

/**
 * 推荐模型在磁盘上的真实文件名。
 * 已安装模型的 id 是文件名去掉后缀（`ggml-large-v3-turbo-q5_0`），而推荐模型的 id 是
 * `large-v3-turbo-q5_0` —— 两个 id 天生对不上，所以只能比文件名。
 */
function isRecommendedModel(path: string, recommended: RecommendedModel): boolean {
  return baseName(path) === recommended.fileName;
}

/** 推荐模型始终用后端给的可读名字，其余用真实文件名 —— 和用户在磁盘上看到的一致。 */
function modelTitle(model: { id: string; path: string }, recommended: RecommendedModel): string {
  return isRecommendedModel(model.path, recommended) ? recommended.label : model.id;
}

export function WhisperModelList({
  status,
  download,
  onSelect,
  onDownload,
  onCancel,
  onChooseFile,
  selectable = true,
}: {
  status: LocalAiStatus;
  download?: DownloadState;
  onSelect: (path: string) => void;
  onDownload: (modelId: string) => void;
  onCancel: (modelId: string) => void;
  onChooseFile?: () => void;
  selectable?: boolean;
}) {
  const recommended = status.recommendedWhisperModel;
  const current = status.whisperModelPath;
  const installed = status.whisperModels.some((model) => isRecommendedModel(model.path, recommended));

  return (
    <>
      <List>
        {status.whisperModels.map((model) => {
          const selected = current === model.path;
          return (
            <ListRow
              key={model.path}
              state="done"
              selected={selected}
              title={modelTitle(model, recommended)}
              badge={selected ? <Pill tone="accent">当前使用</Pill> : undefined}
              meta={
                selected && status.whisperModelSource
                  ? `${status.whisperModelSource} · ${model.path}`
                  : model.path
              }
              trail={formatBytes(model.size)}
              titleAttr={model.path}
              onSelect={selectable && !selected ? () => onSelect(model.path) : undefined}
            />
          );
        })}
        {!installed && (
          <RecommendedWhisperRow
            status={status}
            download={download}
            onDownload={onDownload}
            onCancel={onCancel}
          />
        )}
      </List>
      {onChooseFile && (
        <Button variant="dashed" onClick={onChooseFile}>
          <PlusIcon size={14} /> 使用本机已有的 GGML 模型文件…
        </Button>
      )}
    </>
  );
}

function RecommendedWhisperRow({
  status,
  download,
  onDownload,
  onCancel,
}: {
  status: LocalAiStatus;
  download?: DownloadState;
  onDownload: (modelId: string) => void;
  onCancel: (modelId: string) => void;
}) {
  const recommended = status.recommendedWhisperModel;
  const partial = status.pendingWhisperDownload;
  const resuming = partial !== null && partial.bytes > 0;
  const running = download?.phase === "running";
  const failed = download?.phase === "failed";

  const state: RowState = running ? "working" : failed ? "error" : "idle";
  const meta = running
    ? download?.total
      ? `${formatBytes(download.completed ?? 0)} / ${formatBytes(download.total)}`
      : "正在准备下载"
    : failed
      ? (download?.error ?? "下载失败，已保留已下载的部分")
      : resuming
        ? `已下载 ${formatBytes(partial.bytes)}，可以接着下`
        : `${formatBytes(recommended.bytes)} · 应用内下载，支持断点续传`;

  return (
    <div>
      <ListRow
        state={state}
        title={recommended.label}
        meta={meta}
        trail={
          running ? (
            <Button variant="quiet" ariaLabel={`取消下载 ${recommended.label}`} onClick={() => onCancel(recommended.id)}>
              取消
            </Button>
          ) : (
            <Button
              variant="primary"
              ariaLabel={`${failed ? "重试下载" : resuming ? "继续下载" : "下载"} ${recommended.label}`}
              onClick={() => onDownload(recommended.id)}
            >
              {failed ? "重试" : resuming ? "继续下载" : "下载"}
            </Button>
          )
        }
      />
      {running && (
        <Progress
          completed={download?.completed ?? null}
          total={download?.total ?? null}
          label={download?.label ?? "正在下载"}
        />
      )}
    </div>
  );
}

/** Ollama 的嵌入模型在名字里都带 embedding/embed，用它做分析只会得到垃圾结果。 */
function isEmbeddingModel(name: string): boolean {
  return /embed/i.test(name);
}

export function OllamaModelList({
  models,
  selected,
  suggestions,
  download,
  onSelect,
  onDownload,
  onCancel,
  available,
  role,
}: {
  models: LocalModelInfo[];
  selected: string;
  suggestions: { name: string; hint: string }[];
  download?: DownloadState;
  onSelect: (name: string) => void;
  onDownload: (name: string) => void;
  onCancel: (name: string) => void;
  available: boolean;
  /** analysis 会把嵌入模型滤掉：它们是拿来算向量的，选来写分析只会出错。 */
  role: "analysis" | "embedding";
}) {
  /** analysis 会把嵌入模型滤掉；embedding 反过来只留嵌入模型（外加当前选中的那个，
      免得用户自定义的不带 embed 字样的模型从列表里消失）。 */
  const visible =
    role === "analysis"
      ? models.filter((model) => !isEmbeddingModel(model.name))
      : models.filter((model) => isEmbeddingModel(model.name) || model.name === selected);
  const installedNames = new Set(visible.map((model) => model.name));
  const missingSuggestions = suggestions.filter(
    (item) => !installedNames.has(item.name) && (role === "embedding" || !isEmbeddingModel(item.name)),
  );
  const runningFor = download?.phase === "running" ? download.model : null;

  return (
    <List>
      {visible.map((model) => {
        const isSelected = model.name === selected;
        const isRunning = runningFor === model.name;
        return (
          <div key={model.name}>
            <ListRow
              state={isRunning ? "working" : "done"}
              selected={isSelected}
              title={model.name}
              badge={isSelected ? <Pill tone="accent">当前使用</Pill> : undefined}
              meta={isRunning ? (download?.label ?? "正在下载") : undefined}
              trail={
                isRunning ? (
                  <Button variant="quiet" ariaLabel={`取消下载 ${model.name}`} onClick={() => onCancel(model.name)}>
                    取消
                  </Button>
                ) : (
                  formatBytes(model.size)
                )
              }
              onSelect={!isSelected && !isRunning ? () => onSelect(model.name) : undefined}
            />
            {isRunning && (
              <Progress
                completed={download?.completed ?? null}
                total={download?.total ?? null}
                label={download?.label ?? "正在下载"}
              />
            )}
          </div>
        );
      })}
      {missingSuggestions.map((item) => {
        const isRunning = runningFor === item.name;
        return (
          <div key={item.name}>
            <ListRow
              state={isRunning ? "working" : "idle"}
              title={item.name}
              meta={isRunning ? undefined : item.hint}
              trail={
                isRunning ? (
                  <Button variant="quiet" ariaLabel={`取消下载 ${item.name}`} onClick={() => onCancel(item.name)}>
                    取消
                  </Button>
                ) : (
                  <Button
                    variant="primary"
                    ariaLabel={`下载 ${item.name}`}
                    disabled={!available}
                    onClick={() => onDownload(item.name)}
                  >
                    下载
                  </Button>
                )
              }
            />
            {isRunning && (
              <Progress
                completed={download?.completed ?? null}
                total={download?.total ?? null}
                label={download?.label ?? "正在下载"}
              />
            )}
          </div>
        );
      })}
      {models.length === 0 && missingSuggestions.length === 0 && (
        <p className="em-empty">还没有安装任何模型。</p>
      )}
    </List>
  );
}
