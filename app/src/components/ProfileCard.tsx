import { useEffect, useRef, useState } from "react";
import { SparklesIcon, TriangleAlertIcon } from "./icons";
import { generateMemorySnapshot, getMemoryProfile } from "../lib/tauri";
import { useAnalysisSetup } from "./AnalysisSetupSheet";
import type { ProfileSnapshot, ProfileSummary } from "../shared/types";
import "./profile-card.css";

interface ProfileCardProps {
  onOpenSettings: () => void;
  /** 演化条目有变化（确认/驳回/重新生成）后提醒刷新总述 */
  refreshToken: number;
}

const SECTIONS: Array<{ key: keyof ProfileSummary; title: string; empty: string }> = [
  { key: "stances", title: "我的立场", empty: "还没有足够多的确认条目。" },
  { key: "preferences", title: "我的偏好", empty: "还没有足够多的确认条目。" },
  { key: "trajectory", title: "怎么变过来的", empty: "还没有形成轨迹。" },
  { key: "watching", title: "正在关注", empty: "暂时没有持续关注的问题。" },
];

export default function ProfileCard({ onOpenSettings, refreshToken }: ProfileCardProps) {
  const { ensureReady } = useAnalysisSetup();
  const [snapshot, setSnapshot] = useState<ProfileSnapshot | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [expanded, setExpanded] = useState(false);
  const [generating, setGenerating] = useState(false);
  const requestId = useRef(0);

  async function load(showLoading: boolean) {
    const id = ++requestId.current;
    if (showLoading) setLoading(true);
    try {
      const result = await getMemoryProfile();
      if (requestId.current !== id) return;
      setSnapshot(result);
      setError("");
    } catch (reason) {
      if (requestId.current !== id) return;
      setError(errorMessage(reason));
    } finally {
      if (requestId.current === id) setLoading(false);
    }
  }

  useEffect(() => {
    void load(true);
  }, []);

  useEffect(() => {
    if (refreshToken > 0) void load(false);
  }, [refreshToken]);

  async function regenerate() {
    if (generating) return;
    await ensureReady({
      // 记忆生成属于依赖知识索引的功能：除分析模型外还需要嵌入模型。
      needEmbedding: true,
      action: () => void runRegeneration(),
    });
  }

  async function runRegeneration() {
    setGenerating(true);
    setError("");
    try {
      await generateMemorySnapshot({
        generationId: `profile-${Date.now()}`,
        viewKind: "profile",
        scope: { kind: "all", projectId: null },
        rangeStart: null,
        rangeEnd: null,
      });
      await load(false);
      setExpanded(true);
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setGenerating(false);
    }
  }

  const profile = snapshot?.profile ?? null;
  const empty = !loading && !snapshot?.profile;
  const stale = snapshot?.isStale && snapshot.profile !== null;

  return <section className="profile-card" aria-label="个人总述">
    <header className="profile-card-header">
      <div className="profile-card-heading">
        <span className="profile-card-icon" aria-hidden="true"><SparklesIcon size={16} /></span>
        <div>
          <h3>个人总述</h3>
          <p>{profile ? "从你确认过的记忆条目聚合而来" : "把你的记忆条目聚合成一份画像"}</p>
        </div>
      </div>
      <div className="profile-card-actions">
        {stale && <span className="profile-card-stale" role="status"><TriangleAlertIcon size={13} /> 记忆已更新</span>}
        {!loading && !generating && <button type="button" className="secondary-button" onClick={() => void regenerate()}>{profile ? "重新生成" : "生成总述"}</button>}
        {generating && <span className="profile-card-busy" role="status" aria-live="polite">正在生成…</span>}
      </div>
    </header>

    {loading && <p className="profile-card-loading">正在读取个人总述…</p>}

    {!loading && error && <div className="profile-card-error" role="alert">
      <p>{error}</p>
      <button type="button" onClick={() => void load(true)}>重试</button>
      <button type="button" onClick={onOpenSettings}>检查外部 AI 设置</button>
    </div>}

    {!loading && empty && !error && <p className="profile-card-hint">
      生成总述需要先有认知演化快照。它会汇总你确认过的观点变化，生成一份「我怎么看、怎么变」的画像。
    </p>}

    {!loading && profile && (expanded ? (
      <div className="profile-card-body">
        {profile.summary && <p className="profile-card-summary">{profile.summary}</p>}
        {SECTIONS.map((section) => {
          const items = profile[section.key] as string[];
          return <div className="profile-card-section" key={section.key}>
            <h4>{section.title}</h4>
            {items.length ? <ul>{items.map((item, index) => <li key={index}>{item}</li>)}</ul> : <p>{section.empty}</p>}
          </div>;
        })}
      </div>
    ) : (
      <button type="button" className="profile-card-summary-line" onClick={() => setExpanded(true)}>
        {profile.summary || "展开查看我的立场、偏好与变化轨迹"}
      </button>
    ))}
    {!loading && profile && expanded && <button type="button" className="profile-card-collapse" onClick={() => setExpanded(false)}>收起</button>}
  </section>;
}

function errorMessage(reason: unknown) {
  const message = reason instanceof Error ? reason.message : String(reason);
  return message.replace(/^Error:\s*/i, "") || "发生未知错误，请稍后重试。";
}
