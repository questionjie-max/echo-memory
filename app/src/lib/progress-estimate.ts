/**
 * v1.4 包4：长任务预计剩余时间。
 *
 * processing-progress 事件只带 recordId，前端靠 load() 拉到的
 * progressCurrent/progressTotal 自己记时：有效历史的时间差 ÷ 块数差 =
 * 平均块耗时，剩余块数 × 平均耗时 = 预计剩余。
 */

export interface ProgressPoint {
  /** 本机单调时钟或 Date.now()，毫秒。 */
  at: number;
  /** 已完成的块索引（progressCurrent）。 */
  current: number;
}

/**
 * 记录一个进度点。传入 history 时只记录**有推进**的点（current 严格大于
 * 上一个点），重复事件/回退事件不进历史，防止稀释平均块耗时。
 * 不传 history 时仅返回点本身。
 */
export function rememberProgress(
  current: number,
  at: number,
  history?: ProgressPoint[],
): ProgressPoint {
  const point = { at, current };
  if (history) {
    const last = history[history.length - 1];
    if (!last || current > last.current) history.push(point);
  }
  return point;
}

/**
 * 估算剩余毫秒。规则：
 * - 历史中只有单调递增的点参与计算（回退点丢弃——重试场景下已跑过的块
 *   证明可达，剩余按有效历史最后推进点之后计算）；
 * - 有效点不足两个、无进展、无有效耗时（total≤1）时返回 null，不瞎猜；
 * - 全部完成时返回 0。
 */
export function estimateRemainingMs(
  history: ProgressPoint[],
  current: number,
  total: number,
): number | null {
  if (total <= 1) return null;
  const valid = history.filter(
    (point, index) => index === 0 || point.current > history[index - 1].current,
  );
  if (valid.length < 2) return null;
  const first = valid[0];
  const last = valid[valid.length - 1];
  if (last.current <= first.current) return null;
  const perBlockMs = (last.at - first.at) / (last.current - first.current);
  if (!Number.isFinite(perBlockMs) || perBlockMs <= 0) return null;
  const doneThrough = Math.max(current, last.current);
  const remainingBlocks = Math.max(0, total - 1 - doneThrough);
  return Math.round(perBlockMs * remainingBlocks);
}
