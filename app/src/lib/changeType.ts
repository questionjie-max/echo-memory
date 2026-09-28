/**
 * 认知演化变化类型的归一化层。
 *
 * 线上（memory.rs 的 schema_contract）写死的是英文枚举，存储与传输都不做转换；
 * 显示层是唯一可以翻译的地方——后端提示词要求中文时，新旧快照仍会混着英文值，
 * 所以这里对已知值映射中文、对未知值兜底，而不是信任数据永远干净。
 */
export const EVOLUTION_CHANGE_TYPES = [
  "added",
  "supplemented",
  "revised",
  "overturned",
  "merged",
  "validated",
] as const;

const CHANGE_TYPE_LABELS: Record<(typeof EVOLUTION_CHANGE_TYPES)[number], string> = {
  added: "新增",
  supplemented: "补充",
  revised: "修正",
  overturned: "推翻",
  merged: "合并",
  validated: "验证",
};

/** 已知值给中文标签，未知值（含模型自创词）兜底为「变化」——不把原始值直接抛给界面。 */
export function changeTypeLabel(raw: string): string {
  return CHANGE_TYPE_LABELS[raw.trim().toLowerCase() as (typeof EVOLUTION_CHANGE_TYPES)[number]] ?? "变化";
}

/** 已知值原文作 CSS class（change-added 等），未知值统一走 change-other。 */
export function changeTypeSlug(raw: string): string {
  return EVOLUTION_CHANGE_TYPES.includes(raw.trim().toLowerCase() as (typeof EVOLUTION_CHANGE_TYPES)[number])
    ? raw.trim().toLowerCase()
    : "other";
}
