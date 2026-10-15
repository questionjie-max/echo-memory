import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { useEffect, useRef, useState } from "react";
import { importAgentScans, importChatExports, scanLocalAgents } from "../lib/tauri";
import type { AgentScanResult, ChatImportOutcome } from "../shared/types";
import { BotIcon, CheckIcon, PlusIcon, SearchIcon, ShieldCheckIcon } from "./icons";
import "./document-import.css";

interface ChatImportDialogProps {
  open: boolean;
  projectId: string | null;
  onClose: () => void;
  onImported: () => void;
}

type ImportPhase = "idle" | "importing" | "success";
/** null = 文件选择模式；否则处于扫描模式的对应步骤 */
type ScanPhase = "select-agents" | "scanning" | "results";

const ALLOWED_EXTENSIONS = new Set(["json", "md", "markdown", "txt"]);
const AGENT_LABELS: Record<string, string> = {
  "claude-code": "Claude Code",
  codex: "Codex",
};

export default function ChatImportDialog({
  open,
  projectId,
  onClose,
  onImported,
}: ChatImportDialogProps) {
  const [selectedPaths, setSelectedPaths] = useState<string[]>([]);
  const [phase, setPhase] = useState<ImportPhase>("idle");
  const [error, setError] = useState("");
  const [outcome, setOutcome] = useState<ChatImportOutcome | null>(null);
  const [allowDuplicate, setAllowDuplicate] = useState(false);
  const [scanPhase, setScanPhase] = useState<ScanPhase | null>(null);
  const [scanResult, setScanResult] = useState<AgentScanResult | null>(null);
  const [agentChecked, setAgentChecked] = useState<Record<string, boolean>>({
    "claude-code": true,
    codex: true,
  });
  const [memoryChecked, setMemoryChecked] = useState(true);
  const [selectedSessions, setSelectedSessions] = useState<Set<string>>(new Set());
  const [selectedMemory, setSelectedMemory] = useState<Set<string>>(new Set());
  const selectButtonRef = useRef<HTMLButtonElement>(null);

  const busy = phase === "importing" || scanPhase === "scanning";

  useEffect(() => {
    if (!open) return;
    resetAll();
    window.setTimeout(() => selectButtonRef.current?.focus(), 0);
  }, [open]);

  useEffect(() => {
    if (!open) return;

    function handleKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape" && !busy) onClose();
    }

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [busy, onClose, open]);

  function resetAll() {
    setSelectedPaths([]);
    setPhase("idle");
    setError("");
    setOutcome(null);
    setAllowDuplicate(false);
    setScanPhase(null);
    setScanResult(null);
    setAgentChecked({ "claude-code": true, codex: true });
    setMemoryChecked(true);
    setSelectedSessions(new Set());
    setSelectedMemory(new Set());
  }

  async function chooseFiles() {
    if (busy) return;
    setError("");
    try {
      const selection = await openDialog({
        multiple: true,
        directory: false,
        title: "选择 AI 对话导出文件",
        filters: [{ name: "AI 对话导出", extensions: ["json", "md", "markdown", "txt"] }],
      });
      if (!selection) return;
      const paths = (Array.isArray(selection) ? selection : [selection]).filter(Boolean);
      const supported = paths.filter((path) => {
        const extension = path.split(".").pop()?.toLowerCase() ?? "";
        return ALLOWED_EXTENSIONS.has(extension);
      });
      if (supported.length === 0) {
        setError("暂不支持所选文件。请选择 .json 导出文件或对话式 .md/.txt 文本。");
        return;
      }
      if (supported.length < paths.length) {
        setError(`已忽略 ${paths.length - supported.length} 个不支持的文件。`);
      } else {
        setError("");
      }
      setSelectedPaths(supported);
      setOutcome(null);
      setPhase("idle");
    } catch (reason) {
      setError(readableError(reason, "无法打开文件选择器，请稍后重试。"));
    }
  }

  async function runScan() {
    if (busy) return;
    setScanPhase("scanning");
    setError("");
    try {
      const result = await scanLocalAgents();
      const sessions = new Set(result.sessions.filter((session) => !session.oversized).map((session) => session.path));
      const memory = new Set(result.memoryFiles.map((file) => file.path));
      setScanResult(result);
      setSelectedSessions(sessions);
      setSelectedMemory(memoryChecked ? memory : new Set());
      setScanPhase("results");
    } catch (reason) {
      setScanPhase("select-agents");
      setError(readableError(reason, "扫描失败，请稍后重试。"));
    }
  }

  function toggleSession(path: string) {
    setSelectedSessions((current) => {
      const next = new Set(current);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  }

  function toggleMemory(path: string) {
    setSelectedMemory((current) => {
      const next = new Set(current);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  }

  function toggleAgentGroup(agent: string) {
    if (!scanResult) return;
    const groupPaths = scanResult.sessions.filter((session) => session.agent === agent && !session.oversized).map((session) => session.path);
    const allSelected = groupPaths.every((path) => selectedSessions.has(path));
    setSelectedSessions((current) => {
      const next = new Set(current);
      for (const path of groupPaths) {
        if (allSelected) next.delete(path);
        else next.add(path);
      }
      return next;
    });
  }

  async function runImport() {
    if (busy) return;
    const fromFiles = scanPhase === null && selectedPaths.length > 0;
    const sessionSelections = (scanResult?.sessions ?? [])
      .filter((session) => selectedSessions.has(session.path))
      .map((session) => ({ agent: session.agent, path: session.path }));
    const memoryPaths = (scanResult?.memoryFiles ?? [])
      .filter((file) => selectedMemory.has(file.path))
      .map((file) => file.path);
    if (!fromFiles && sessionSelections.length === 0 && memoryPaths.length === 0) return;

    setPhase("importing");
    setError("");
    try {
      const result = fromFiles
        ? await importChatExports({
            sourcePaths: selectedPaths,
            projectId,
            duplicateConfirmed: allowDuplicate,
          })
        : await importAgentScans({
            sessions: sessionSelections,
            memoryPaths,
            projectId,
            duplicateConfirmed: allowDuplicate,
          });
      setOutcome(result);
      setPhase("success");
      if (result.imported.length > 0) onImported();
    } catch (reason) {
      setPhase("idle");
      setError(readableError(reason, "导入失败，请确认文件内容后重试。"));
    }
  }

  function closeDialog() {
    if (!busy) onClose();
  }

  if (!open) return null;

  const importedCount = outcome?.imported.length ?? 0;
  const skipped = outcome?.skipped ?? [];
  const totalSelected =
    selectedSessions.size + selectedMemory.size + (scanPhase === null ? selectedPaths.length : 0);

  return (
    <div
      className="document-import-scrim"
      role="presentation"
      onMouseDown={(event) => event.target === event.currentTarget && closeDialog()}
    >
      <section
        className="document-import-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="chat-import-title"
        aria-describedby="chat-import-description"
      >
        <header className="document-import-header">
          <div>
            <p className="document-import-eyebrow">知识库资料</p>
            <h2 id="chat-import-title">{scanPhase ? "扫描本机 Agent 对话" : "导入 AI 对话"}</h2>
          </div>
          <button
            type="button"
            className="document-import-close"
            aria-label="关闭导入 AI 对话对话框"
            disabled={busy}
            onClick={closeDialog}
          >
            ×
          </button>
        </header>

        <div className="document-import-body">
          {scanPhase === null && (
            <>
              <p id="chat-import-description" className="document-import-lead">
                把与 ChatGPT / Claude 的对话记录导入资料库。对话会带「用户 / 助手」说话人标签，
                之后可参与搜索、知识问答，并进入认知演化分析。
              </p>
              <button
                ref={selectButtonRef}
                type="button"
                className="document-import-picker"
                onClick={() => void chooseFiles()}
              >
                <span className="document-import-picker-icon" aria-hidden="true"><PlusIcon size={18} /></span>
                <strong>{selectedPaths.length > 0 ? "重新选择文件" : "选择导出文件"}</strong>
                <span>支持 ChatGPT / Claude 导出的 .json，以及对话式 .md、.txt（可多选）</span>
              </button>
              <button
                type="button"
                className="chat-import-scan-entry"
                onClick={() => { setScanPhase("select-agents"); setError(""); }}
              >
                <span aria-hidden="true"><SearchIcon size={15} /></span>
                <strong>扫描本机 Agent 对话</strong>
                <span>自动查找 Claude Code、Codex 在本机的会话与记忆文件</span>
              </button>
              {selectedPaths.length > 0 && (
                <section className="document-import-file" aria-label="已选择的文件">
                  <div className="document-import-file-icon" aria-hidden="true">
                    <BotIcon size={16} />
                  </div>
                  <div className="document-import-file-copy">
                    <strong>已选择 {selectedPaths.length} 个文件</strong>
                    <dl>
                      <div>
                        <dt>导入到</dt>
                        <dd>{projectId ? "当前知识库" : "未归档资料"}</dd>
                      </div>
                    </dl>
                  </div>
                  <button type="button" className="document-import-link" disabled={busy} onClick={() => setSelectedPaths([])}>
                    取消选择
                  </button>
                </section>
              )}
              <ul className="chat-import-file-list" aria-label="文件清单">
                {selectedPaths.slice(0, 6).map((path) => (
                  <li key={path} title={path}>{path.split(/[\\/]/).pop()}</li>
                ))}
                {selectedPaths.length > 6 && <li>…等 {selectedPaths.length} 个文件</li>}
              </ul>
            </>
          )}

          {scanPhase === "select-agents" && (
            <>
              <p id="chat-import-description" className="document-import-lead">
                勾选要扫描的 Agent，应用只读取它们在本机的固定会话目录，不会扫描其他位置。
              </p>
              <div className="chat-import-agent-list" role="group" aria-label="选择要扫描的 Agent">
                {Object.entries(AGENT_LABELS).map(([agent, label]) => (
                  <label key={agent} className="chat-import-agent-row">
                    <input
                      type="checkbox"
                      checked={agentChecked[agent] ?? false}
                      disabled={busy}
                      onChange={(event) => setAgentChecked((current) => ({ ...current, [agent]: event.target.checked }))}
                    />
                    <span>{label}</span>
                    <small>{agent === "codex" ? "~/.codex/sessions" : "~/.claude/projects"}</small>
                  </label>
                ))}
                <label className="chat-import-agent-row">
                  <input
                    type="checkbox"
                    checked={memoryChecked}
                    disabled={busy}
                    onChange={(event) => setMemoryChecked(event.target.checked)}
                  />
                  <span>记忆 / 规则文件</span>
                  <small>CLAUDE.md、AGENTS.md、GEMINI.md（作为文档导入）</small>
                </label>
              </div>
              <aside className="document-import-privacy">
                <span aria-hidden="true"><ShieldCheckIcon size={18} /></span>
                <p>
                  <strong>隐私说明</strong>
                  <span>只读取上述白名单位置，绝不全盘扫描；扫到的每个文件在导入前都可见、可取消；全程本机完成，不上传任何内容。</span>
                </p>
              </aside>
              <div className="chat-import-scan-actions">
                <button type="button" className="document-import-secondary" disabled={busy} onClick={() => setScanPhase(null)}>返回</button>
                <button type="button" className="document-import-primary" disabled={busy || (!agentChecked["claude-code"] && !agentChecked["codex"] && !memoryChecked)} onClick={() => void runScan()}>
                  开始扫描
                </button>
              </div>
            </>
          )}

          {scanPhase === "scanning" && (
            <div className="document-import-progress" role="status" aria-live="polite">
              <div><strong>正在扫描本机…</strong><span>只读取白名单位置</span></div>
              <div className="document-import-progress-track" aria-hidden="true"><span /></div>
            </div>
          )}

          {scanPhase === "results" && scanResult && (
            <>
              <p className="document-import-lead">
                找到 {scanResult.sessions.length} 个会话、{scanResult.memoryFiles.length} 个记忆文件。勾选要导入的内容。
              </p>
              <div className="chat-import-scan-results">
                {Object.entries(AGENT_LABELS).map(([agent, label]) => {
                  const group = scanResult.sessions.filter((session) => session.agent === agent);
                  if (group.length === 0) return null;
                  const selectable = group.filter((session) => !session.oversized);
                  const allSelected = selectable.every((session) => selectedSessions.has(session.path));
                  return (
                    <section key={agent} className="chat-import-agent-group" aria-label={`${label} 会话`}>
                      <header>
                        <strong>{label}</strong>
                        <span>{group.length} 个会话</span>
                        <button type="button" onClick={() => toggleAgentGroup(agent)}>{allSelected ? "全不选" : "全选"}</button>
                      </header>
                      <ul>
                        {group.map((session) => (
                          <li key={session.path} className={session.oversized ? "oversized" : ""}>
                            <label>
                              <input
                                type="checkbox"
                                checked={selectedSessions.has(session.path)}
                                disabled={session.oversized}
                                onChange={() => toggleSession(session.path)}
                              />
                              <span className="chat-import-session-title" title={session.path}>{session.title}</span>
                              <small>{session.startedAt.slice(0, 10)} · {formatBytes(session.sizeBytes)}{session.oversized ? " · 过大" : ""}</small>
                            </label>
                          </li>
                        ))}
                      </ul>
                    </section>
                  );
                })}
                {scanResult.memoryFiles.length > 0 && (
                  <section className="chat-import-agent-group" aria-label="记忆文件">
                    <header>
                      <strong>记忆 / 规则文件</strong>
                      <span>{scanResult.memoryFiles.length} 个</span>
                      <button type="button" onClick={() => setSelectedMemory(selectedMemory.size === scanResult.memoryFiles.length ? new Set() : new Set(scanResult.memoryFiles.map((file) => file.path)))}>
                        {selectedMemory.size === scanResult.memoryFiles.length ? "全不选" : "全选"}
                      </button>
                    </header>
                    <ul>
                      {scanResult.memoryFiles.map((file) => (
                        <li key={file.path}>
                          <label>
                            <input
                              type="checkbox"
                              checked={selectedMemory.has(file.path)}
                              onChange={() => toggleMemory(file.path)}
                            />
                            <span className="chat-import-session-title" title={file.path}>{file.path.split("/").pop()}</span>
                            <small>{file.agentLabel} · {formatBytes(file.sizeBytes)}</small>
                          </label>
                        </li>
                      ))}
                    </ul>
                  </section>
                )}
              </div>
              <div className="chat-import-scan-actions">
                <button type="button" className="document-import-secondary" disabled={busy} onClick={() => setScanPhase("select-agents")}>上一步</button>
                <button
                  type="button"
                  className="document-import-primary"
                  disabled={busy || (selectedSessions.size === 0 && selectedMemory.size === 0)}
                  onClick={() => void runImport()}
                >
                  {busy ? "处理中…" : `导入所选（${totalSelected}）`}
                </button>
              </div>
            </>
          )}

          {scanPhase === null && (
            <>
              <section className="document-import-explainer" aria-label="导入处理说明">
                <div>
                  <span className="document-import-step-number">1</span>
                  <p><strong>解析对话</strong><span>按会话拆分，识别用户与助手发言，全部在本机完成。</span></p>
                </div>
                <div>
                  <span className="document-import-step-number">2</span>
                  <p><strong>写入资料库</strong><span>每个会话成为一条记录，重复内容会自动跳过。</span></p>
                </div>
                <div>
                  <span className="document-import-step-number">3</span>
                  <p><strong>后台分析</strong><span>导入完成后在后台提炼观点，可在认知演化中确认。</span></p>
                </div>
              </section>

              <aside className="document-import-privacy">
                <span aria-hidden="true"><ShieldCheckIcon size={18} /></span>
                <p>
                  <strong>隐私说明</strong>
                  <span>对话文件在本机读取和保存。若当前记忆分析使用外部 AI，提炼出的文字会按你的现有设置发送；原始对话不会直接上传。</span>
                </p>
              </aside>

              <label className="chat-import-duplicate-toggle">
                <input
                  type="checkbox"
                  checked={allowDuplicate}
                  disabled={busy}
                  onChange={(event) => setAllowDuplicate(event.target.checked)}
                />
                <span>重复的对话也导入为独立副本</span>
              </label>
            </>
          )}

          {phase === "importing" && (
            <div className="document-import-progress" role="status" aria-live="polite">
              <div>
                <strong>正在解析并写入…</strong>
                <span>请保持应用开启</span>
              </div>
              <div className="document-import-progress-track" aria-hidden="true"><span /></div>
              <p>正在逐会话解析并写入资料库；索引和分析会在后台继续进行。</p>
            </div>
          )}

          {phase === "success" && (
            <div className="document-import-success" role="status" aria-live="polite">
              <span aria-hidden="true"><CheckIcon size={12} /></span>
              <p>
                <strong>成功导入 {importedCount} 项</strong>
                <span>{skipped.length > 0 ? `${skipped.length} 项被跳过，明细见下方。` : "索引和分析正在后台进行。"}</span>
              </p>
            </div>
          )}

          {phase === "success" && skipped.length > 0 && (
            <div className="chat-import-skipped" role="list" aria-label="跳过明细">
              <strong>跳过的内容</strong>
              <ul>
                {skipped.map((item, index) => (
                  <li key={`${item.file}-${index}`} role="listitem">
                    <span>{item.title ?? item.file}</span>
                    <span>{item.reason}</span>
                  </li>
                ))}
              </ul>
            </div>
          )}

          {error && <p className="document-import-error" role="alert">{error}</p>}
        </div>

        <footer className="document-import-actions">
          {phase === "success" ? (
            <button type="button" className="document-import-primary" onClick={closeDialog}>完成</button>
          ) : scanPhase ? (
            <button type="button" className="document-import-secondary" disabled={busy} onClick={() => setScanPhase(null)}>用文件方式导入</button>
          ) : (
            <>
              <button type="button" className="document-import-secondary" disabled={busy} onClick={closeDialog}>取消</button>
              <button
                type="button"
                className="document-import-primary"
                disabled={selectedPaths.length === 0 || busy}
                onClick={() => void runImport()}
              >
                {busy ? "处理中…" : `导入并分析${selectedPaths.length > 0 ? `（${selectedPaths.length} 个文件）` : ""}`}
              </button>
            </>
          )}
        </footer>
      </section>
    </div>
  );
}

function formatBytes(bytes: number) {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(0)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function readableError(reason: unknown, fallback: string): string {
  if (reason instanceof Error && reason.message.trim()) return reason.message;
  if (typeof reason === "string" && reason.trim()) return reason;
  if (reason && typeof reason === "object" && "message" in reason) {
    const message = String((reason as { message?: unknown }).message ?? "").trim();
    if (message) return message;
  }
  return fallback;
}
