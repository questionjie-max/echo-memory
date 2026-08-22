/**
 * 跨组件共享的展示格式化工具。
 * 时间格式统一为 `分:秒`（如 3:07）；超过 1 小时由调用方自行拼装日期文案。
 */
import type { RecordStatus } from "../shared/types";

/**
 * 将毫秒格式化为 `分:秒`。
 * @param round 为 true 时秒数四舍五入（录音时长展示），默认向下取整（时间点定位）。
 */
export function formatMinutesSeconds(ms: number, round = false): string {
  const totalSeconds = round ? Math.round(ms / 1000) : Math.floor(ms / 1000);
  return `${Math.floor(totalSeconds / 60)}:${String(totalSeconds % 60).padStart(2, "0")}`;
}

/** 记录是否处于本机处理中（准备/转写/分析），用于轮询与状态徽标。 */
export function isProcessingStatus(status: RecordStatus): boolean {
  return status === "preparing" || status === "transcribing" || status === "analyzing";
}
