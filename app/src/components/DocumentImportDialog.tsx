import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { useEffect, useRef, useState } from "react";
import { getRecord, importDocument } from "../lib/tauri";
import type { IngestResult, RecordBrief } from "../shared/types";
import "./document-import.css";

interface DocumentImportDialogProps {
  open: boolean;
  projectId: string | null;
  onClose: () => void;
  onImported: (record: RecordBrief, created: boolean) => void;
}

type ImportPhase = "idle" | "importing" | "duplicate" | "success";

type SelectedDocument = {
  path: string;
  name: string;
  extension: "md" | "markdown" | "txt" | "docx";
};

const FORMAT_LABELS: Record<SelectedDocument["extension"], string> = {
  md: "Markdown",
  markdown: "Markdown",
  txt: "纯文本",
  docx: "Word 文档",
};

const ALLOWED_EXTENSIONS = new Set<SelectedDocument["extension"]>(["md", "markdown", "txt", "docx"]);

export default function DocumentImportDialog({
  open,
  projectId,
  onClose,
  onImported,
}: DocumentImportDialogProps) {
  const [selected, setSelected] = useState<SelectedDocument | null>(null);
  const [phase, setPhase] = useState<ImportPhase>("idle");
  const [error, setError] = useState("");
  const [successTitle, setSuccessTitle] = useState("");
  const [analysisQueued, setAnalysisQueued] = useState(false);
  const [duplicateRecord, setDuplicateRecord] = useState<RecordBrief | null>(null);
  const [usedExistingRecord, setUsedExistingRecord] = useState(false);
  const selectButtonRef = useRef<HTMLButtonElement>(null);
  const requestVersionRef = useRef(0);

  const busy = phase === "importing";

  useEffect(() => {
    requestVersionRef.current += 1;
    if (!open) return;

    setSelected(null);
    setPhase("idle");
    setError("");
    setSuccessTitle("");
    setAnalysisQueued(false);
    setDuplicateRecord(null);
    setUsedExistingRecord(false);
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

  async function chooseDocument() {
    if (busy) return;

    setError("");
    try {
      const selection = await openDialog({
        multiple: false,
        directory: false,
        title: "选择要导入的文字文档",
        filters: [
          { name: "文字文档", extensions: ["md", "markdown", "txt", "docx"] },
        ],
      });
      if (!selection) return;

      const sourcePath = Array.isArray(selection) ? selection[0] : selection;
      if (!sourcePath) return;

      const document = parseSelectedDocument(sourcePath);
      if (!document) {
        setError("暂不支持该文件格式。请选择 Markdown、TXT 或 DOCX 文件。");
        return;
      }

      setSelected(document);
      setPhase("idle");
      setSuccessTitle("");
      setAnalysisQueued(false);
      setDuplicateRecord(null);
      setUsedExistingRecord(false);
    } catch (reason) {
      setError(readableError(reason, "无法打开文件选择器，请稍后重试。"));
    }
  }

  function clearSelection() {
    if (busy) return;
    setSelected(null);
    setPhase("idle");
    setError("");
    setSuccessTitle("");
    setAnalysisQueued(false);
    setDuplicateRecord(null);
    setUsedExistingRecord(false);
    window.setTimeout(() => selectButtonRef.current?.focus(), 0);
  }

  async function runImport(duplicateConfirmed: boolean) {
    if (!selected || busy) return;

    const requestVersion = ++requestVersionRef.current;
    setPhase("importing");
    setError("");

    try {
      const result: IngestResult = await importDocument({
        sourcePath: selected.path,
        projectId,
        duplicateConfirmed,
      });

      if (requestVersion !== requestVersionRef.current) return;

      if (result.duplicate && !duplicateConfirmed) {
        const existingRecord = await getRecord(result.recordId);
        if (requestVersion !== requestVersionRef.current) return;
        setDuplicateRecord(existingRecord);
        setSuccessTitle(result.title || existingRecord.title);
        setPhase("duplicate");
        return;
      }

      const record = await getRecord(result.recordId);
      if (requestVersion !== requestVersionRef.current) return;

      setSuccessTitle(result.title || record.title || selected.name);
      setAnalysisQueued(true);
      setDuplicateRecord(null);
      setUsedExistingRecord(false);
      setPhase("success");
      onImported(record, true);
    } catch (reason) {
      if (requestVersion !== requestVersionRef.current) return;
      setPhase("idle");
      setError(readableError(reason, "导入失败，请确认文档未损坏后重试。"));
    }
  }

  function useExistingRecord() {
    if (!duplicateRecord) return;
    setUsedExistingRecord(true);
    setAnalysisQueued(false);
    setPhase("success");
    onImported(duplicateRecord, false);
  }

  function closeDialog() {
    if (!busy) onClose();
  }

  if (!open) return null;

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
        aria-labelledby="document-import-title"
        aria-describedby="document-import-description"
      >
        <header className="document-import-header">
          <div>
            <p className="document-import-eyebrow">知识库资料</p>
            <h2 id="document-import-title">导入文字文档</h2>
          </div>
          <button
            type="button"
            className="document-import-close"
            aria-label="关闭导入文档对话框"
            disabled={busy}
            onClick={closeDialog}
          >
            ×
          </button>
        </header>

        <div className="document-import-body">
          <p id="document-import-description" className="document-import-lead">
            从 Markdown、TXT 或 Word 文档提取文字，分析后加入知识库，之后可参与搜索和知识问答。
          </p>

          {!selected ? (
            <button
              ref={selectButtonRef}
              type="button"
              className="document-import-picker"
              onClick={() => void chooseDocument()}
            >
              <span className="document-import-picker-icon" aria-hidden="true">＋</span>
              <strong>选择一个文字文档</strong>
              <span>支持 .md、.markdown、.txt、.docx</span>
            </button>
          ) : (
            <section className="document-import-file" aria-label="已选择的文档">
              <div className="document-import-file-icon" aria-hidden="true">
                {selected.extension === "docx" ? "W" : "T"}
              </div>
              <div className="document-import-file-copy">
                <strong title={selected.name}>{selected.name}</strong>
                <dl>
                  <div>
                    <dt>格式</dt>
                    <dd>{FORMAT_LABELS[selected.extension]}</dd>
                  </div>
                  <div>
                    <dt>导入到</dt>
                    <dd>{projectId ? "当前知识库" : "未归档资料"}</dd>
                  </div>
                </dl>
              </div>
              {phase !== "success" && (
                <button type="button" className="document-import-link" disabled={busy} onClick={clearSelection}>
                  取消选择
                </button>
              )}
            </section>
          )}

          <section className="document-import-explainer" aria-label="导入处理说明">
            <div>
              <span className="document-import-step-number">1</span>
              <p><strong>读取文字</strong><span>保留标题和段落结构，不修改原文件。</span></p>
            </div>
            <div>
              <span className="document-import-step-number">2</span>
              <p><strong>分析内容</strong><span>提炼主题、结论和可检索的知识信息。</span></p>
            </div>
            <div>
              <span className="document-import-step-number">3</span>
              <p><strong>写入知识库</strong><span>导入完成后可在记录详情查看处理状态。</span></p>
            </div>
          </section>

          <aside className="document-import-privacy">
            <span aria-hidden="true">⌾</span>
            <p>
              <strong>隐私说明</strong>
              <span>文件在本机读取和保存。若当前分析配置使用外部 AI，提取出的文字会按你的现有设置发送；原始文档不会直接上传。</span>
            </p>
          </aside>

          {phase === "importing" && (
            <div className="document-import-progress" role="status" aria-live="polite">
              <div>
                <strong>正在导入并分析…</strong>
                <span>请保持应用开启</span>
              </div>
              <div className="document-import-progress-track" aria-hidden="true"><span /></div>
              <p>正在读取文字、建立记录并写入知识库，较长的文档可能需要一些时间。</p>
            </div>
          )}

          {phase === "duplicate" && (
            <div className="document-import-duplicate" role="alert" aria-labelledby="document-duplicate-title">
              <strong id="document-duplicate-title">这个文档已经导入过</strong>
              <p>知识库中已有“{successTitle || selected?.name}”。你可以保留原记录，或创建一份独立副本。</p>
              <div>
                <button type="button" className="document-import-secondary" onClick={useExistingRecord}>使用已有记录</button>
                <button type="button" className="document-import-primary" onClick={() => void runImport(true)}>创建副本</button>
              </div>
            </div>
          )}

          {phase === "success" && (
            <div className="document-import-success" role="status" aria-live="polite">
              <span aria-hidden="true">✓</span>
              <p>
                <strong>{usedExistingRecord ? `“${successTitle || selected?.name}”已存在于知识库` : `“${successTitle || selected?.name}”已导入`}</strong>
                <span>{usedExistingRecord ? "未创建重复副本，已定位到原有记录。" : analysisQueued ? "内容已写入，分析任务正在后台进行。" : "内容已写入知识库，可在记录详情查看分析状态。"}</span>
              </p>
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
                disabled={!selected || busy || phase === "duplicate"}
                onClick={() => void runImport(false)}
              >
                {busy ? "处理中…" : "导入并分析"}
              </button>
            </>
          )}
        </footer>
      </section>
    </div>
  );
}

function parseSelectedDocument(sourcePath: string): SelectedDocument | null {
  const name = sourcePath.split(/[\\/]/).pop()?.trim() || "未命名文档";
  const extension = name.includes(".") ? name.split(".").pop()?.toLowerCase() : "";
  if (!extension || !ALLOWED_EXTENSIONS.has(extension as SelectedDocument["extension"])) return null;
  return { path: sourcePath, name, extension: extension as SelectedDocument["extension"] };
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
