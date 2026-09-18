/**
 * 主题：深色（Lovart 方案）为默认，浅色保留。
 *
 * 生效机制分两层：
 * - index.html 的首帧内联脚本在 CSS 加载前把 data-theme 写到 <html>，
 *   避免深色用户启动时看到一帧白底；
 * - 这里负责运行时读取/切换/持久化，两处取值必须一致（em.theme）。
 */

export type Theme = "dark" | "light";

const STORAGE_KEY = "em.theme";
const DEFAULT_THEME: Theme = "dark";

export function getStoredTheme(): Theme {
  try {
    const stored = window.localStorage.getItem(STORAGE_KEY);
    if (stored === "light" || stored === "dark") return stored;
  } catch {
    /* localStorage 不可用（隐私模式等），走默认 */
  }
  return DEFAULT_THEME;
}

export function applyTheme(theme: Theme): void {
  document.documentElement.setAttribute("data-theme", theme);
}

export function setTheme(theme: Theme): void {
  applyTheme(theme);
  try {
    window.localStorage.setItem(STORAGE_KEY, theme);
  } catch {
    /* 写不进去就只在本会话内生效 */
  }
}

/** 初始化：把存储里（或默认）的主题挂上。main.tsx 启动时调用一次。 */
export function initTheme(): Theme {
  const theme = getStoredTheme();
  applyTheme(theme);
  return theme;
}
