import { describe, expect, it } from "vitest";
import {
  deriveVerifyState,
  healthFingerprintMatches,
  VERIFY_STATE_LABELS,
  verifiedDescription,
  failedDescription,
} from "../../src/lib/verify-state";
import { healthSnapshotFixture } from "./settings-mocks";
import type { VerifyStatusInput } from "../../src/lib/verify-state";

/** 与 input() 指纹对齐的快照：dashscope / qwen-turbo / revision 1。 */
function snap(overrides: Partial<Parameters<typeof healthSnapshotFixture>[0]> = {}) {
  return healthSnapshotFixture({
    providerId: "dashscope",
    baseUrl: "https://dashscope.aliyuncs.com/compatible-mode/v1",
    model: "qwen-turbo",
    ...overrides,
  });
}

function input(overrides: Partial<VerifyStatusInput> = {}): VerifyStatusInput {
  return {
    hasCredentials: true,
    providerId: "dashscope",
    baseUrl: "https://dashscope.aliyuncs.com/compatible-mode/v1",
    model: "qwen-turbo",
    health: snap(),
    credentialRevision: 1,
    catalogVersion: 1,
    draftDeviation: false,
    busy: false,
    ...overrides,
  };
}

describe("六态推导：优先级链", () => {
  it("无凭据或未选服务商 = 未配置（优先级最高，快照屏蔽）", () => {
    expect(deriveVerifyState(input({ hasCredentials: false }))).toBe("unconfigured");
    expect(deriveVerifyState(input({ providerId: "none" }))).toBe("unconfigured");
    expect(deriveVerifyState(input({ providerId: "" }))).toBe("unconfigured");
  });

  it("请求进行中 = 正在验证（覆盖一切快照态）", () => {
    expect(deriveVerifyState(input({ busy: true }))).toBe("verifying");
    expect(deriveVerifyState(input({ busy: true, draftDeviation: true }))).toBe("verifying");
  });

  it("草稿偏离已保存配置 = 需重新验证（优先于快照的通过/失败结论）", () => {
    expect(deriveVerifyState(input({ draftDeviation: true }))).toBe("stale");
    // 失败快照不跟随草稿显示：失败结论只对产生它的配置指纹负责。
    expect(
      deriveVerifyState(input({ draftDeviation: true, health: snap({ status: "failed" }) })),
    ).toBe("stale");
  });

  it("配置齐全但从未验证 = 已配置，未验证", () => {
    expect(deriveVerifyState(input({ health: null }))).toBe("unverified");
  });

  it("快照指纹全匹配且通过 = 已验证", () => {
    expect(deriveVerifyState(input())).toBe("verified");
  });

  it("快照指纹全匹配且失败 = 验证失败", () => {
    expect(deriveVerifyState(input({ health: snap({ status: "failed" }) }))).toBe("failed");
  });

  it("任一指纹字段变化 = 需重新验证", () => {
    for (const broken of [
      input({ credentialRevision: 2 }), // 凭据改过
      input({ model: "qwen-plus" }), // 模型换了
      input({ providerId: "zhipu" }), // 服务商换了
      input({ catalogVersion: 2 }), // 目录升级
    ]) {
      expect(deriveVerifyState(broken)).toBe("stale");
    }
  });

  it("baseUrl 尾斜杠不破坏指纹匹配", () => {
    expect(
      healthFingerprintMatches(snap(), {
        providerId: "dashscope",
        baseUrl: "https://dashscope.aliyuncs.com/compatible-mode/v1",
        model: "qwen-turbo",
        credentialRevision: 1,
        catalogVersion: 1,
      }),
    ).toBe(true);
    expect(
      healthFingerprintMatches(snap(), {
        providerId: "dashscope",
        baseUrl: "https://dashscope.aliyuncs.com/compatible-mode/v1/",
        model: "qwen-turbo",
        credentialRevision: 1,
        catalogVersion: 1,
      }),
    ).toBe(true);
  });

  it("六个状态都有中文文案，且不得出现「已连接」这类过度承诺", () => {
    const labels = Object.values(VERIFY_STATE_LABELS);
    expect(labels).toHaveLength(6);
    for (const label of labels) {
      expect(label.length).toBeGreaterThan(0);
      expect(label).not.toContain("已连接");
    }
  });

  it("已验证描述带模型与时间；失败描述引导重新验证；其余为空", () => {
    expect(verifiedDescription(snap())).toContain("qwen-turbo");
    expect(failedDescription(snap({ status: "failed" }))).toContain("重新验证");
    expect(verifiedDescription(snap({ status: "failed" }))).toBeNull();
    expect(verifiedDescription(null)).toBeNull();
  });
});
