/**
 * 设置面板的构件层。
 *
 * 设计规则来自对 OpenLess 设置界面的拆解：按「用户带着什么问题来」组织信息、
 * 一个事实只有一个来源、列表即选择器、状态用「图标 + 文字 + 颜色」三重编码、
 * 每个可见控件都必须真的可用。视觉上：0.5px 发丝线、实底表面、999px 药丸。
 */
import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { CheckCircleIcon, TriangleAlertIcon, CircleXIcon, CheckIcon, CircleIcon } from "./icons";
import "./settings-kit.css";

/* ---------------- 面板骨架 ---------------- */

export interface NavItem {
  id: string;
  label: string;
}

export function PanelShell({
  brand,
  version,
  nav,
  active,
  onSelect,
  title,
  subtitle,
  onClose,
  children,
}: {
  brand: string;
  version?: string;
  nav: NavItem[];
  active: string;
  onSelect: (id: string) => void;
  title: string;
  subtitle?: string;
  onClose: () => void;
  children: ReactNode;
}) {
  return (
    <section className="em-panel" role="dialog" aria-modal="true" aria-label={title}>
      <nav className="em-rail" aria-label="设置分区">
        <div className="em-rail-brand">
          <strong>{brand}</strong>
          {version && <span>{version}</span>}
        </div>
        <div className="em-rail-group" role="tablist" aria-orientation="vertical">
          {nav.map((item) => (
            <button
              key={item.id}
              type="button"
              role="tab"
              className="em-nav-item"
              aria-selected={active === item.id}
              onClick={() => onSelect(item.id)}
            >
              <span className="em-nav-dot" aria-hidden="true" />
              {item.label}
            </button>
          ))}
        </div>
        <div className="em-rail-spacer" />
      </nav>
      <div className="em-main">
        <header className="em-main-head">
          <div>
            <h2>{title}</h2>
            {subtitle && <p>{subtitle}</p>}
          </div>
        </header>
        <button type="button" className="em-close" onClick={onClose} aria-label="关闭设置" title="关闭">
          ×
        </button>
        <div className="em-main-body">{children}</div>
      </div>
    </section>
  );
}

/* ---------------- 卡片 ---------------- */

export function Card({
  title,
  description,
  status,
  actions,
  footer,
  tone,
  children,
}: {
  title: string;
  description?: ReactNode;
  status?: ReactNode;
  actions?: ReactNode;
  footer?: ReactNode;
  tone?: "accent";
  children?: ReactNode;
}) {
  return (
    <section className={tone ? `em-card ${tone}` : "em-card"}>
      <div className="em-card-head">
        <div>
          <h3>{title}</h3>
          {description && <p>{description}</p>}
        </div>
        <div className="em-card-actions">
          {status}
          {actions}
        </div>
      </div>
      {children}
      {footer && <div className="em-card-foot">{footer}</div>}
    </section>
  );
}

export function Stack({ children }: { children: ReactNode }) {
  return <div className="em-stack">{children}</div>;
}

/* ---------------- 药丸 / 状态 ---------------- */

export type Tone = "default" | "ok" | "warn" | "err" | "accent";

const TONE_GLYPH: Record<Exclude<Tone, "default">, ReactNode> = {
  ok: <CheckCircleIcon size={11} />,
  warn: <TriangleAlertIcon size={11} />,
  err: <CircleXIcon size={11} />,
  accent: "•",
};

/** 状态一律「图标 + 文字 + 颜色」三重编码，不靠颜色单独表意。 */
export function Pill({ tone = "default", children }: { tone?: Tone; children: ReactNode }) {
  return (
    <span className={`em-pill ${tone}`}>
      {tone !== "default" && (
        <span className="em-pill-glyph" aria-hidden="true">
          {TONE_GLYPH[tone]}
        </span>
      )}
      {children}
    </span>
  );
}

/** 「已就绪 / 未就绪」这类二元状态的统一写法，顺便把 unknown 也说清楚。 */
export function StatusPill({ ok, okText = "已就绪", badText = "未就绪" }: { ok: boolean | null; okText?: string; badText?: string }) {
  if (ok === null) return <Pill tone="warn">待检测</Pill>;
  return ok ? <Pill tone="ok">{okText}</Pill> : <Pill tone="warn">{badText}</Pill>;
}

/* ---------------- 按钮 ---------------- */

export type ButtonVariant = "primary" | "ghost" | "soft" | "danger" | "quiet" | "dashed";

export function Button({
  variant = "ghost",
  onClick,
  disabled,
  title,
  ariaLabel,
  children,
}: {
  variant?: ButtonVariant;
  onClick?: () => void;
  disabled?: boolean;
  title?: string;
  /** 按钮文字只是「下载」这类通用词时，用它把对象说清楚，读屏软件才分得清。 */
  ariaLabel?: string;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      className={`em-btn ${variant}`}
      onClick={onClick}
      disabled={disabled}
      title={title ?? ariaLabel}
      aria-label={ariaLabel}
    >
      {children}
    </button>
  );
}

/* ---------------- 分段控件 ---------------- */

export function Segmented<T extends string>({
  value,
  options,
  onChange,
  label,
}: {
  value: T;
  options: { value: T; label: string; disabled?: boolean; hint?: string }[];
  onChange: (value: T) => void;
  label: string;
}) {
  return (
    <div className="em-seg" role="group" aria-label={label}>
      {options.map((option) => (
        <button
          key={option.value}
          type="button"
          className="em-seg-item"
          aria-pressed={value === option.value}
          disabled={option.disabled}
          title={option.hint}
          onClick={() => onChange(option.value)}
        >
          {option.label}
        </button>
      ))}
    </div>
  );
}

/* ---------------- 列表行（列表即选择器） ---------------- */

export type RowState = "idle" | "done" | "working" | "error";

const ROW_GLYPH: Record<RowState, ReactNode> = { idle: <CircleIcon size={11} />, done: <CheckIcon size={11} />, working: "", error: <TriangleAlertIcon size={11} /> };

export function ListRow({
  state = "idle",
  title,
  meta,
  badge,
  trail,
  selected,
  onSelect,
  static: isStatic,
  titleAttr,
}: {
  state?: RowState;
  title: ReactNode;
  meta?: ReactNode;
  badge?: ReactNode;
  trail?: ReactNode;
  selected?: boolean;
  onSelect?: () => void;
  static?: boolean;
  titleAttr?: string;
}) {
  const className = `em-row${isStatic || !onSelect ? " static" : ""}`;
  const body = (
    <>
      <span className={`em-row-glyph ${state}`} aria-hidden="true">
        {ROW_GLYPH[state]}
      </span>
      <span className="em-row-main">
        <span className="em-row-title">
          <strong>{title}</strong>
          {badge}
        </span>
        {meta && <span className="em-row-meta">{meta}</span>}
      </span>
      {trail && <span className="em-row-trail">{trail}</span>}
    </>
  );
  if (!onSelect) {
    return (
      <div className={className} title={titleAttr}>
        {body}
      </div>
    );
  }
  return (
    <button
      type="button"
      className={className}
      aria-pressed={Boolean(selected)}
      onClick={onSelect}
      title={titleAttr}
    >
      {body}
    </button>
  );
}

export function List({ children }: { children: ReactNode }) {
  return <div className="em-list">{children}</div>;
}

/* ---------------- 进度 ---------------- */

export function Progress({
  completed,
  total,
  label,
  state = "running",
}: {
  completed: number | null;
  total: number | null;
  label: string;
  state?: "running" | "failed";
}) {
  const percent = completed !== null && total ? Math.min(100, Math.round((completed / total) * 100)) : null;
  const indeterminate = percent === null;
  return (
    <div className={`em-progress${indeterminate ? " indeterminate" : ""}${state === "failed" ? " failed" : ""}`} role="status">
      <div className="em-progress-head">
        <span>{label}</span>
        <span>{percent !== null ? `${percent}%` : ""}</span>
      </div>
      <div className="em-progress-track">
        <span className="em-progress-fill" style={{ width: percent !== null ? `${percent}%` : undefined }} />
      </div>
    </div>
  );
}

/* ---------------- 表单字段 ---------------- */

export function Field({
  label,
  hint,
  children,
}: {
  label: string;
  hint?: ReactNode;
  children: ReactNode;
}) {
  return (
    <label className="em-field">
      <span className="em-field-label">{label}</span>
      <span className="em-field-control">{children}</span>
      {hint && <span className="em-field-hint">{hint}</span>}
    </label>
  );
}

export function ToggleRow({
  label,
  checked,
  onChange,
  disabled,
}: {
  label: ReactNode;
  checked: boolean;
  onChange: (next: boolean) => void;
  disabled?: boolean;
}) {
  return (
    <label className="em-toggle-row">
      <span>{label}</span>
      <span className="switch">
        <input type="checkbox" checked={checked} disabled={disabled} onChange={(event) => onChange(event.target.checked)} />
        <span />
      </span>
    </label>
  );
}

export function Collapsible({
  title,
  children,
  open,
  onToggle,
}: {
  title: string;
  children: ReactNode;
  open?: boolean;
  onToggle?: (open: boolean) => void;
}) {
  return (
    <details className="em-collapsible" open={open} onToggle={(event) => onToggle?.((event.currentTarget as HTMLDetailsElement).open)}>
      <summary>{title}</summary>
      <div className="em-collapsible-body">{children}</div>
    </details>
  );
}

/* ---------------- 保存提示 ---------------- */

export type SavedState = { tone: "saved" | "failed"; message: string } | null;

/**
 * 「改了就生效」的轻提示。设置面板不再有「保存设置」按钮，
 * 每次写入用这里把结果告诉用户：成功飘一句，失败留在原地。
 */
export function useSavedFlash(timeoutMs = 2200) {
  const [flash, setFlash] = useState<SavedState>(null);
  const timer = useRef<number | null>(null);

  useEffect(() => () => {
    if (timer.current !== null) window.clearTimeout(timer.current);
  }, []);

  const show = useCallback(
    (tone: "saved" | "failed", message: string) => {
      if (timer.current !== null) window.clearTimeout(timer.current);
      setFlash({ tone, message });
      if (tone === "saved") {
        timer.current = window.setTimeout(() => setFlash(null), timeoutMs);
      }
    },
    [timeoutMs],
  );

  return { flash, show };
}

export function SavedToast({ flash }: { flash: SavedState }) {
  if (!flash) return null;
  return (
    <span className={`em-saved${flash.tone === "failed" ? " failed" : ""}`} role="status">
      {flash.message}
    </span>
  );
}

/**
 * 文本输入的「改了就存」。
 *
 * delayMs 传数字时停止输入若干毫秒后落盘；传 null 时只在失焦时落盘 ——
 * 接口地址这类字段适合后者：敲到一半的地址既不该写库，也不该被当成用户的最终意图。
 * 焦点变化前用户还会继续编辑，所以两种模式都不回写覆盖正在输入的内容。
 */
export function useDraftSave<T extends Record<string, unknown>>(
  remote: T,
  save: (draft: T) => void,
  delayMs: number | null = 500,
) {
  const [draft, setDraft] = useState<T>(remote);
  const dirty = useRef(false);
  const saveRef = useRef(save);
  saveRef.current = save;
  // 用序列化后的值做依赖：每次渲染传进来的新对象引用不会触发同步，值变了才同步。
  const remoteKey = JSON.stringify(remote);

  useEffect(() => {
    if (!dirty.current) setDraft(JSON.parse(remoteKey) as T);
  }, [remoteKey]);

  useEffect(() => {
    if (delayMs === null || !dirty.current) return;
    const timer = window.setTimeout(() => {
      dirty.current = false;
      saveRef.current(draft);
    }, delayMs);
    return () => window.clearTimeout(timer);
  }, [draft, delayMs]);

  const update = useCallback((patch: Partial<T>) => {
    dirty.current = true;
    setDraft((current) => ({ ...current, ...patch }));
  }, []);

  const flush = useCallback(() => {
    if (!dirty.current) return;
    dirty.current = false;
    saveRef.current(draft);
  }, [draft]);

  return { draft, update, flush };
}
