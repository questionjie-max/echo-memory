import type { ProviderHealthSnapshot } from "../shared/types";

/** 第三方连接的六个状态（与后端健康快照/凭据修订号/目录版本共同推导）。 */
export type VerifyState =
  | "unconfigured"
  | "unverified"
  | "verifying"
  | "verified"
  | "stale"
  | "failed";

export interface VerifyStatusInput {
  /** 当前槽位凭据是否填齐（settings.hasApiKey / transcriptionHasApiKey）。 */
  hasCredentials: boolean;
  /** 已保存的服务商（空或 none = 未配置）。 */
  providerId: string;
  baseUrl: string;
  model: string;
  /** 健康快照（null = 从未验证过）。 */
  health: ProviderHealthSnapshot | null;
  credentialRevision: number;
  catalogVersion: number;
  /**
   * 草稿偏离已保存配置：provider/baseUrl/model 任一被编辑，或凭据字段
   * 被动过。切回原值且凭据未动时恢复 false（指纹匹配即恢复显示快照态）。
   */
  draftDeviation: boolean;
  /** 诊断/保存进行中（唯一由运行时标志决定的态）。 */
  busy: boolean;
}

/**
 * 六态推导（优先级自上而下命中即停）：
 *   unconfigured > verifying > 草稿偏离(stale) > 无快照(unverified)
 *   > 指纹匹配 ? (passed ? verified : failed) : stale
 * 失败结论只对产生它的配置指纹负责：草稿偏离时不显示旧失败原因。
 */
export function deriveVerifyState(input: VerifyStatusInput): VerifyState {
  if (!input.hasCredentials || !input.providerId || input.providerId === "none") {
    return "unconfigured";
  }
  if (input.busy) {
    return "verifying";
  }
  if (input.draftDeviation) {
    return "stale";
  }
  if (!input.health) {
    return "unverified";
  }
  if (!healthFingerprintMatches(input.health, input)) {
    return "stale";
  }
  return input.health.status === "passed" ? "verified" : "failed";
}

/** 指纹比对：provider/baseUrl/model/凭据修订号/目录版本全部一致。 */
export function healthFingerprintMatches(
  health: ProviderHealthSnapshot,
  input: Pick<VerifyStatusInput, "providerId" | "baseUrl" | "model" | "credentialRevision" | "catalogVersion">,
): boolean {
  const normalize = (value: string) => value.trim().replace(/\/+$/, "");
  return (
    health.providerId === input.providerId &&
    normalize(health.baseUrl) === normalize(input.baseUrl) &&
    health.model === input.model &&
    health.credentialRevision === input.credentialRevision &&
    health.catalogVersion === input.catalogVersion
  );
}

/** 状态条的中文文案（卡片头药丸与表单状态条共用同一词表）。 */
export const VERIFY_STATE_LABELS: Record<VerifyState, string> = {
  unconfigured: "待配置",
  unverified: "已配置，未验证",
  verifying: "正在验证",
  verified: "已验证",
  stale: "配置已变更，需重新验证",
  failed: "验证失败",
};

/** 验证成功时在状态条附带的描述：模型与验证时间。 */
export function verifiedDescription(health: ProviderHealthSnapshot | null): string | null {
  if (!health || health.status !== "passed") return null;
  const time = new Date(health.verifiedAt);
  const shown = Number.isNaN(time.getTime())
    ? health.verifiedAt
    : time.toLocaleString("zh-CN", { month: "numeric", day: "numeric", hour: "2-digit", minute: "2-digit" });
  return `${health.model} · ${shown}`;
}

/** 验证失败时在状态条附带的描述（不重复技术详情）。 */
export function failedDescription(health: ProviderHealthSnapshot | null): string | null {
  if (!health || health.status !== "failed") return null;
  return "上次验证未通过，请重新验证或修改配置";
}
