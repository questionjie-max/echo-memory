/**
 * 原生确认与提示对话框。
 *
 * 为什么不用 window.confirm / window.alert：打包后的 WKWebView 里它们不弹任何界面，
 * confirm 直接返回 true、alert 直接丢弃——结果是「删除」这类破坏性操作不询问就执行，
 * 错误提示静默消失。两者都必须走 dialog 插件的原生对话框（权限见
 * src-tauri/capabilities/default.json 的 dialog:allow-confirm）。
 *
 * 非 Tauri 环境（Vitest / 浏览器预览）退回 window.*，测试里的 confirm 打桩照旧生效。
 */
import { isTauri } from "@tauri-apps/api/core";
import { confirm as nativeConfirm, message as nativeMessage } from "@tauri-apps/plugin-dialog";

const DIALOG_TITLE = "回声记忆";
/* 按钮文案显式给中文：插件默认用系统语言（英文 Cancel/OK），
   在纯中文界面里就是混排，这里统一成 确定 / 取消。 */
const CONFIRM_OPTIONS = {
  title: DIALOG_TITLE,
  kind: "warning",
  okLabel: "确定",
  cancelLabel: "取消",
} as const;
const MESSAGE_OPTIONS = {
  title: DIALOG_TITLE,
  kind: "error",
  okLabel: "确定",
} as const;

export async function confirmAction(text: string): Promise<boolean> {
  if (isTauri()) {
    try {
      return await nativeConfirm(text, CONFIRM_OPTIONS);
    } catch {
      // 插件不可用时退回浏览器确认：宁可弹两次，也不能静默放行破坏性操作。
      return window.confirm(text);
    }
  }
  return window.confirm(text);
}

export async function notify(text: string): Promise<void> {
  if (isTauri()) {
    try {
      await nativeMessage(text, MESSAGE_OPTIONS);
      return;
    } catch {
      // 落回浏览器 alert；再不行就保持静默（提示失败不应再抛错）。
    }
  }
  window.alert(text);
}
