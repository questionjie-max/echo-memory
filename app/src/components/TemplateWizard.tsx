import { useState } from "react";
import type { TemplateDraft } from "../shared/types";
import { createAnalysisTemplate, generateTemplateDraft } from "../lib/tauri";

interface Props {
  onClose: () => void;
  onCreated: () => void;
}

interface WizardTurn {
  role: "user" | "assistant";
  content: string;
}

/**
 * AI 模板向导：对话描述需求 → 本地模型生成分析模板草稿 → 预览确认入库。
 * 对话只在向导内保留，不进入 AI 伙伴的会话历史。
 */
export default function TemplateWizard({ onClose, onCreated }: Props) {
  const [turns, setTurns] = useState<WizardTurn[]>([]);
  const [draft, setDraft] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [preview, setPreview] = useState<TemplateDraft | null>(null);
  const [saving, setSaving] = useState(false);

  async function send() {
    const content = draft.trim();
    if (!content || busy) return;
    setBusy(true);
    setError(null);
    setTurns((current) => [...current, { role: "user", content }]);
    setDraft("");
    // 向导不接外部引擎：先回填一条引导性回复，用户可继续补充后点击生成。
    setTurns((current) => [
      ...current,
      {
        role: "assistant",
        content:
          "收到。如果还有补充（重点栏目、输出格式、行业场景）请继续说；没有的话，点击「生成模板草稿」。",
      },
    ]);
    setBusy(false);
  }

  async function generate() {
    if (turns.filter((turn) => turn.role === "user").length === 0) {
      setError("请先描述你的模板需求");
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const result = await generateTemplateDraft(
        turns.map((turn) => ({ role: turn.role, content: turn.content })),
      );
      setPreview(result);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setBusy(false);
    }
  }

  async function confirmCreate() {
    if (!preview) return;
    setSaving(true);
    setError(null);
    try {
      await createAnalysisTemplate({
        name: preview.name,
        description: preview.description,
        focusInstructions: `按「${preview.name}」模板进行结构化分析，覆盖以下栏目：${preview.sections
          .map((section) => section.title)
          .join("、")}。`,
        customSections: preview.sections.map((section) => ({
          key: section.key,
          title: section.title,
          format: section.format === "list" ? "list" : "paragraph",
          instruction: section.instruction,
        })),
      });
      onCreated();
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setSaving(false);
    }
  }

  function patchPreview(patch: Partial<TemplateDraft>) {
    setPreview((current) => (current ? { ...current, ...patch } : current));
  }

  return (
    <div className="dialog-scrim" role="presentation" onMouseDown={(event) => event.target === event.currentTarget && onClose()}>
      <section className="template-wizard material" role="dialog" aria-modal="true" aria-label="AI 生成分析模板">
        <header className="template-wizard-header">
          <div>
            <p className="pane-eyebrow">本地模型生成</p>
            <h3>AI 模板向导</h3>
          </div>
          <button type="button" className="close-button" onClick={onClose} aria-label="关闭">×</button>
        </header>

        <div className="template-wizard-body">
          <div className="template-wizard-chat" aria-live="polite">
            {turns.length === 0 && (
              <p className="dock-empty">
                描述你需要的分析模板，比如：「客户访谈模板，重点挖痛点和竞品对比，要一个报价讨论栏目」。
              </p>
            )}
            {turns.map((turn, index) => (
              <article key={index} className={`dock-message ${turn.role}`}>
                <p>{turn.content}</p>
              </article>
            ))}
          </div>

          {!preview && (
            <div className="dock-composer">
              <textarea
                value={draft}
                rows={2}
                placeholder="描述你的模板需求…（⌘↵ 发送）"
                onChange={(event) => setDraft(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key === "Enter" && (event.metaKey || event.ctrlKey) && !event.nativeEvent.isComposing) {
                    event.preventDefault();
                    void send();
                  }
                }}
              />
              <button type="button" className="primary-button" disabled={busy || !draft.trim()} onClick={() => void send()}>
                补充
              </button>
              <button type="button" className="primary-button" disabled={busy || turns.length === 0} onClick={() => void generate()}>
                {busy ? "生成中…" : "生成模板草稿"}
              </button>
            </div>
          )}

          {preview && (
            <div className="template-wizard-preview">
              <h4>模板预览（可编辑）</h4>
              <label>
                <span>模板名称</span>
                <input value={preview.name} onChange={(event) => patchPreview({ name: event.target.value })} />
              </label>
              <label>
                <span>描述</span>
                <input
                  value={preview.description}
                  onChange={(event) => patchPreview({ description: event.target.value })}
                />
              </label>
              <div className="template-wizard-sections">
                {preview.sections.map((section, index) => (
                  <div className="template-wizard-section" key={section.key}>
                    <strong>{section.title}</strong>
                    <input
                      value={section.title}
                      onChange={(event) => {
                        const sections = [...preview.sections];
                        sections[index] = { ...section, title: event.target.value };
                        patchPreview({ sections });
                      }}
                    />
                    <small>{section.format === "list" ? "列表" : "段落"} · {section.instruction}</small>
                  </div>
                ))}
              </div>
              <div className="template-wizard-actions">
                <button type="button" className="toolbar-button" onClick={() => setPreview(null)}>
                  重新对话
                </button>
                <button
                  type="button"
                  className="primary-button"
                  disabled={saving || !preview.name.trim() || preview.sections.length < 2}
                  onClick={() => void confirmCreate()}
                >
                  {saving ? "添加中…" : "添加到模板库"}
                </button>
              </div>
            </div>
          )}

          {error && <p className="inline-error" role="alert">{error}</p>}
        </div>
      </section>
    </div>
  );
}
