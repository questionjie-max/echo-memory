import { describe, expect, it } from "vitest";
import { estimateRemainingMs, rememberProgress } from "../../src/lib/progress-estimate";

/** v1.4 包4：长任务预计剩余时间。
 *
 * processing-progress 事件只带 recordId，前端靠 load() 拉到的
 * progressCurrent/progressTotal 自己记时：两次事件的时间差 ÷ 块数差 =
 * 平均块耗时，剩余块数 × 平均耗时 = 预计剩余。
 */
describe("预计剩余时间（v1.4 包4）", () => {
  it("两个进度点即可估算：平均块耗时 × 剩余块数", () => {
    const history = [
      rememberProgress(0, 0),
      rememberProgress(1, 60_000), // 1 分钟/块
    ];
    // 共 12 块，已完成到索引 1，剩 10 块 × 60s = 600s
    expect(estimateRemainingMs(history, 1, 12)).toBe(600_000);
  });

  it("多点取整体平均，随进度收敛", () => {
    const history = [
      rememberProgress(0, 0),
      rememberProgress(2, 120_000),
      rememberProgress(4, 260_000), // 前两块各 60s，后两块各 70s
    ];
    // 平均 65s/块；共 12 块已完成到索引 4，剩 7 块
    expect(estimateRemainingMs(history, 4, 12)).toBe(455_000);
  });

  it("历史不足两个点或无进展时不给估计（返回 null，避免瞎猜）", () => {
    expect(estimateRemainingMs([], 0, 12)).toBeNull();
    expect(estimateRemainingMs([rememberProgress(3, 1000)], 3, 12)).toBeNull();
    // 块数没变（重复事件）不产生估计
    const same = [rememberProgress(3, 0), rememberProgress(3, 5000)];
    expect(estimateRemainingMs(same, 3, 12)).toBeNull();
  });

  it("进度回退（重试）时只用单调递增的点，剩余按最后推进点之后算", () => {
    const history = [
      rememberProgress(2, 0),
      rememberProgress(5, 90_000), // 30s/块
      rememberProgress(3, 95_000), // 回退点（后端重试）
    ];
    // 回退意味着块 4、5 会重跑：剩余按有效历史最后推进点（5）之后算 → 6 块 × 30s
    expect(estimateRemainingMs(history, 3, 12)).toBe(180_000);
  });

  it("rememberProgress 过滤掉没有推进的点（不进历史，防止稀释均值）", () => {
    const history: Array<{ at: number; current: number }> = [];
    rememberProgress(0, 0, history);
    rememberProgress(0, 3000, history); // 没推进：不进
    rememberProgress(1, 6000, history);
    expect(history).toHaveLength(2);
  });

  it("全部完成时剩余为 0", () => {
    const history = [
      rememberProgress(0, 0),
      rememberProgress(11, 660_000),
    ];
    expect(estimateRemainingMs(history, 11, 12)).toBe(0);
  });
});
