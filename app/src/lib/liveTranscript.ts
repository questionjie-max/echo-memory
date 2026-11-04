/**
 * 实时文字（流式转写预览）的前端合并逻辑。
 *
 * 后端每拍推送一个 tick：newFinal 是新定稿（或被接续合并改写后重发的尾句），
 * provisionalTail 是压线句子（下一拍可能变）。定稿按 startMs 作为同键替换依据：
 * 后端接续合并会改写尾句（startMs 不变），同键到达时替换而不是追加。
 * 这里刻意做成纯函数：合并规则出问题（重复、丢句、乱序）都在单测里抓。
 */
import type { LiveSegment, LiveTickPayload } from "../shared/types";

export interface LiveTranscriptView {
  finals: LiveSegment[];
  provisional: LiveSegment[];
}

export function emptyLiveView(): LiveTranscriptView {
  return { finals: [], provisional: [] };
}

export function applyLiveTick(
  view: LiveTranscriptView,
  payload: LiveTickPayload,
): LiveTranscriptView {
  const finals = [...view.finals];
  for (const segment of payload.newFinal) {
    if (segment.text.trim() === "") continue;
    const index = finals.findIndex((existing) => existing.startMs === segment.startMs);
    if (index >= 0) {
      finals[index] = segment;
    } else {
      finals.push(segment);
    }
  }
  // 定稿列表按时间排序展示（后端保证基本有序，防御性排序不改变正确性）。
  finals.sort((left, right) => left.startMs - right.startMs);
  // 暂定尾与定稿对称过滤空文本：后端已过滤，这里是纯函数自洽防线。
  const provisional = payload.provisionalTail.filter((segment) => segment.text.trim() !== "");
  return { finals, provisional };
}

/** mm:ss 会话时间（与逐字稿面板口径一致，超过一小时进 h:mm:ss）。 */
export function formatSegmentTime(ms: number): string {
  if (!Number.isFinite(ms) || ms < 0) return "00:00";
  const totalSeconds = Math.floor(ms / 1_000);
  const hours = Math.floor(totalSeconds / 3_600);
  const minutes = Math.floor((totalSeconds % 3_600) / 60);
  const seconds = totalSeconds % 60;
  const pad = (value: number) => String(value).padStart(2, "0");
  return hours > 0
    ? `${hours}:${pad(minutes)}:${pad(seconds)}`
    : `${pad(minutes)}:${pad(seconds)}`;
}
