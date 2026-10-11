import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { useEffect, useRef, useState } from "react";
import { importChatExports } from "../lib/tauri";
import type { ChatImportOutcome } from "../shared/types";
import { BotIcon, CheckIcon, PlusIcon, ShieldCheckIcon } from "./icons";
import "./document-import.css";

interface ChatImportDialogProps {
  open: boolean;
  projectId: string | null;
  onClose: () => void;
  onImported: () => void;
}

type ImportPhase = "idle" | "importing" | "success";

const ALLOWED_EXTENSIONS = new Set(["json", "md", "markdown", "txt"]);

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
  const selectButtonRef = useRef<HTMLButtonElement>(null);

  const busy = phase === "importing";

  useEffect(() => {
    if (!open) return;
    setSelectedPaths([]);
    setPhase("idle");
    setError("");
    setOutcome(null);
    setAllowDuplicate(false);
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

  async function runImport() {
    if (selectedPaths.length === 0 || busy) return;
    setPhase("importing");
    setError("");
    try {
      const result = await importChatExports({
        sourcePaths: selectedPaths,
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
            <h2 id="chat-import-title">导入 AI 对话</h2>
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
          <p id="chat-import-description" className="document-import-lead">
            把与 ChatGPT / Claude 的对话记录导入资料库。对话会带「用户 / 助手」说话人标签，
            之后可参与搜索、知识问答，并进入认知演化分析。
          </p>

          {phase !== "success" && (
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
          )}

          {selectedPaths.length > 0 && phase !== "success" && (
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

          {phase === "importing" && (
            <div className="document-import-progress" role="status" aria-live="polite">
              <div>
                <strong>正在解析并写入…</strong>
                <span>请保持应用开启</span>
              </div>
              <div className="document-import-progress-track" aria-hidden="true"><span /></div>
              <p>正在逐会话解析导出文件并写入资料库；索引和分析会在后台继续进行。</p>
            </div>
          )}

          {phase === "success" && (
            <div className="document-import-success" role="status" aria-live="polite">
              <span aria-hidden="true"><CheckIcon size={12} /></span>
              <p>
                <strong>成功导入 {importedCount} 个会话</strong>
                <span>{skipped.length > 0 ? `${skipped.length} 个会话被跳过，明细见下方。` : "索引和分析正在后台进行。"}</span>
              </p>
            </div>
          )}

          {phase === "success" && skipped.length > 0 && (
            <div className="chat-import-skipped" role="list" aria-label="跳过明细">
              <strong>跳过的会话</strong>
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

function readableError(reason: unknown, fallback: string): string {
  if (reason instanceof Error && reason.message.trim()) return reason.message;
  if (typeof reason === "string" && reason.trim()) return reason;
  if (reason && typeof reason === "object" && "message" in reason) {
    const message = String((reason as { message?: unknown }).message ?? "").trim();
    if (message) return message;
  }
  return fallback;
}
