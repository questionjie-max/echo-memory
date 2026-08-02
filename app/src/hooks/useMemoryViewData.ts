import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type {
  ExternalAiSettings,
  MemoryScope,
  MemorySnapshot,
  MemoryViewKind,
  TimelineItem,
} from "../shared/types";
import {
  cancelMemoryGeneration,
  getExternalAiSettings,
  getLocalTimeline,
  getMemoryGenerationJob,
  listMemorySnapshots,
  startMemoryGeneration,
} from "../lib/tauri";

export type MemoryRangeKey = "7d" | "30d" | "90d" | "all";

export function memoryRangeBounds(range: MemoryRangeKey) {
  if (range === "all") return { rangeStart: null, rangeEnd: null };
  const days = range === "7d" ? 7 : range === "30d" ? 30 : 90;
  const end = new Date();
  end.setHours(23, 59, 59, 999);
  const start = new Date(end);
  start.setDate(start.getDate() - days + 1);
  start.setHours(0, 0, 0, 0);
  return { rangeStart: start.toISOString(), rangeEnd: end.toISOString() };
}

export function memoryRangeLabel(range: MemoryRangeKey) {
  return range === "all" ? "全部时间" : `最近 ${range.replace("d", "")} 天`;
}

export function memoryScopeLabel(scope: MemoryScope) {
  if (scope.kind === "project") return "当前项目";
  if (scope.kind === "unfiled") return "未归档";
  return "全部资料";
}

export function formatMemoryDate(
  value: string,
  options: Intl.DateTimeFormatOptions,
  fallback = "时间未知",
) {
  const date = new Date(value);
  return Number.isFinite(date.getTime())
    ? new Intl.DateTimeFormat("zh-CN", options).format(date)
    : fallback;
}

export function useMemoryViewData(
  viewKind: MemoryViewKind,
  scope: MemoryScope,
  range: MemoryRangeKey,
) {
  const bounds = useMemo(() => memoryRangeBounds(range), [range]);
  const [settings, setSettings] = useState<ExternalAiSettings | null>(null);
  const [localItems, setLocalItems] = useState<TimelineItem[]>([]);
  const [snapshots, setSnapshots] = useState<MemorySnapshot[]>([]);
  const [selectedSnapshotId, setSelectedSnapshotId] = useState<string | null>(
    null,
  );
  const [loading, setLoading] = useState(true);
  const [generating, setGenerating] = useState(false);
  const [generationId, setGenerationId] = useState<string | null>(null);
  const [generationContextKey, setGenerationContextKey] = useState<string | null>(null);
  const [error, setError] = useState("");
  const cancelRequested = useRef(false);
  const cancelConfirmationAttempts = useRef(0);
  const loadRequestId = useRef(0);
  const contextKey = `${viewKind}:${scope.kind}:${scope.projectId ?? ""}:${bounds.rangeStart ?? ""}:${bounds.rangeEnd ?? ""}`;
  const contextKeyRef = useRef(contextKey);
  contextKeyRef.current = contextKey;

  const load = useCallback(async () => {
    const requestId = ++loadRequestId.current;
    setLoading(true);
    setError("");
    try {
      const [external, local, versions] = await Promise.allSettled([
        getExternalAiSettings(),
        getLocalTimeline(scope, bounds.rangeStart, bounds.rangeEnd),
        listMemorySnapshots(viewKind, scope, bounds.rangeStart, bounds.rangeEnd),
      ]);
      if (requestId !== loadRequestId.current) return;

      const errors: string[] = [];
      const nextSnapshots = versions.status === "fulfilled" ? versions.value : [];
      if (external.status === "fulfilled") setSettings(external.value);
      else errors.push(`外部 AI 设置读取失败：${String(external.reason)}`);
      if (local.status === "fulfilled") setLocalItems(local.value);
      else errors.push(`本地时间轴读取失败：${String(local.reason)}`);
      if (versions.status === "fulfilled") setSnapshots(versions.value);
      else errors.push(`快照读取失败：${String(versions.reason)}`);
      setError(errors.join("；"));
      setSelectedSnapshotId((current) => {
        if (current && nextSnapshots.some((item) => item.id === current))
          return current;
        return (
          nextSnapshots.find(
            (item) => item.status === "completed" || item.status === "partial",
          )?.id ??
          nextSnapshots[0]?.id ??
          null
        );
      });
      return nextSnapshots;
    } catch (reason) {
      if (requestId === loadRequestId.current) setError(String(reason));
      return null;
    } finally {
      if (requestId === loadRequestId.current) setLoading(false);
    }
  }, [
    viewKind,
    scope.kind,
    scope.projectId,
    bounds.rangeStart,
    bounds.rangeEnd,
  ]);

  useEffect(() => {
    setLocalItems([]);
    setSnapshots([]);
    setSelectedSnapshotId(null);
    void load();
    return () => {
      loadRequestId.current += 1;
    };
  }, [load]);

  useEffect(() => {
    cancelRequested.current = false;
    cancelConfirmationAttempts.current = 0;
    setGenerating(false);
    setGenerationId(null);
    setGenerationContextKey(null);
  }, [contextKey]);

  useEffect(() => {
    if (!generationId || generationContextKey !== contextKey) return;
    const taskId = generationId;
    let active = true;
    let timer: number | undefined;
    const schedule = () => {
      timer = window.setTimeout(() => void poll(), 800);
    };
    const poll = async () => {
      try {
        const job = await getMemoryGenerationJob(taskId);
        if (!active) return;
        if (job.status === "generating") {
          schedule();
          return;
        }

        const refreshed = await load();
        if (!active) return;
        const snapshot = job.snapshotId
          ? refreshed?.find((item) => item.id === job.snapshotId)
          : null;
        if (job.status === "cancelled" && cancelRequested.current) {
          cancelConfirmationAttempts.current += 1;
          const snapshotStillGenerating = Boolean(
            snapshot?.status === "generating" ||
              refreshed?.some((item) => item.status === "generating"),
          );
          if (
            cancelConfirmationAttempts.current < 2 ||
            (snapshotStillGenerating && cancelConfirmationAttempts.current < 5)
          ) {
            schedule();
            return;
          }
        }
        if (job.snapshotId && snapshot) setSelectedSnapshotId(job.snapshotId);
        if (job.errorMessage) setError(job.errorMessage);
        setGenerating(false);
        setGenerationId(null);
        setGenerationContextKey(null);
        cancelRequested.current = false;
        cancelConfirmationAttempts.current = 0;
      } catch (reason) {
        if (!active) return;
        setGenerating(false);
        setGenerationId(null);
        setGenerationContextKey(null);
        cancelRequested.current = false;
        cancelConfirmationAttempts.current = 0;
        setError(`无法读取生成任务状态：${String(reason)}`);
      }
    };
    timer = window.setTimeout(() => void poll(), 300);
    return () => {
      active = false;
      if (timer !== undefined) window.clearTimeout(timer);
    };
  }, [contextKey, generationContextKey, generationId, load]);

  const selectedSnapshot =
    snapshots.find((item) => item.id === selectedSnapshotId) ?? null;
  const sourceRecordCount = new Set(
    localItems.flatMap((item) => item.sources.map((source) => source.recordId)),
  ).size;

  async function generate() {
    const taskContextKey = contextKey;
    const id = `memory-${Date.now()}-${Math.random().toString(16).slice(2)}`;
    setGenerating(true);
    setError("");
    cancelRequested.current = false;
    cancelConfirmationAttempts.current = 0;
    setGenerationContextKey(contextKey);
    try {
      const job = await startMemoryGeneration({
        generationId: id,
        viewKind,
        scope,
        rangeStart: bounds.rangeStart,
        rangeEnd: bounds.rangeEnd,
      });
      if (contextKeyRef.current !== taskContextKey) return job;
      setGenerationId(job.generationId);
      return job;
    } catch (reason) {
      if (contextKeyRef.current !== taskContextKey) return null;
      setGenerating(false);
      setGenerationId(null);
      setError(String(reason));
      return null;
    }
  }

  async function cancel() {
    if (!generationId) return;
    if (cancelRequested.current) return;
    cancelRequested.current = true;
    cancelConfirmationAttempts.current = 0;
    try {
      await cancelMemoryGeneration(generationId);
    } catch (reason) {
      cancelRequested.current = false;
      setError(String(reason));
    }
  }

  return {
    settings,
    localItems,
    snapshots,
    selectedSnapshot,
    selectedSnapshotId,
    setSelectedSnapshotId,
    loading,
    generating,
    error,
    sourceRecordCount,
    bounds,
    generate,
    cancel,
    reload: load,
  };
}
