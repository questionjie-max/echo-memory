/**
 * 会议纪要面板（v1.1 块3）。
 *
 * 议题、结论、待办、术语表。每条结论/待办都带可点击回原文的时间戳——
 * 生成侧对每条引文做了逐字稿校验（fail-closed：对不上出处的条目直接丢弃），
 * 留下的每一条都能点回经校验的出处。
 *
 * 生成失败不阻塞录音：面板如实显示 failed 与原因，提供重试；
 * generating 时自动轮询，完成后停止。
 */
import { useCallback, useEffect, useRef, useState } from "react";
import type { MinutesItem, MinutesItemStatus, MinutesView } from "../shared/types";
import { generateMinutes, getRecordMinutes, setMinutesItemStatus } from "../lib/tauri";

interface Props {
  recordId: string;
  /** 点击时间戳回跳播放（与逐字稿/分析面板同一播放器）。 */
  onSeek: (ms: number) => void;
}

function formatTimestamp(ms: number): string {
  const totalSeconds = Math.floor(Math.max(0, ms) / 1_000);
  return `${String(Math.floor(totalSeconds / 60)).padStart(2, "0")}:${String(totalSeconds % 60).padStart(2, "0")}`;
}

function displayError(reason: unknown): string {
  return reason instanceof Error ? reason.message : String(reason);
}

export default function MinutesPanel({ recordId, onSeek }: Props) {
  const [view, setView] = useState<MinutesView | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const loadedRecord = useRef<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      const next = await getRecordMinutes(recordId);
      // 迟到的旧记录响应不得覆盖新记录的视图（切换记录的异步竞态）。
      if (loadedRecord.current === recordId) {
        setView(next);
      }
    } catch (reason) {
      if (loadedRecord.current === recordId) {
        setError(displayError(reason));
      }
    }
  }, [recordId]);

  useEffect(() => {
    if (loadedRecord.current !== recordId) {
      loadedRecord.current = recordId;
      setView(null);
      setError("");
    }
    void refresh();
  }, [recordId, refresh]);

  // generating 时轮询；完成/失败后停止。
  useEffect(() => {
    if (view?.status !== "generating") return;
    const timer = window.setInterval(() => void refresh(), 2_000);
    return () => window.clearInterval(timer);
  }, [view?.status, refresh]);

  async function generate() {
    setBusy(true);
    setError("");
    try {
      setView(await generateMinutes(recordId));
    } catch (reason) {
      setError(displayError(reason));
      await refresh();
    } finally {
      setBusy(false);
    }
  }

  function statusLine(view: MinutesView | null): string {
    switch (view?.status) {
      case "generating":
        return "正在生成本机纪要…";
      case "failed":
        return `纪要生成失败：${view.error ?? "未知原因"}（录音与逐字稿不受影响，可重试）`;
      case "completed":
        return view.provider === "ollama"
          ? `由本机模型 ${view.model ?? ""} 生成`
          : `由 ${view.model ?? "外部模型"} 生成`;
      default:
        return view ? "纪要待生成。" : "";
    }
  }

  async function toggleItem(item: MinutesItem, current: MinutesItemStatus) {
    const next: MinutesItemStatus = current === "confirmed" ? "provisional" : "confirmed";
    setError("");
    try {
      const confirmed = await setMinutesItemStatus(item.id, next);
      setView((current) =>
        current
          ? {
              ...current,
              itemStatuses: current.itemStatuses.map(([id, status]) =>
                id === item.id ? [id, confirmed] : [id, status],
              ),
            }
          : current,
      );
    } catch (reason) {
      setError(displayError(reason));
    }
  }

  function renderItem(item: MinutesItem, status: MinutesItemStatus | undefined) {
    const confirmed = status === "confirmed";
    return (
      <li className="minutes-item" key={item.id}>
        <button
          type="button"
          className="minutes-evidence-time"
          title={item.evidence.quoteText}
          onClick={() => onSeek(item.evidence.startMs)}
        >
          {formatTimestamp(item.evidence.startMs)}
        </button>
        <div className="minutes-item-body">
          <p>
            {item.text}
            <span className={`minutes-item-state${confirmed ? " confirmed" : ""}`}>
              {confirmed ? "已确认" : "暂定"}
            </span>
          </p>
          <blockquote>{item.evidence.quoteText}</blockquote>
          {item.owner && <span className="minutes-owner">负责人：{item.owner}</span>}
        </div>
        <div className="minutes-item-status">
          <button
            type="button"
            className={confirmed ? "secondary-button" : "link-button"}
            // 生成窗口期的确认会随全量重建被静默丢弃（回到暂定），
            // 直接禁用而不是让用户做一个注定丢失的操作。
            disabled={busy || view?.status === "generating"}
            title={
              view?.status === "generating"
                ? "纪要正在重新生成，生成完成后才能确认"
                : confirmed
                  ? "从记忆库撤回这条内容"
                  : "确认后这条内容才会进入记忆库，可被 MCP 读取；记录原文不受此门槛限制"
            }
            onClick={() => void toggleItem(item, status ?? "provisional")}
          >
            {confirmed ? "撤回" : "确认入记忆"}
          </button>
        </div>
      </li>
    );
  }

  const document = view?.document ?? null;
  const statusOf = (id: string): MinutesItemStatus | undefined => {
    const match = view?.itemStatuses.find(([itemId]) => itemId === id);
    return match?.[1];
  };
  const hasContent =
    document !== null &&
    (document.topics.length > 0 ||
      document.conclusions.length > 0 ||
      document.actionItems.length > 0 ||
      document.glossary.length > 0);

  return (
    <div className="minutes-view" aria-label="会议纪要">
      <div className="minutes-action">
        <div>
          <strong>{document ? "重新生成纪要" : "生成会议纪要"}</strong>
          <p>本机提取议题、结论、待办与术语表；每条结论与待办都经逐字稿校验，可点回出处。</p>
        </div>
        <button
          type="button"
          className="primary-button"
          disabled={busy || view?.status === "generating"}
          onClick={() => void generate()}
        >
          {view?.status === "generating" ? "生成中…" : document ? "重新生成" : "生成纪要"}
        </button>
      </div>

      {view?.status === "generating" && (
        <p className="minutes-status" role="status">{statusLine(view)}</p>
      )}
      {view?.status === "failed" && (
        <p className="minutes-status failed" role="alert">{statusLine(view)}</p>
      )}
      {view?.status === "completed" && (
        <p className="minutes-status">{statusLine(view)}</p>
      )}
      {document && document.droppedUnverifiable > 0 && (
        <p className="minutes-dropped">
          有 {document.droppedUnverifiable} 条条目因无法在逐字稿中找到可靠出处被丢弃（宁缺毋假）。
        </p>
      )}
      {error && <p className="minutes-status failed" role="alert">{error}</p>}

      {hasContent && document && (
        <div className="minutes-content">
          {document.topics.length > 0 && (
            <section>
              <h4>议题</h4>
              <ul className="minutes-topics">
                {document.topics.map((topic) => (
                  <li key={topic.id}>
                    {topic.startMs !== null && (
                      <button
                        type="button"
                        className="minutes-evidence-time"
                        onClick={() => onSeek(topic.startMs as number)}
                      >
                        {formatTimestamp(topic.startMs)}
                      </button>
                    )}
                    <div>
                      <strong>{topic.title}</strong>
                      {topic.summary && <p>{topic.summary}</p>}
                    </div>
                  </li>
                ))}
              </ul>
            </section>
          )}
          {document.conclusions.length > 0 && (
            <section>
              <h4>结论（{document.conclusions.length}）——未确认的不会进入记忆库</h4>
              <ul className="minutes-list">
                {document.conclusions.map((item) => renderItem(item, statusOf(item.id)))}
              </ul>
            </section>
          )}
          {document.actionItems.length > 0 && (
            <section>
              <h4>待办（{document.actionItems.length}）——未确认的不会进入记忆库</h4>
              <ul className="minutes-list">
                {document.actionItems.map((item) => renderItem(item, statusOf(item.id)))}
              </ul>
            </section>
          )}
          {document.glossary.length > 0 && (
            <section>
              <h4>术语表</h4>
              <ul className="minutes-glossary">
                {document.glossary.map((entry) => (
                  <li key={entry.term}>
                    <strong>{entry.term}</strong>
                    {entry.variants.length > 0 && (
                      <span>变体：{entry.variants.join("、")}</span>
                    )}
                  </li>
                ))}
              </ul>
            </section>
          )}
        </div>
      )}

      {view?.status === "completed" && !hasContent && (
        <p className="minutes-status">这段录音没有提取到结构化纪要内容。</p>
      )}
    </div>
  );
}
