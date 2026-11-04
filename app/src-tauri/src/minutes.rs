//! 会议纪要（v1.1 块3/块4/块5）。
//!
//! 事后结构化纪要：议题、结论、待办、术语表。每条结论/待办必须带可验证的
//! 逐字证据（回链到逐字稿时间窗），验证不过的条目整体丢弃（fail-closed，
//! 与分析链路「引用对不上直接丢弃」同一底线）。生成失败不阻塞录音入库：
//! minutes 行状态独立，failed 可重试。
//!
//! 确认红线（块4）：条目两态 provisional/confirmed 存 minutes_items 表，
//! 确认是显式命令；MCP 侧只读 confirmed 条目，未确认的对 MCP 完全不可见。
//! 重新生成会整体重建条目并回到 provisional——确认必须针对当下这份内容。

use crate::error::{AppError, AppResult};
use crate::library::ManagedLibrary;
use crate::types::TranscriptSegment;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MinutesStatus {
    Pending,
    Generating,
    Completed,
    Failed,
}

impl MinutesStatus {
    fn from_db(value: &str) -> Self {
        match value {
            "generating" => Self::Generating,
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            _ => Self::Pending,
        }
    }

    fn as_db(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Generating => "generating",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MinutesItemKind {
    Conclusion,
    Action,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MinutesItemStatus {
    Provisional,
    Confirmed,
}

/// 结论/待办的证据：必须能对回逐字稿的时间窗与原话。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MinutesEvidence {
    pub start_ms: i64,
    pub end_ms: i64,
    pub quote_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MinutesItem {
    pub id: String,
    pub kind: MinutesItemKind,
    pub text: String,
    pub owner: Option<String>,
    pub evidence: MinutesEvidence,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MinutesTopic {
    pub id: String,
    pub title: String,
    pub summary: String,
    pub start_ms: Option<i64>,
    pub end_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GlossaryEntry {
    pub term: String,
    pub variants: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MinutesDocument {
    pub topics: Vec<MinutesTopic>,
    pub conclusions: Vec<MinutesItem>,
    pub action_items: Vec<MinutesItem>,
    pub glossary: Vec<GlossaryEntry>,
    /// 生成时因证据无法验证而被丢弃的条目数（fail-closed 的透明度）。
    pub dropped_unverifiable: usize,
}

impl Default for MinutesDocument {
    fn default() -> Self {
        Self {
            topics: Vec::new(),
            conclusions: Vec::new(),
            action_items: Vec::new(),
            glossary: Vec::new(),
            dropped_unverifiable: 0,
        }
    }
}

/// 纪要视图（命令返回给前端的聚合体）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MinutesView {
    pub record_id: String,
    pub status: MinutesStatus,
    pub document: Option<MinutesDocument>,
    /// 条目两态（块4）：item_id → status。重新生成后全部回到 provisional。
    pub item_statuses: Vec<(String, MinutesItemStatus)>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub error: Option<String>,
    pub generated_at: Option<String>,
}

/* ---------------- 纯逻辑：提示词、解析、证据校验（测试重点） ---------------- */

/// generating 状态的无心跳阈值：超过视为崩溃残留，可被恢复/抢占。
pub const MINUTES_GENERATION_STALE_SECS: i64 = 600;
/// 长逐字稿分块上限（字符）：对齐 Ollama 通道 num_ctx 的安全余量，
/// 防止长会议静默截断出「半场纪要」。
pub const MINUTES_CHUNK_CHARS: usize = 6_000;

/// 按段累计切分逐字稿（软上限）：单段超长时独立成块。
pub fn chunk_segments(segments: &[TranscriptSegment]) -> Vec<&[TranscriptSegment]> {
    let mut chunks = Vec::new();
    let mut start = 0usize;
    let mut chars = 0usize;
    for (index, segment) in segments.iter().enumerate() {
        chars += effective_text(segment).chars().count();
        if chars >= MINUTES_CHUNK_CHARS && index > start {
            chunks.push(&segments[start..index]);
            start = index;
            chars = effective_text(segment).chars().count();
        }
    }
    if start < segments.len() {
        chunks.push(&segments[start..]);
    }
    chunks
}

/// 跨块合并：结论/待办按归一化引文判重（严格相等或 0.72 相似——不同块解出
/// 同一句但引文有细微差异时也去重），议题按块序拼接，术语表按 term 归并 variants。
fn merge_document(merged: &mut MinutesDocument, document: MinutesDocument) {
    merged.topics.extend(document.topics);
    let same_quote = |existing: &MinutesItem, item: &MinutesItem| {
        let left = normalize_for_match(&existing.evidence.quote_text);
        let right = normalize_for_match(&item.evidence.quote_text);
        left == right || similarity_at_least(&existing.evidence.quote_text, &item.evidence.quote_text, 0.72)
    };
    for item in document.conclusions {
        if !merged.conclusions.iter().any(|existing| same_quote(existing, &item)) {
            merged.conclusions.push(item);
        }
    }
    for item in document.action_items {
        if !merged.action_items.iter().any(|existing| same_quote(existing, &item)) {
            merged.action_items.push(item);
        }
    }
    for entry in document.glossary {
        match merged.glossary.iter_mut().find(|existing| existing.term == entry.term) {
            Some(existing) => {
                for variant in entry.variants {
                    if !existing.variants.contains(&variant) {
                        existing.variants.push(variant);
                    }
                }
            }
            None => merged.glossary.push(entry),
        }
    }
}

/// 纪要 JSON 的宽松中间表示（LLM 输出直接反序列化目标）。
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "snake_case", default)]
pub struct MinutesDraftJson {
    topics: Vec<MinutesTopicDraft>,
    conclusions: Vec<MinutesItemDraft>,
    action_items: Vec<MinutesItemDraft>,
    glossary: Vec<GlossaryDraft>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct MinutesTopicDraft {
    title: String,
    summary: String,
    start_ms: Option<i64>,
    end_ms: Option<i64>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct MinutesItemDraft {
    text: String,
    #[serde(default)]
    owner: Option<String>,
    quote: String,
    start_ms: Option<i64>,
    end_ms: Option<i64>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct GlossaryDraft {
    term: String,
    #[serde(default)]
    variants: Vec<String>,
}

pub const MINUTES_PROMPT_HEADER: &str = r#"你是会议纪要整理助手。给你一段带时间戳的会议逐字稿（格式：[序号][时间 mm:ss] 文本）。请提取：
1. topics：讨论的议题，按出现顺序；每项给 title、summary（2 句以内）、start_ms、end_ms（该议题讨论的起止毫秒）。
2. conclusions：达成的结论/决定；每项给 text（结论本身）、quote（逐字稿中支持该结论的原话片段，至少 8 个字）、start_ms、end_ms（quote 在逐字稿中的起止毫秒）。
3. action_items：待办事项；每项给 text（做什么）、owner（谁负责，没有就空字符串）、quote、start_ms、end_ms（同上）。
4. glossary：本场出现的人名与专有术语；每项给 term（规范写法）、variants（逐字稿里出现的其它写法/识别变体，没有就空数组）。
硬性规则：
- quote 必须是逐字稿原文的逐字片段；给不出可靠出处的结论/待办宁可不写，不许编造。
- start_ms/end_ms 必须对应 quote 实际出现的位置。
- 只输出 JSON，不要多余文字。格式：
{"topics":[],"conclusions":[],"action_items":[],"glossary":[]}"#;

/// 构建纪要提示词（逐字稿编号 + 时间戳）。
pub fn build_minutes_prompt(segments: &[TranscriptSegment]) -> String {
    let mut prompt = String::from(MINUTES_PROMPT_HEADER);
    prompt.push_str("\n\n逐字稿：\n");
    for (index, segment) in segments.iter().enumerate() {
        let start = format_mmss(segment.start_ms);
        let speaker = segment
            .speaker_label
            .as_deref()
            .filter(|label| !label.trim().is_empty())
            .map(|label| format!("[{label}] "))
            .unwrap_or_default();
        let text = segment
            .edited_text
            .as_deref()
            .or(segment.normalized_text.as_deref())
            .unwrap_or(&segment.original_text);
        prompt.push_str(&format!("[{}][{}]{label}{text}\n", index + 1, start, label = speaker));
    }
    prompt
}

pub fn format_mmss(ms: i64) -> String {
    let total_seconds = (ms / 1_000).max(0);
    format!("{:02}:{:02}", total_seconds / 60, total_seconds % 60)
}

/// 解析 LLM 输出（容忍 ```json 围栏）。
pub fn parse_minutes_draft(raw: &str) -> AppResult<MinutesDraftJson> {
    let trimmed = raw.trim();
    let json_body = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .and_then(|body| body.strip_suffix("```"))
        .map(str::trim)
        .unwrap_or(trimmed);
    serde_json::from_str(json_body)
        .map_err(|error| AppError::Invalid(format!("纪要输出不是有效 JSON：{error}")))
}

/* ---------------- 证据校验（fail-closed） ---------------- */

/// 归一化文本：只留字母数字与 CJK，小写。
fn normalize_for_match(text: &str) -> String {
    text.chars()
        .filter(|ch| ch.is_alphanumeric())
        .flat_map(|ch| ch.to_lowercase())
        .collect()
}

/// 编辑距离（小规模 O(nm)，逐字稿窗口足够小）。
fn levenshtein(left: &[char], right: &[char]) -> usize {
    let mut previous: Vec<usize> = (0..=right.len()).collect();
    for (i, left_char) in left.iter().enumerate() {
        let mut current = vec![i + 1];
        for (j, right_char) in right.iter().enumerate() {
            let substitution = previous[j] + usize::from(left_char != right_char);
            let deletion = previous[j + 1] + 1;
            let insertion = current[j] + 1;
            current.push(substitution.min(deletion).min(insertion));
        }
        previous = current;
    }
    previous[right.len()]
}

/// 相似度 ≥0.72 判定（与分析链路口径一致）。
fn similarity_at_least(left: &str, right: &str, threshold: f64) -> bool {
    let left: Vec<char> = normalize_for_match(left).chars().collect();
    let right: Vec<char> = normalize_for_match(right).chars().collect();
    if left.is_empty() || right.is_empty() {
        return false;
    }
    let distance = levenshtein(&left, &right);
    let max = left.len().max(right.len());
    (1.0 - distance as f64 / max as f64) >= threshold
}

/// 在逐字稿里找 quote 的真实位置：先按声称的窗口附近找，找不到就全文滑窗反查。
/// 返回校正后的证据窗口；找不到返回 None（该条目将被丢弃）。
/// 强制线：归一化后至少 8 字（与提示词承诺一致），防止两三个字就「命中」。
pub fn locate_quote(
    quote: &str,
    claimed_start_ms: Option<i64>,
    claimed_end_ms: Option<i64>,
    segments: &[TranscriptSegment],
) -> Option<MinutesEvidence> {
    let quote_normalized = normalize_for_match(quote);
    if quote_normalized.chars().count() < 8 {
        return None;
    }
    let claimed = claimed_start_ms
        .zip(claimed_end_ms)
        .filter(|(start, end)| end >= start);
    // 1) 按声称窗口拼段匹配；命中后仍收敛到最小跨度（宽声称窗不原样采信，
    //    seek 锚点才不会偏早）。
    if let Some((start, end)) = claimed {
        let overlapping: Vec<&TranscriptSegment> = segments
            .iter()
            .filter(|segment| segment.start_ms < end && segment.end_ms > start)
            .collect();
        let window_text: String = overlapping
            .iter()
            .map(|segment| effective_text(segment))
            .collect::<Vec<_>>()
            .join("");
        let window_normalized = normalize_for_match(&window_text);
        let window_matches = window_normalized.contains(&quote_normalized)
            || similarity_at_least(&window_text, quote, 0.72);
        if window_matches {
            // 两指收缩：起点取「尾部仍包含 quote」的最晚位置（最大 best_start），
            // 终点取「头部仍包含 quote」的最早位置（最小 best_end）——
            // 得到包含 quote 的最小连续区间。
            let mut best_start = 0usize;
            let mut best_end = overlapping.len().saturating_sub(1);
            if !overlapping.is_empty() {
                for candidate in 0..overlapping.len() {
                    let tail: String = overlapping[candidate..]
                        .iter()
                        .map(|segment| effective_text(segment))
                        .collect();
                    if normalize_for_match(&tail).contains(&quote_normalized) {
                        best_start = candidate;
                    }
                }
                for candidate in best_start..overlapping.len() {
                    let head: String = overlapping[best_start..=candidate]
                        .iter()
                        .map(|segment| effective_text(segment))
                        .collect();
                    if normalize_for_match(&head).contains(&quote_normalized) {
                        best_end = candidate;
                        break; // 正向扫：第一个仍包含 quote 的位置就是最小终点。
                    }
                }
            }
            return Some(MinutesEvidence {
                start_ms: overlapping[best_start].start_ms,
                end_ms: overlapping[best_end].end_ms,
                quote_text: quote.to_owned(),
            });
        }
    }
    // 2) 全文滑窗反查（≤24 段窗口，分析链路同款上限）。
    for start_index in 0..segments.len() {
        let mut combined = String::new();
        for (offset, segment) in segments[start_index..]
            .iter()
            .take(24)
            .enumerate()
        {
            combined.push_str(&effective_text(segment));
            let combined_normalized = normalize_for_match(&combined);
            let matched = combined_normalized.contains(&quote_normalized)
                || similarity_at_least(&combined, quote, 0.72);
            if matched {
                // 收敛到最小跨度：从命中段往回收，保留仍然包含 quote 的最晚起点，
                // 回链时间窗才不至于把无关前缀也圈进去。
                let end_index = start_index + offset;
                let mut best_start = start_index;
                for candidate in start_index..=end_index {
                    let tail: String = segments[candidate..=end_index]
                        .iter()
                        .map(effective_text)
                        .collect();
                    if normalize_for_match(&tail).contains(&quote_normalized) {
                        best_start = candidate;
                    }
                }
                return Some(MinutesEvidence {
                    start_ms: segments[best_start].start_ms,
                    end_ms: segments[end_index].end_ms,
                    quote_text: quote.to_owned(),
                });
            }
        }
    }
    None
}

fn effective_text(segment: &TranscriptSegment) -> &str {
    segment
        .edited_text
        .as_deref()
        .or(segment.normalized_text.as_deref())
        .unwrap_or(&segment.original_text)
}

/// 解析草稿 → 逐条证据校验 → 丢弃不可溯源条目。
/// `duration_ms` 用于夹紧时间窗（LLM 给出越界时间时收敛到录音范围内）。
pub fn finalize_minutes_draft(
    draft: MinutesDraftJson,
    segments: &[TranscriptSegment],
    duration_ms: i64,
) -> MinutesDocument {
    let mut dropped = 0usize;
    let mut conclusions = Vec::new();
    for item in draft.conclusions {
        if item.text.trim().is_empty() {
            dropped += 1;
            continue;
        }
        match locate_quote(&item.quote, item.start_ms, item.end_ms, segments) {
            Some(evidence) => conclusions.push(MinutesItem {
                id: Uuid::new_v4().to_string(),
                kind: MinutesItemKind::Conclusion,
                text: item.text.trim().to_owned(),
                owner: item.owner.map(|owner| owner.trim().to_owned()).filter(|owner| !owner.is_empty()),
                evidence: clamp_evidence(evidence, duration_ms),
            }),
            None => dropped += 1,
        }
    }
    let mut action_items = Vec::new();
    for item in draft.action_items {
        if item.text.trim().is_empty() {
            dropped += 1;
            continue;
        }
        match locate_quote(&item.quote, item.start_ms, item.end_ms, segments) {
            Some(evidence) => action_items.push(MinutesItem {
                id: Uuid::new_v4().to_string(),
                kind: MinutesItemKind::Action,
                text: item.text.trim().to_owned(),
                owner: item.owner.map(|owner| owner.trim().to_owned()).filter(|owner| !owner.is_empty()),
                evidence: clamp_evidence(evidence, duration_ms),
            }),
            None => dropped += 1,
        }
    }
    let topics = draft
        .topics
        .iter()
        .filter(|topic| !topic.title.trim().is_empty())
        .map(|topic| {
            let start_ms = topic.start_ms.map(|ms| ms.clamp(0, duration_ms.max(0)));
            let end_ms = topic.end_ms.map(|ms| ms.clamp(0, duration_ms.max(0)));
            // 时间窗顺序修正：LLM 给反了起止时收敛为点。
            let (start_ms, end_ms) = match (start_ms, end_ms) {
                (Some(start), Some(end)) if end < start => (Some(end), Some(start)),
                other => other,
            };
            MinutesTopic {
                id: Uuid::new_v4().to_string(),
                title: topic.title.trim().to_owned(),
                summary: topic.summary.trim().to_owned(),
                start_ms,
                end_ms,
            }
        })
        .collect();
    let glossary = draft
        .glossary
        .iter()
        .filter(|entry| !entry.term.trim().is_empty())
        .map(|entry| GlossaryEntry {
            term: entry.term.trim().to_owned(),
            variants: entry
                .variants
                .iter()
                .map(|variant| variant.trim().to_owned())
                .filter(|variant| !variant.is_empty())
                .collect(),
        })
        .collect();
    MinutesDocument {
        topics,
        conclusions,
        action_items,
        glossary,
        dropped_unverifiable: dropped,
    }
}

fn clamp_evidence(mut evidence: MinutesEvidence, duration_ms: i64) -> MinutesEvidence {
    if duration_ms > 0 {
        evidence.start_ms = evidence.start_ms.clamp(0, duration_ms);
        evidence.end_ms = evidence.end_ms.clamp(0, duration_ms);
    }
    if evidence.end_ms < evidence.start_ms {
        evidence.end_ms = evidence.start_ms;
    }
    evidence
}

/// 100% 回链自检：任何结论/待办缺证据（或时间窗无效）都算结构不完整。
pub fn minutes_document_is_fully_linkable(document: &MinutesDocument) -> bool {
    document
        .conclusions
        .iter()
        .chain(document.action_items.iter())
        .all(|item| {
            !item.evidence.quote_text.trim().is_empty()
                && item.evidence.end_ms >= item.evidence.start_ms
                && item.evidence.start_ms >= 0
                && !item.text.trim().is_empty()
        })
}

/* ---------------- 生成编排 ---------------- */

pub struct GenerationOutcome {
    pub provider: String,
    pub model: String,
    pub document: MinutesDocument,
}

/// 用给定的文本生成器跑一次纪要生成分块（本地 Ollama 与云端共用入口）。
pub fn generate_minutes_chunk<F>(
    segments: &[TranscriptSegment],
    generate: &mut F,
) -> AppResult<MinutesDraftJson>
where
    F: FnMut(&str) -> AppResult<String>,
{
    let prompt = build_minutes_prompt(segments);
    let raw = generate(&prompt)?;
    parse_minutes_draft(&raw)
}

/// 完整生成：长逐字稿按 MINUTES_CHUNK_CHARS 分块逐块生成再合并
/// （防长会议撞上下文窗口静默截断出「半场纪要」）；证据校验始终对
/// 全量逐字稿进行（跨块引文也能定位）。
pub fn generate_minutes_document<F>(
    segments: &[TranscriptSegment],
    duration_ms: i64,
    mut generate: F,
) -> AppResult<MinutesDocument>
where
    F: FnMut(&str) -> AppResult<String>,
{
    let chunks = chunk_segments(segments);
    let mut merged = MinutesDocument::default();
    let mut dropped_total = 0usize;
    for chunk in chunks {
        let draft = generate_minutes_chunk(chunk, &mut generate)?;
        let mut document = finalize_minutes_draft(draft, segments, duration_ms);
        dropped_total += document.dropped_unverifiable;
        document.dropped_unverifiable = 0;
        merge_document(&mut merged, document);
    }
    merged.dropped_unverifiable = dropped_total;
    Ok(merged)
}

/// 后台生成 + 落库。调用方先用 start_minutes_generation 拿闩（原子，
/// 拿不到说明另一路在跑）。失败写 failed 状态（可重试），绝不影响录音本身。
pub fn run_minutes_generation(library: &ManagedLibrary, record_id: &str) -> AppResult<MinutesDocument> {
    let repository = library.repository();
    let segments = repository.list_transcript_segments(record_id)?;
    if segments.is_empty() {
        return Err(AppError::Invalid("该记录还没有逐字稿，无法生成纪要".to_owned()));
    }
    let duration_ms = repository
        .get_record(record_id)
        .map(|record| record.audio_duration_ms)
        .unwrap_or(0);
    let outcome = (|| -> AppResult<GenerationOutcome> {
        // 通道判定与分析链路同款：走云端当且仅当理解整理已启用 + 隐私同意已给
        // + Key 可用；纪要同样会把转写文本发给模型，隐私门禁一个都不能少。
        let ext_settings = repository.external_ai_settings(false, false)?;
        if ext_settings.enabled && ext_settings.privacy_consent_at.is_some() {
            let settings = crate::external_ai_gate::require_external_ai_consent(library)?;
            let api_key = crate::external_ai_gate::get_api_key()?.ok_or_else(|| {
                AppError::Invalid("请先配置第三方文本理解 API Key".to_owned())
            })?;
            let client = crate::memory::OpenAiCompatibleClient::new(
                &settings.base_url,
                &settings.model,
                &api_key,
            )?;
            let document = generate_minutes_document(segments.as_slice(), duration_ms, |prompt| {
                // 心跳挂在生成闭包上：每个分块一次模型调用，调用间刷新 updated_at，
                // 防长生成被陈旧抢占误伤。
                let _ = repository.touch_minutes_generation(record_id);
                client.complete_text(
                    "你是会议纪要整理器。仅返回约定的简体中文 JSON，不要使用 Markdown 或额外说明。",
                    prompt,
                )
            })?;
            return Ok(GenerationOutcome {
                provider: "external-openai-compatible".to_owned(),
                model: settings.model,
                document,
            });
        }
        let model = repository.knowledge_settings()?.analysis_model;
        let adapter = crate::analysis::OllamaAdapter::detect(&model)?;
        let document = generate_minutes_document(segments.as_slice(), duration_ms, |prompt| {
            let _ = repository.touch_minutes_generation(record_id);
            adapter.raw_text(prompt)
        })?;
        Ok(GenerationOutcome {
            provider: "ollama".to_owned(),
            model,
            document,
        })
    })();
    match outcome {
        Ok(outcome) => {
            if !minutes_document_is_fully_linkable(&outcome.document) {
                repository.fail_minutes(
                    record_id,
                    "纪要结构不完整：存在无法回链的条目（不应发生，属于生成侧缺陷）",
                )?;
                return Err(AppError::Invalid(
                    "纪要结构不完整：存在无法回链的条目".to_owned(),
                ));
            }
            repository.save_minutes_document(
                record_id,
                &outcome.provider,
                &outcome.model,
                &outcome.document,
            )?;
            Ok(outcome.document)
        }
        Err(error) => {
            repository.fail_minutes(record_id, &error.to_string())?;
            Err(error)
        }
    }
}

/* ---------------- 命令 ---------------- */

#[tauri::command]
pub fn get_record_minutes(state: tauri::State<'_, crate::state::AppState>, record_id: String) -> Result<Option<MinutesView>, String> {
    minutes_view(&state.library, &record_id).map_err(|error| error.to_frontend())
}

fn minutes_view(library: &ManagedLibrary, record_id: &str) -> AppResult<Option<MinutesView>> {
    let repository = library.repository();
    let Some(row) = repository.get_minutes(record_id)? else {
        return Ok(None);
    };
    let document: Option<MinutesDocument> = if row.content_json == "{}" {
        None
    } else {
        serde_json::from_str(&row.content_json)
            .map_err(|error| AppError::Invalid(format!("纪要内容损坏：{error}")))
            .map(Some)?
    };
    let item_statuses = repository
        .list_minutes_items(record_id)?
        .into_iter()
        .map(|item| {
            (
                item.id,
                if item.status == "confirmed" {
                    MinutesItemStatus::Confirmed
                } else {
                    MinutesItemStatus::Provisional
                },
            )
        })
        .collect();
    Ok(Some(MinutesView {
        record_id: row.record_id,
        status: MinutesStatus::from_db(&row.status),
        document,
        item_statuses,
        provider: row.provider,
        model: row.model,
        error: row.error_message,
        generated_at: (row.status == "completed").then_some(row.updated_at),
    }))
}

/// 手动生成/重试。先同步拿生成闩（保证返回视图一定是 generating，前端才
/// 会进入轮询），再后台执行；panic 也兜底回 failed，不留死状态。
#[tauri::command]
pub async fn generate_minutes(
    state: tauri::State<'_, crate::state::AppState>,
    record_id: String,
) -> Result<MinutesView, String> {
    let library = state.library.clone();
    library
        .repository()
        .ensure_minutes_pending(&record_id)
        .map_err(|error| error.to_frontend())?;
    if !library
        .repository()
        .start_minutes_generation(&record_id)
        .map_err(|error| error.to_frontend())?
    {
        return Err("纪要正在生成中，请稍候".to_owned());
    }
    let library_for_thread = library.clone();
    let record_for_thread = record_id.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run_minutes_generation(&library_for_thread, &record_for_thread)
        }));
        if let Err(panic) = result {
            let message = panic
                .downcast_ref::<String>()
                .cloned()
                .unwrap_or_else(|| "纪要生成线程异常".to_owned());
            let _ = library_for_thread
                .repository()
                .fail_minutes(&record_for_thread, &format!("生成异常：{message}"));
        }
    });
    minutes_view(&state.library, &record_id)
        .map_err(|error| error.to_frontend())?
        .ok_or_else(|| "纪要初始化失败".to_owned().into())
}

/// 转写完成后自动入队：只在 minutes 尚无任何行时触发一次。
/// 生成失败记 failed 状态，录音与逐字稿不受任何影响。
pub fn maybe_autogenerate_minutes(library_root: std::path::PathBuf, record_id: String) {
    std::thread::Builder::new()
        .name("minutes-autogenerate".to_owned())
        .spawn(move || {
            let Ok(library) = crate::library::ManagedLibrary::open(library_root) else {
                return;
            };
            let repository = library.repository();
            if repository.get_minutes(&record_id).ok().flatten().is_some() {
                return; // 已有行（生成过/失败过），不自动重跑
            }
            if repository.ensure_minutes_pending(&record_id).is_err() {
                return;
            }
            // 拿生成闩（原子）；拿不到说明另一路在生成，直接退出。
            match repository.start_minutes_generation(&record_id) {
                Ok(true) => {}
                _ => return,
            }
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                run_minutes_generation(&library, &record_id)
            }));
            if let Err(panic) = result {
                let message = panic
                    .downcast_ref::<String>()
                    .cloned()
                    .unwrap_or_else(|| "纪要生成线程异常".to_owned());
                let _ = repository.fail_minutes(&record_id, &format!("生成异常：{message}"));
            }
        })
        .ok();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::TranscriptSegmentInput;

    fn segments_fixture() -> Vec<TranscriptSegment> {
        vec![
            segment("seg-1", 0, 6_000, "我们把第三季度的产品复盘安排在下周三下午两点"),
            segment("seg-2", 6_000, 12_000, "服务端迁移由老张负责，这周内完成灰度"),
        ]
    }

    fn segment(id: &str, start_ms: i64, end_ms: i64, text: &str) -> TranscriptSegment {
        TranscriptSegment {
            id: id.to_owned(),
            record_id: "record-1".to_owned(),
            sequence: 0,
            speaker_label: Some("说话人 1".to_owned()),
            start_ms,
            end_ms,
            original_text: text.to_owned(),
            normalized_text: None,
            normalization_version: None,
            edited_text: None,
        }
    }

    #[test]
    fn parse_minutes_draft_tolerates_code_fence_and_defaults() {
        let fenced = parse_minutes_draft("```json\n{\"conclusions\":[{\"text\":\"结论\",\"quote\":\"我们把第三季度的产品复盘\"}]}\n```").unwrap();
        assert_eq!(fenced.conclusions.len(), 1);
        assert!(fenced.topics.is_empty());
        // 缺字段 → 空默认。
        let empty = parse_minutes_draft("{}").unwrap();
        assert!(empty.conclusions.is_empty());
        assert!(parse_minutes_draft("不是 JSON").is_err());
    }

    #[test]
    fn locate_quote_verifies_real_and_rejects_fabricated() {
        let segments = segments_fixture();
        // 真实存在的引文：即使时间戳给偏了也能全文反查校正。
        let located = locate_quote(
            "服务端迁移由老张负责",
            Some(60_000),
            Some(70_000),
            &segments,
        )
        .expect("真实引文必须能定位");
        assert_eq!(located.start_ms, 6_000);
        assert_eq!(located.end_ms, 12_000);
        // 编造的引文：拒绝（fail-closed）。
        assert!(locate_quote("这句话逐字稿里根本没有说过呀", None, None, &segments).is_none());
        // 低于 8 字强制线的引文：拒绝。
        assert!(locate_quote("完成灰度", None, None, &segments).is_none());
    }

    #[test]
    fn claimed_window_is_refined_to_minimal_span() {
        let segments = segments_fixture();
        // 声称窗口圈住了两段，但 quote 只在第二段：收敛到 [6s, 12s]，
        // seek 锚点不应被无关前缀拖早。
        let located = locate_quote(
            "服务端迁移由老张负责，这周内完成灰度",
            Some(0),
            Some(12_000),
            &segments,
        )
        .expect("逐字引文必须命中");
        assert_eq!(located.start_ms, 6_000);
        assert_eq!(located.end_ms, 12_000);
        // 声称窗本身就贴得准：原样采用。
        let located =
            locate_quote("我们把第三季度的产品复盘", Some(0), Some(6_000), &segments).unwrap();
        assert_eq!(located.start_ms, 0);
        assert_eq!(located.end_ms, 6_000);
        // 终点收敛边界：quote 在声称窗首段 → 终点应收到首段末尾，而不是整窗。
        let located =
            locate_quote("我们把第三季度的产品复盘", Some(0), Some(12_000), &segments).unwrap();
        assert_eq!(located.start_ms, 0);
        assert_eq!(located.end_ms, 6_000);
        // 跨段 quote + 宽声称窗：最小跨度 = 两段并集。
        let located = locate_quote(
            "我们把第三季度的产品复盘安排在下周三下午两点服务端迁移由老张负责",
            Some(0),
            Some(12_000),
            &segments,
        )
        .unwrap();
        assert_eq!(located.start_ms, 0);
        assert_eq!(located.end_ms, 12_000);
    }

    #[test]
    fn long_transcript_is_chunked_and_merged_without_duplicates() {
        // 构造 4 段、每段 2000 字（总 8000 > 6000 → 应切成 2 块）。
        let filler: String = "会议讨论".repeat(500);
        let segments: Vec<TranscriptSegment> = (0..4)
            .map(|index| {
                segment(
                    &format!("seg-{index}"),
                    index * 10_000,
                    (index + 1) * 10_000,
                    &format!("{filler}第{index}段结论依据"),
                )
            })
            .collect();
        let chunk_sizes: Vec<usize> = chunk_segments(&segments)
            .iter()
            .map(|chunk| chunk.len())
            .collect();
        assert_eq!(chunk_sizes, vec![2, 2], "应按 6000 字上限切成两块");

        let mut calls = 0;
        let document = generate_minutes_document(&segments, 40_000, |_prompt| {
            calls += 1;
            // 每块返回同样的结论（模拟跨块重复解出同一句）——合并必须去重。
            Ok(r#"{"conclusions":[{"text":"同一句结论","quote":"会议讨论会议讨论"}]}"#.to_owned())
        })
        .unwrap();
        assert_eq!(calls, 2, "每个块一次模型调用");
        assert_eq!(document.conclusions.len(), 1, "跨块重复引文必须去重");
        assert_eq!(document.dropped_unverifiable, 0);
    }

    #[test]
    fn generation_latch_is_atomic_and_stale_generating_is_recoverable() {
        let (library, root) = test_library();
        let record_id = seed_record_with_transcript(&library);
        let repository = library.repository();
        repository.ensure_minutes_pending(&record_id).unwrap();
        // 闩：第一次拿到，第二次必须拿不到（并发双击防护）。
        assert!(repository.start_minutes_generation(&record_id).unwrap());
        assert!(!repository.start_minutes_generation(&record_id).unwrap());
        // 陈旧抢占：把 updated_at 拨回 11 分钟前（> 10 分钟阈值）→ 可重新拿闩。
        {
            let connection = rusqlite::Connection::open(library.root().join("memory.db")).unwrap();
            connection
                .execute(
                    "UPDATE minutes SET updated_at = ?1 WHERE record_id = ?2",
                    rusqlite::params![
                        (chrono::Utc::now() - chrono::Duration::seconds(660)).to_rfc3339(),
                        record_id
                    ],
                )
                .unwrap();
        }
        assert!(repository.start_minutes_generation(&record_id).unwrap());
        // 再回拨（抢占会刷新心跳）：模拟「拿到闩之后进程崩了」的残留。
        {
            let connection = rusqlite::Connection::open(library.root().join("memory.db")).unwrap();
            connection
                .execute(
                    "UPDATE minutes SET updated_at = ?1 WHERE record_id = ?2",
                    rusqlite::params![
                        (chrono::Utc::now() - chrono::Duration::seconds(660)).to_rfc3339(),
                        record_id
                    ],
                )
                .unwrap();
        }
        // 启动恢复：generating 且无心跳 → failed + 明确原因。
        let recovered = repository.recover_stale_minutes_generating().unwrap();
        assert!(recovered >= 1);
        let view = minutes_view(&library, &record_id).unwrap().unwrap();
        assert_eq!(view.status, MinutesStatus::Failed);
        assert!(view.error.unwrap_or_default().contains("中断"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn finalize_drops_unverifiable_and_keeps_verified() {
        let draft: MinutesDraftJson = serde_json::from_str(
            r#"{
                "conclusions": [
                    {"text": "复盘时间定了", "quote": "我们把第三季度的产品复盘安排在下周三下午两点", "start_ms": 0, "end_ms": 6000},
                    {"text": "编造的结论", "quote": "这句话逐字稿里根本没有"}
                ],
                "action_items": [
                    {"text": "完成灰度", "owner": "老张", "quote": "服务端迁移由老张负责，这周内完成灰度", "start_ms": 6000, "end_ms": 12000}
                ],
                "glossary": [{"term": "灰度", "variants": ["灰度发布"]}]
            }"#,
        )
        .unwrap();
        let document = finalize_minutes_draft(draft, &segments_fixture(), 12_000);
        assert_eq!(document.conclusions.len(), 1);
        assert_eq!(document.action_items.len(), 1);
        assert_eq!(document.dropped_unverifiable, 1);
        assert_eq!(document.action_items[0].owner.as_deref(), Some("老张"));
        // 100% 回链：留下的每一条都有有效证据。
        assert!(minutes_document_is_fully_linkable(&document));
    }

    #[test]
    fn linkable_check_rejects_missing_evidence() {
        let mut document = MinutesDocument::default();
        assert!(minutes_document_is_fully_linkable(&document));
        document.conclusions.push(MinutesItem {
            id: "i".to_owned(),
            kind: MinutesItemKind::Conclusion,
            text: "结论".to_owned(),
            owner: None,
            evidence: MinutesEvidence {
                start_ms: 0,
                end_ms: 0,
                quote_text: String::new(),
            },
        });
        assert!(!minutes_document_is_fully_linkable(&document));
    }

    #[test]
    fn prompt_carries_numbered_timestamped_transcript() {
        let prompt = build_minutes_prompt(&segments_fixture());
        assert!(prompt.contains("[1][00:00][说话人 1] 我们把第三季度的产品复盘"));
        assert!(prompt.contains("[2][00:06][说话人 1] 服务端迁移"));
        assert!(prompt.contains("quote 必须是逐字稿原文"));
    }

    fn test_library() -> (crate::library::ManagedLibrary, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!("echo-minutes-{}", Uuid::new_v4()));
        (crate::library::ManagedLibrary::open(root.clone()).unwrap(), root)
    }

    fn seed_record_with_transcript(library: &crate::library::ManagedLibrary) -> String {
        let repository = library.repository();
        let record = repository
            .create_record(
                "测试会议",
                None,
                &library.root().join("audio/test.wav"),
                "hash-1",
                12_000,
            )
            .unwrap();
        repository
            .save_transcript_with_metadata(
                &record.id,
                "whisper.cpp",
                "test-model",
                "zh",
                "enhanced-v2",
                "{}",
                &[
                    TranscriptSegmentInput {
                        start_ms: 0,
                        end_ms: 6_000,
                        speaker_label: Some("说话人 1".to_owned()),
                        original_text: "我们把第三季度的产品复盘安排在下周三下午两点".to_owned(),
                    },
                    TranscriptSegmentInput {
                        start_ms: 6_000,
                        end_ms: 12_000,
                        speaker_label: Some("说话人 1".to_owned()),
                        original_text: "服务端迁移由老张负责，这周内完成灰度".to_owned(),
                    },
                ],
            )
            .unwrap();
        record.id
    }

    fn verified_document() -> MinutesDocument {
        finalize_minutes_draft(
            serde_json::from_str(
                r#"{"conclusions":[{"text":"复盘时间定了","quote":"我们把第三季度的产品复盘安排在下周三下午两点"}],
                    "action_items":[{"text":"完成灰度","owner":"老张","quote":"服务端迁移由老张负责，这周内完成灰度"}]}"#,
            )
            .unwrap(),
            &segments_fixture(),
            12_000,
        )
    }

    #[test]
    fn generation_pipeline_with_fake_generator_saves_document() {
        let (library, root) = test_library();
        let record_id = seed_record_with_transcript(&library);
        let document = generate_minutes_document(
            library.repository().list_transcript_segments(&record_id).unwrap().as_slice(),
            12_000,
            |_prompt| {
                Ok(r#"{"conclusions":[{"text":"复盘时间定了","quote":"我们把第三季度的产品复盘安排在下周三下午两点"}],
                      "action_items":[{"text":"完成灰度","owner":"老张","quote":"服务端迁移由老张负责，这周内完成灰度"}]}"#
                    .to_owned())
            },
        )
        .unwrap();
        let repository = library.repository();
        repository.ensure_minutes_pending(&record_id).unwrap();
        repository
            .save_minutes_document(&record_id, "fake", "fake-model", &document)
            .unwrap();
        let view = minutes_view(&library, &record_id).unwrap().unwrap();
        assert_eq!(view.status, MinutesStatus::Completed);
        assert_eq!(view.document.unwrap().conclusions.len(), 1);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn item_two_states_and_mcp_visibility() {
        let (library, root) = test_library();
        let record_id = seed_record_with_transcript(&library);
        let repository = library.repository();
        repository.ensure_minutes_pending(&record_id).unwrap();
        repository
            .save_minutes_document(&record_id, "fake", "fake", &verified_document())
            .unwrap();

        // 未确认（默认）：已确认列表必须为空 —— MCP 侧只读这里，不可见性成立。
        assert!(repository.list_confirmed_minutes_items(100).unwrap().is_empty());

        // 确认一条 → 出现在已确认列表；溯源字段完整。
        let items = repository.list_minutes_items(&record_id).unwrap();
        assert_eq!(items.len(), 2);
        let target = items.iter().find(|i| i.kind == "conclusion").unwrap();
        repository
            .set_minutes_item_status(&target.id, MinutesItemStatus::Confirmed)
            .unwrap();
        let confirmed = repository.list_confirmed_minutes_items(100).unwrap();
        assert_eq!(confirmed.len(), 1);
        assert_eq!(confirmed[0].id, target.id);
        let evidence: MinutesEvidence = serde_json::from_str(&confirmed[0].evidence_json).unwrap();
        assert!(evidence.end_ms > evidence.start_ms);
        assert!(!evidence.quote_text.is_empty());

        // 取消确认 → 再次不可见。
        repository
            .set_minutes_item_status(&target.id, MinutesItemStatus::Provisional)
            .unwrap();
        assert!(repository.list_confirmed_minutes_items(100).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn regeneration_resets_confirmations_to_provisional() {
        let (library, root) = test_library();
        let record_id = seed_record_with_transcript(&library);
        let repository = library.repository();
        repository.ensure_minutes_pending(&record_id).unwrap();
        repository
            .save_minutes_document(&record_id, "fake", "fake", &verified_document())
            .unwrap();
        let items = repository.list_minutes_items(&record_id).unwrap();
        for item in &items {
            repository
                .set_minutes_item_status(&item.id, MinutesItemStatus::Confirmed)
                .unwrap();
        }
        assert_eq!(repository.list_confirmed_minutes_items(100).unwrap().len(), 2);

        // 重新生成：条目全量重建，确认状态一律回到 provisional（隐私红线：
        // 确认必须针对当下这份内容，旧确认不能默默延续到新内容上）。
        repository
            .save_minutes_document(&record_id, "fake", "fake", &verified_document())
            .unwrap();
        assert!(repository.list_confirmed_minutes_items(100).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn failed_generation_keeps_record_intact_and_is_retryable() {
        let (library, root) = test_library();
        let record_id = seed_record_with_transcript(&library);
        let repository = library.repository();
        repository.ensure_minutes_pending(&record_id).unwrap();
        // 失败落状态：录音与逐字稿不受影响（record/transcript 完好）。
        repository.fail_minutes(&record_id, "模型不可用").unwrap();
        let view = minutes_view(&library, &record_id).unwrap().unwrap();
        assert_eq!(view.status, MinutesStatus::Failed);
        assert_eq!(view.error.as_deref(), Some("模型不可用"));
        assert!(!repository
            .list_transcript_segments(&record_id)
            .unwrap()
            .is_empty());
        // 重试路径：start_minutes_generation 清错误、回 generating。
        repository.start_minutes_generation(&record_id).unwrap();
        let view = minutes_view(&library, &record_id).unwrap().unwrap();
        assert_eq!(view.status, MinutesStatus::Generating);
        assert!(view.error.is_none());
        let _ = std::fs::remove_dir_all(root);
    }
}
