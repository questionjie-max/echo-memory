import { useEffect, useRef, useState } from "react";
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

function draftSummary(draft: TemplateDraft): string {
  return `已生成草案「${draft.name}」，栏目：${draft.sections
    .map((section) => section.title)
    .join("、")}。不满意就直接说要调整哪里（增删栏目、改侧重、换名字都可以），满意就点下方按钮入库。`;
}

/**
 * AI 模板向导（对话式）：用户描述使用场景 → 本地模型生成模板 →
 * 用户「满意」入库 /「不满意」继续用自然语言调整 → 循环直至满意。
 */
export default function TemplateWizard({ onClose, onCreated }: Props) {
  const [turns, setTurns] = useState<WizardTurn[]>([]);
  const [draft, setDraft] = useState<TemplateDraft | null>(null);
  const [input, setInput] = useState("");
  const [busy, setBusy] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const inputRef = useRef<HTMLTextAreaElement | null>(null);
  const scrollRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (scrollRef.current) scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
  }, [turns, draft, busy]);

  async function generate(allTurns: WizardTurn[]) {
    const result = await generateTemplateDraft(
      allTurns.map((turn) => ({ role: turn.role, content: turn.content })),
    );
    setDraft(result);
    setTurns((current) => [...current, { role: "assistant", content: draftSummary(result) }]);
  }

  async function send() {
    const content = input.trim();
    if (!content || busy) return;
    const nextTurns: WizardTurn[] = [...turns, { role: "user", content }];
    setTurns(nextTurns);
    setInput("");
    setBusy(true);
    setError(null);
    try {
      await generate(nextTurns);
    } catch (reason) {
      setError(
        `${reason instanceof Error ? reason.message : String(reason)}（可换个说法再试一次）`,
      );
    } finally {
      setBusy(false);
    }
  }

  async function confirmCreate() {
    if (!draft) return;
    setSaving(true);
    setError(null);
    try {
      await createAnalysisTemplate({
        name: draft.name,
        description: draft.description,
        focusInstructions: `按「${draft.name}」模板进行结构化分析，覆盖以下栏目：${draft.sections
          .map((section) => section.title)
          .join("、")}。`,
        customSections: draft.sections.map((section) => ({
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

  const hasDescribed = turns.some((turn) => turn.role === "user");

  return (
    <div
      className="dialog-scrim"
      role="presentation"
      onMouseDown={(event) => event.target === event.currentTarget && onClose()}
    >
      <section className="template-wizard" role="dialog" aria-modal="true" aria-label="AI 生成分析模板">
        <header className="template-wizard-header">
          <div>
            <p className="pane-eyebrow">本地模型 · 对话生成</p>
            <h3>AI 模板向导</h3>
          </div>
          <button type="button" className="close-button" onClick={onClose} aria-label="关闭">×</button>
        </header>

        <div className="template-wizard-body">
          <div className="template-wizard-chat" ref={scrollRef} aria-live="polite">
            {turns.length === 0 && (
              <p className="dock-empty">
                描述你的使用场景，比如：「我做客户访谈，想要一个重点挖痛点和竞品对比的模板，再加一个报价讨论栏目」。
                生成后不满意可以继续说怎么改，满意再入库。
              </p>
            )}
            {turns.map((turn, index) => (
              <article key={index} className={`dock-message ${turn.role}`}>
                <p>{turn.content}</p>
              </article>
            ))}

            {draft && (
              <div className="wizard-preview-card">
                <header>
                  <h4>模板草案（可改名）</h4>
                  <span className="wizard-feedback-hint">{draft.sections.length} 个栏目</span>
                </header>
                <input
                  className="editable-name"
                  value={draft.name}
                  aria-label="模板名称"
                  onChange={(event) => setDraft({ ...draft, name: event.target.value })}
                />
                <div className="wizard-section-chips">
                  {draft.sections.map((section) => (
                    <span key={section.key} title={section.instruction}>
                      {section.title}
                      <small> · {section.format === "list" ? "列表" : "段落"}</small>
                    </span>
                  ))}
                </div>
                <div className="wizard-preview-actions">
                  <button
                    type="button"
                    className="primary-button"
                    disabled={saving || !draft.name.trim()}
                    onClick={() => void confirmCreate()}
                  >
                    {saving ? "添加中…" : "满意，添加到模板库"}
                  </button>
                  <button
                    type="button"
                    className="toolbar-button"
                    onClick={() => inputRef.current?.focus()}
                  >
                    不满意，继续调整
                  </button>
                </div>
              </div>
            )}

            {busy && (
              <p className="dock-status" role="status">
                本地模型正在{hasDescribed && draft ? "按你的反馈调整" : "生成"}模板…
              </p>
            )}
          </div>

          <div className="dock-composer">
            <textarea
              ref={inputRef}
              value={input}
              rows={2}
              placeholder={
                draft ? "说说要调整的地方，比如「加一个风险提示栏目」「去掉竞品对比」…" : "描述你的使用场景…（⌘↵ 发送）"
              }
              onChange={(event) => setInput(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Enter" && (event.metaKey || event.ctrlKey) && !event.nativeEvent.isComposing) {
                  event.preventDefault();
                  void send();
                }
              }}
            />
            <button
              type="button"
              className="primary-button"
              disabled={busy || !input.trim()}
              onClick={() => void send()}
            >
              发送
            </button>
          </div>

          {error && <p className="inline-error" role="alert">{error}</p>}
        </div>
      </section>
    </div>
  );
}
