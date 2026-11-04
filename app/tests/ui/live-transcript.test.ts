/**
 * 实时文字前端合并逻辑的纯函数测试。
 *
 * 盯着块2 的验收承诺：不重复、不乱序、暂定尾整段替换、接续合并后的尾句
 * 按 startMs 同键替换。
 */
import { describe, expect, it } from "vitest";
import {
  applyLiveTick,
  emptyLiveView,
  formatSegmentTime,
} from "../../src/lib/liveTranscript";
import type { LiveTickPayload } from "../../src/shared/types";

function tickPayload(overrides: Partial<LiveTickPayload> = {}): LiveTickPayload {
  return {
    sessionId: "session-1",
    source: "microphone",
    newFinal: [],
    provisionalTail: [],
    committedMs: 0,
    capturedMs: 0,
    sentAtUnixMs: 0,
    ...overrides,
  };
}

describe("实时文字合并", () => {
  it("新定稿追加，暂定尾整段替换，不残留上一拍的压线句", () => {
    let view = emptyLiveView();
    view = applyLiveTick(
      view,
      tickPayload({
        newFinal: [{ startMs: 100, endMs: 900, text: "开场" }],
        provisionalTail: [{ startMs: 1200, endMs: 2400, text: "还在说" }],
      }),
    );
    expect(view.finals).toHaveLength(1);
    expect(view.provisional).toEqual([{ startMs: 1200, endMs: 2400, text: "还在说" }]);

    // 下一拍：压线句转正，暂定区换成了新的压线句——旧的暂定不得残留。
    view = applyLiveTick(
      view,
      tickPayload({
        newFinal: [{ startMs: 1200, endMs: 2400, text: "还在说话" }],
        provisionalTail: [{ startMs: 3000, endMs: 4100, text: "新的压线" }],
      }),
    );
    expect(view.finals).toHaveLength(2);
    expect(view.finals[1].text).toBe("还在说话");
    expect(view.provisional).toHaveLength(1);
    expect(view.provisional[0].text).toBe("新的压线");
  });

  it("接续合并后的尾句按 startMs 同键替换，不产生重复行", () => {
    let view = emptyLiveView();
    view = applyLiveTick(
      view,
      tickPayload({
        newFinal: [{ startMs: 3980, endMs: 6000, text: "我们把第三季度的产品" }],
      }),
    );
    // 后端接续合并：尾句变完整并重发（startMs 不变）。
    view = applyLiveTick(
      view,
      tickPayload({
        newFinal: [
          {
            startMs: 3980,
            endMs: 9000,
            text: "我们把第三季度的产品复盘安排在下周三下午2点",
          },
        ],
      }),
    );
    expect(view.finals).toHaveLength(1);
    expect(view.finals[0].text).toBe("我们把第三季度的产品复盘安排在下周三下午2点");
  });

  it("防御性按时间排序，乱序到达也不乱序展示；空文本不进列表", () => {
    let view = emptyLiveView();
    view = applyLiveTick(
      view,
      tickPayload({
        newFinal: [
          { startMs: 5000, endMs: 6000, text: "后到的前句" },
          { startMs: 1000, endMs: 2000, text: "先说的后到" },
          { startMs: 7000, endMs: 8000, text: "   " },
        ],
      }),
    );
    expect(view.finals).toHaveLength(2);
    expect(view.finals[0].startMs).toBe(1_000);
    expect(view.finals[1].startMs).toBe(5_000);
  });

  it("时间格式：mm:ss，超过一小时进 h:mm:ss", () => {
    expect(formatSegmentTime(0)).toBe("00:00");
    expect(formatSegmentTime(65_000)).toBe("01:05");
    expect(formatSegmentTime(3_723_000)).toBe("1:02:03");
    expect(formatSegmentTime(-5)).toBe("00:00");
  });
});
