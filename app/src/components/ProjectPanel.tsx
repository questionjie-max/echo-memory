import { useEffect, useState } from "react";
import type { McpStatus, Project } from "../shared/types";
import {
  createProject,
  deleteProject,
  getMcpStatus,
  listProjects,
  setMcpEnabled,
  updateProject,
} from "../lib/tauri";

interface Props {
  selectedScope: string;
  refreshKey: number;
  onSelect: (scope: string) => void;
}

export default function ProjectPanel({ selectedScope, refreshKey, onSelect }: Props) {
  const [projects, setProjects] = useState<Project[]>([]);
  const [mcp, setMcp] = useState<McpStatus | null>(null);
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [mcpFeedback, setMcpFeedback] = useState("");

  async function refresh() {
    setError("");
    try {
      const [items, status] = await Promise.all([listProjects(), getMcpStatus()]);
      setProjects(items);
      setMcp(status);
    } catch (reason) {
      setError(String(reason));
    }
  }

  useEffect(() => {
    void refresh();
  }, [refreshKey]);

  async function onCreate() {
    if (!name.trim()) return;
    setBusy(true);
    setError("");
    try {
      const project = await createProject(name.trim());
      setName("");
      await refresh();
      onSelect(project.id);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  async function rename(project: Project) {
    const nextName = window.prompt("知识库名称", project.name)?.trim();
    if (!nextName || nextName === project.name) return;
    setBusy(true);
    try {
      await updateProject(project.id, { name: nextName });
      await refresh();
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  async function toggleArchive(project: Project) {
    setBusy(true);
    try {
      await updateProject(project.id, {
        status: project.status === "active" ? "archived" : "active",
      });
      if (project.id === selectedScope && project.status === "active") onSelect("all");
      await refresh();
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  async function remove(project: Project) {
    if (!window.confirm(`删除知识库“${project.name}”？其中的录音会变为未归档，不会被删除。`)) return;
    setBusy(true);
    try {
      await deleteProject(project.id);
      if (project.id === selectedScope) onSelect("unfiled");
      await refresh();
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  async function toggleMcp(enabled: boolean) {
    setBusy(true);
    try {
      setMcp(await setMcpEnabled(enabled));
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  async function copyMcpPath() {
    if (!mcp?.executablePath) return;
    setMcpFeedback("");
    try {
      await navigator.clipboard.writeText(mcp.executablePath);
      setMcpFeedback("路径已复制");
    } catch (reason) {
      setMcpFeedback(`复制失败：${String(reason)}`);
    }
  }

  const active = projects.filter((project) => project.status === "active");
  const archived = projects.filter((project) => project.status === "archived");
  const recentlyConnected = Boolean(
    mcp?.recentCalls[0]
      && Date.now() - new Date(mcp.recentCalls[0].calledAt).getTime() < 5 * 60 * 1000,
  );

  return (
    <aside className="sidebar" aria-label="知识库">
      <div className="pane-heading">
        <div>
          <p className="pane-eyebrow">资料组织</p>
          <h2>知识库</h2>
        </div>
        <span className="count-badge">{active.length}</span>
      </div>

      <nav className="scope-list" aria-label="知识库范围">
        <ScopeButton label="全部记录" selected={selectedScope === "all"} onClick={() => onSelect("all")} />
        <ScopeButton label="未归档" selected={selectedScope === "unfiled"} onClick={() => onSelect("unfiled")} />
      </nav>

      <div className="sidebar-section-heading">
        <span>我的知识库</span>
      </div>

      <form
        className="knowledge-create"
        onSubmit={(event) => {
          event.preventDefault();
          void onCreate();
        }}
      >
        <input value={name} onChange={(event) => setName(event.target.value)} placeholder="新知识库名称" aria-label="新知识库名称" />
        <button type="submit" className="icon-button" disabled={busy || !name.trim()} aria-label="新建知识库" title="新建知识库">+</button>
      </form>

      <div className="knowledge-list">
        {active.map((project) => (
          <KnowledgeRow
            key={project.id}
            project={project}
            selected={selectedScope === project.id}
            busy={busy}
            onSelect={() => onSelect(project.id)}
            onRename={() => void rename(project)}
            onArchive={() => void toggleArchive(project)}
            onDelete={() => void remove(project)}
          />
        ))}
        {active.length === 0 && <p className="sidebar-empty">还没有知识库。</p>}
      </div>

      {archived.length > 0 && (
        <details className="archived-list">
          <summary>已归档知识库（{archived.length}）</summary>
          {archived.map((project) => (
            <div className="archived-row" key={project.id}>
              <span>{project.name}</span>
              <button type="button" disabled={busy} onClick={() => void toggleArchive(project)}>恢复</button>
              <button type="button" className="danger-text" disabled={busy} onClick={() => void remove(project)}>删除</button>
            </div>
          ))}
        </details>
      )}

      <section className="mcp-section" aria-label="AI 连接">
        <div className="mcp-title-row">
          <div>
            <strong>AI 连接</strong>
            <span>{!mcp?.executableAvailable ? "MCP 组件未安装" : mcp.enabled ? (recentlyConnected ? "最近有连接" : "等待客户端连接") : "未启用"}</span>
          </div>
          <label className="switch" title={mcp?.executableAvailable ? "启用只读 MCP" : "MCP 组件未安装"}>
            <input
              type="checkbox"
              checked={mcp?.enabled ?? false}
              disabled={busy || !mcp?.executableAvailable}
              onChange={(event) => void toggleMcp(event.target.checked)}
            />
            <span aria-hidden="true" />
          </label>
        </div>
        <p>{mcp?.authorizedScope ?? "全部知识库（只读）"}</p>
        {mcp?.executablePath && (
          <details className="mcp-path">
            <summary>MCP 组件路径</summary>
            <code title={mcp.executablePath}>{mcp.executablePath}</code>
            <button type="button" onClick={() => void copyMcpPath()}>复制路径</button>
            {mcpFeedback && <span role="status">{mcpFeedback}</span>}
          </details>
        )}
        {mcp?.recentCalls.length ? (
          <details className="mcp-calls">
            <summary>最近调用</summary>
            {mcp.recentCalls.map((call, index) => (
              <span key={`${call.calledAt}-${index}`}>{call.toolName} · {new Date(call.calledAt).toLocaleTimeString()}</span>
            ))}
          </details>
        ) : null}
      </section>

      {error && <p className="inline-error">{error}</p>}
    </aside>
  );
}

function ScopeButton({ label, selected, onClick }: { label: string; selected: boolean; onClick: () => void }) {
  return <button type="button" className={`scope-button${selected ? " selected" : ""}`} onClick={onClick}>{label}</button>;
}

function KnowledgeRow({ project, selected, busy, onSelect, onRename, onArchive, onDelete }: {
  project: Project;
  selected: boolean;
  busy: boolean;
  onSelect: () => void;
  onRename: () => void;
  onArchive: () => void;
  onDelete: () => void;
}) {
  return (
    <div className={`knowledge-row${selected ? " selected" : ""}`}>
      <button type="button" className="knowledge-main" onClick={onSelect}>{project.name}</button>
      {selected && (
        <details className="row-menu">
          <summary aria-label={`${project.name}操作`} title="知识库操作">•••</summary>
          <div className="row-menu-popover">
            <button type="button" disabled={busy} onClick={onRename}>重命名</button>
            <button type="button" disabled={busy} onClick={onArchive}>归档</button>
            <button type="button" className="danger-text" disabled={busy} onClick={onDelete}>删除</button>
          </div>
        </details>
      )}
    </div>
  );
}
