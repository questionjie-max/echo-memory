import type { DiagnosisReport, SuggestedAction } from "../shared/types";

/** 建议动作 → 中文按钮/提示文案。 */
export function suggestedActionText(action: SuggestedAction): string {
  switch (action) {
    case "refill_key":
      return "重新填写 Key，或到服务商控制台检查";
    case "switch_model":
      return "换一个文本对话模型再测试";
    case "check_account":
      return "到服务商控制台检查账号开通、余额与权限";
    case "check_network":
      return "检查网络、代理或接口地址后重试";
    case "retry":
      return "稍等片刻再重试，必要时检查账户额度";
    case "change_model_or_permission":
      return "换一个文本对话模型；若仍失败，到服务商控制台检查账号权限";
  }
}

/** 诊断阶段 → 「发生了什么」的一句话。 */
function stageText(report: DiagnosisReport): string {
  switch (report.stage) {
    case "catalog":
      return "无法读取服务商模型目录";
    case "network":
      return "无法连接到服务商";
    case "model":
      return "当前模型不可用（不存在、已下线或账号无权使用）";
    case "protocol":
      return "当前模型不支持这项能力的接口或参数";
    case "quota":
      return "触发服务商频率限制或额度不足";
    case "inference":
      return "真实推理请求失败";
    case "response":
      return "服务商返回 200 但没有模型输出内容";
    case "format":
      return "连接可用，但模型输出不满足结构化分析要求";
    case "ok":
      return "验证通过";
  }
}

/** 诊断阶段 → 「最可能原因」。 */
function causeText(report: DiagnosisReport): string | null {
  switch (report.stage) {
    case "catalog":
      return report.httpStatus === 401 || report.httpStatus === 403
        ? "Key 无效或没有模型目录权限"
        : "服务商接口或账号状态异常";
    case "network":
      return "网络不通、代理拦截或接口地址写错";
    case "model":
      return report.errorCode
        ? `服务商返回错误码：${report.errorCode}`
        : "推荐模型可能已变更，或账号未开通该模型";
    case "protocol":
      return "该模型不是文本对话模型，或参数不被支持";
    case "quota":
      return "请求过于频繁，或账户余额/额度不足";
    case "inference":
      return "无法确认是 Key 还是模型的问题：请先换模型重试";
    case "response":
      return "服务商响应格式异常";
    case "format":
      return "该模型不适合结构化 JSON 输出，换更适合分析的模型";
    case "ok":
      return null;
  }
}

/** 诊断报告 → 错误展示的四段式（what / cause / action / 技术详情）。 */
export function describeDiagnosis(report: DiagnosisReport): {
  what: string;
  cause: string | null;
  action: string;
  details: string | null;
} {
  const detailsParts: string[] = [];
  detailsParts.push(`阶段：${report.stage}`);
  if (report.httpStatus != null) detailsParts.push(`HTTP ${report.httpStatus}`);
  if (report.errorCode) detailsParts.push(`错误码：${report.errorCode}`);
  detailsParts.push(`模型：${report.model}`);
  if (report.providerMessage) detailsParts.push(`服务商返回：${report.providerMessage}`);
  return {
    what: stageText(report),
    cause: causeText(report),
    action: suggestedActionText(report.suggestedAction),
    details: detailsParts.join("\n"),
  };
}
