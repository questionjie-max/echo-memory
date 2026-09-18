/**
 * 模型下载的进度订阅与动作。
 *
 * 设置面板和引导向导共用这一份：下载状态跟着模型走，跟着模型那一行显示，
 * 不再是一个和模型脱钩的、贴在页面底部的进度条。
 */
import { useCallback, useEffect, useRef, useState } from "react";
import { isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { cancelModelDownload, downloadWhisperModel, pullOllamaModel } from "../lib/tauri";
import type { ModelDownloadProgress } from "../shared/types";

export type DownloadPhase = "running" | "done" | "failed" | "cancelled";

export interface DownloadState {
  model: string;
  phase: DownloadPhase;
  label: string;
  completed: number | null;
  total: number | null;
  error: string | null;
}

export type DownloadKind = "whisper" | "ollama";

/** Ollama 回传的英文状态直接展示在中文界面上不合适，认识的翻成人话，不认识的按有无字节数兜底。 */
function describe(status: string, completed: number | null, total: number | null): string {
  const normalized = status.toLowerCase();
  if (normalized.includes("manifest") || normalized.includes("pulling")) return "正在准备";
  if (normalized.includes("verifying")) return "正在校验";
  if (normalized.includes("writing")) return "正在写入";
  if (normalized === "success") return "已完成";
  if (completed !== null && total) return "正在下载";
  return status;
}

function toState(progress: ModelDownloadProgress): DownloadState {
  const base = { model: progress.model, completed: progress.completed, total: progress.total, error: progress.error };
  if (progress.status === "completed") {
    return { ...base, phase: "done", label: "已完成", error: null };
  }
  if (progress.status === "failed") {
    return { ...base, phase: "failed", label: "下载失败" };
  }
  if (progress.status === "cancelled") {
    return { ...base, phase: "cancelled", label: "已取消", error: null };
  }
  return {
    ...base,
    phase: "running",
    label: progress.status.startsWith("校验完成") ? "校验完成" : describe(progress.status, progress.completed, progress.total),
  };
}

export function useModelDownloads(onSettled?: (state: DownloadState) => void) {
  const [downloads, setDownloads] = useState<Record<string, DownloadState>>({});
  // 事件监听只挂一次，用 ref 拿最新的回调，避免闭包捕获首次渲染的版本。
  const settled = useRef(onSettled);
  settled.current = onSettled;

  useEffect(() => {
    if (!isTauri()) return;
    const record = (state: DownloadState) => {
      setDownloads((current) => ({ ...current, [state.model]: state }));
      if (state.phase !== "running") settled.current?.(state);
    };
    const stop = [
      listen<ModelDownloadProgress>("whisper-model-download-progress", (event) => record(toState(event.payload))),
      listen<ModelDownloadProgress>("model-download-progress", (event) => record(toState(event.payload))),
    ];
    return () => {
      for (const pending of stop) void pending.then((off) => off());
    };
  }, []);

  const start = useCallback(async (kind: DownloadKind, model: string) => {
    setDownloads((current) => ({
      ...current,
      [model]: { model, phase: "running", label: "正在准备", completed: null, total: null, error: null },
    }));
    try {
      if (kind === "whisper") await downloadWhisperModel(model);
      else await pullOllamaModel(model);
    } catch (reason) {
      setDownloads((current) => ({
        ...current,
        [model]: { model, phase: "failed", label: "下载失败", completed: null, total: null, error: String(reason) },
      }));
    }
  }, []);

  const cancel = useCallback(async (model: string) => {
    try {
      await cancelModelDownload(model);
    } catch {
      // 取消失败不阻塞界面：后端仍会在下载结束时发终态事件。
    }
  }, []);

  const dismiss = useCallback((model: string) => {
    setDownloads((current) => {
      const next = { ...current };
      delete next[model];
      return next;
    });
  }, []);

  return { downloads, start, cancel, dismiss };
}

export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0KB";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value.toFixed(value >= 100 || unit === 0 ? 0 : 1)}${units[unit]}`;
}
