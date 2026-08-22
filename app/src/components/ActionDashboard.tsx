import { useEffect, useState } from "react";
import type { ActionDashboard as ActionDashboardData } from "../shared/types";
import { getActionDashboard, setActionItemStatus } from "../lib/tauri";

interface Props {
  onOpenRecord: (recordId: string, segmentId: string | null) => void;
}

/**
 * 行动仪表盘：跨记录聚合行动项与未解决问题，可标记完成并跳回原始证据。
 */
export default function ActionDashboard({ onOpenRecord }: Props) {
  const [dashboard, setDashboard] = useState<ActionDashboardData | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busyId, setBusyId] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const data = await getActionDashboard();
        if (!cancelled) setDashboard(data);
      } catch (reason) {
        if (!cancelled) setError(String(reason));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  async function toggleStatus(id: string, status: string) {
    setBusyId(id);
    try {
      await setActionItemStatus(id, status === "open" ? "done" : "open");
      setDashboard(await getActionDashboard());
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusyId(null);
    }
  }

  if (error) {
    return <div className="action-dashboard-shell" role="alert"><p className="inline-error">{error}</p></div>;
  }
  if (!dashboard) {
    return <div className="action-dashboard-shell" role="status"><p>正在汇总行动项…</p></div>;
  }
  if (dashboard.items.length === 0 && dashboard.openQuestions.length === 0) {
    return (
      <div className="action-dashboard-shell">
        <div className="memory-empty">
          <strong>还没有可跟进的事项</strong>
          <p>分析录音后，识别出的行动项和未解决问题会自动汇总到这里。</p>
        </div>
      </div>
    );
  }

  const openItems = dashboard.items.filter((item) => item.status === "open");
  const doneItems = dashboard.items.filter((item) => item.status !== "open");

  return (
    <div className="action-dashboard-shell">
      <header className="action-dashboard-header">
        <h2>行动</h2>
        <p>{dashboard.openCount} 项待办 · {dashboard.doneCount} 项已完成 · {dashboard.openQuestions.length} 个未解决问题</p>
      </header>

      <section className="action-dashboard-section">
        <h3>行动项</h3>
        {openItems.length === 0 && doneItems.length === 0 && <p className="overview-empty">暂无行动项。</p>}
        {openItems.length > 0 && (
          <ul className="action-list">
            {openItems.map((item) => (
              <li key={item.id} className="action-item">
                <button
                  type="button"
                  className="action-item-body"
                  onClick={() => onOpenRecord(item.recordId, item.sourceSegmentId ?? null)}
                >
                  <strong>{item.title}</strong>
                  <span>
                    {item.recordTitle}
                    {item.projectName ? ` · ${item.projectName}` : ""}
                    {item.dueText ? ` · ${item.dueText}` : ""}
                    {item.ownerText ? ` · ${item.ownerText}` : ""}
                  </span>
                </button>
                <button
                  type="button"
                  className="toolbar-button"
                  disabled={busyId === item.id}
                  onClick={() => void toggleStatus(item.id, item.status)}
                >
                  完成
                </button>
              </li>
            ))}
          </ul>
        )}
        {doneItems.length > 0 && (
          <details className="action-done-group">
            <summary>已完成（{doneItems.length}）</summary>
            <ul className="action-list">
              {doneItems.map((item) => (
                <li key={item.id} className="action-item done">
                  <button
                    type="button"
                    className="action-item-body"
                    onClick={() => onOpenRecord(item.recordId, item.sourceSegmentId ?? null)}
                  >
                    <strong>{item.title}</strong>
                    <span>{item.recordTitle}</span>
                  </button>
                  <button
                    type="button"
                    className="toolbar-button"
                    disabled={busyId === item.id}
                    onClick={() => void toggleStatus(item.id, item.status)}
                  >
                    重新打开
                  </button>
                </li>
              ))}
            </ul>
          </details>
        )}
      </section>

      {dashboard.openQuestions.length > 0 && (
        <section className="action-dashboard-section">
          <h3>未解决问题</h3>
          <ul className="action-list">
            {dashboard.openQuestions.map((question, index) => (
              <li key={`${question.recordId}-${index}`} className="action-item">
                <button
                  type="button"
                  className="action-item-body"
                  onClick={() =>
                    onOpenRecord(question.recordId, question.citationSegmentIds[0] ?? null)
                  }
                >
                  <strong>{question.text}</strong>
                  <span>{question.recordTitle}</span>
                </button>
              </li>
            ))}
          </ul>
        </section>
      )}
    </div>
  );
}
