use crate::types::{TranscriptBlock, TranscriptSegment};
use ferrous_opencc::{config::BuiltinConfig, OpenCC};
use std::sync::OnceLock;

pub const NORMALIZATION_VERSION: &str = "opencc-t2s-v1";

pub fn normalize_chinese(text: &str) -> String {
    static CONVERTER: OnceLock<Option<OpenCC>> = OnceLock::new();
    CONVERTER
        .get_or_init(|| OpenCC::from_config(BuiltinConfig::T2s).ok())
        .as_ref()
        .map_or_else(|| text.to_owned(), |converter| converter.convert(text))
}

pub fn effective_text(segment: &TranscriptSegment) -> &str {
    segment
        .edited_text
        .as_deref()
        .or(segment.normalized_text.as_deref())
        .unwrap_or(&segment.original_text)
}

pub fn build_blocks(segments: &[TranscriptSegment]) -> Vec<TranscriptBlock> {
    let mut blocks = Vec::new();
    let mut current: Vec<TranscriptSegment> = Vec::new();
    for segment in segments {
        if let Some(previous) = current.last() {
            let duration = segment.end_ms - current[0].start_ms;
            let gap = segment.start_ms - previous.end_ms;
            let speaker_changed = previous.speaker_label.is_some()
                && segment.speaker_label.is_some()
                && previous.speaker_label != segment.speaker_label;
            let natural_boundary =
                ends_sentence(effective_text(previous)) || gap >= 1_200 || speaker_changed;
            if duration > 20_000 || (duration >= 8_000 && natural_boundary) {
                blocks.push(make_block(&current));
                current.clear();
            }
        }
        current.push(segment.clone());
        if segment.end_ms - segment.start_ms > 20_000 {
            blocks.push(make_block(&current));
            current.clear();
        }
    }
    if !current.is_empty() {
        if let Some(previous) = blocks.last_mut() {
            let current_duration = current.last().unwrap().end_ms - current[0].start_ms;
            if current_duration < 8_000
                && current.last().unwrap().end_ms - previous.start_ms <= 20_000
            {
                previous
                    .segment_ids
                    .extend(current.iter().map(|item| item.id.clone()));
                previous.segments.extend(current);
                refresh_block(previous);
                return blocks;
            }
        }
        blocks.push(make_block(&current));
    }
    blocks
}

fn make_block(segments: &[TranscriptSegment]) -> TranscriptBlock {
    let first = segments.first().expect("block must contain a segment");
    let mut block = TranscriptBlock {
        id: first.id.clone(),
        segment_ids: segments.iter().map(|item| item.id.clone()).collect(),
        start_ms: first.start_ms,
        end_ms: segments.last().unwrap().end_ms,
        text: String::new(),
        speaker_label: common_speaker(segments),
        segments: segments.to_vec(),
    };
    refresh_block(&mut block);
    block
}

fn refresh_block(block: &mut TranscriptBlock) {
    block.end_ms = block
        .segments
        .last()
        .map_or(block.start_ms, |item| item.end_ms);
    block.speaker_label = common_speaker(&block.segments);
    block.text = join_text(&block.segments);
}

fn common_speaker(segments: &[TranscriptSegment]) -> Option<String> {
    let first = segments.first()?.speaker_label.as_deref()?;
    segments
        .iter()
        .all(|item| item.speaker_label.as_deref() == Some(first))
        .then(|| first.to_owned())
}

fn join_text(segments: &[TranscriptSegment]) -> String {
    let mut output = String::new();
    for segment in segments {
        let text = effective_text(segment).trim();
        if text.is_empty() {
            continue;
        }
        if needs_space(output.chars().last(), text.chars().next()) {
            output.push(' ');
        }
        output.push_str(text);
    }
    output
}

fn needs_space(left: Option<char>, right: Option<char>) -> bool {
    matches!((left, right), (Some(a), Some(b)) if a.is_ascii_alphanumeric() && b.is_ascii_alphanumeric())
}

fn ends_sentence(text: &str) -> bool {
    text.trim_end().chars().last().is_some_and(|character| {
        matches!(character, '。' | '！' | '？' | '；' | '.' | '!' | '?' | ';')
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segment(index: i64, start_ms: i64, end_ms: i64, text: &str) -> TranscriptSegment {
        TranscriptSegment {
            id: format!("s{index}"),
            record_id: "r".to_owned(),
            sequence: index,
            speaker_label: Some("未知".to_owned()),
            start_ms,
            end_ms,
            original_text: text.to_owned(),
            normalized_text: Some(normalize_chinese(text)),
            normalization_version: Some(NORMALIZATION_VERSION.to_owned()),
            edited_text: None,
        }
    }

    #[test]
    fn converts_traditional_to_simplified() {
        assert_eq!(normalize_chinese("這是一段繁體中文"), "这是一段繁体中文");
    }

    #[test]
    fn effective_text_prefers_edit_then_normalized_then_original() {
        let mut item = segment(0, 0, 1_000, "原文");
        item.normalized_text = Some("简体".to_owned());
        assert_eq!(effective_text(&item), "简体");
        item.edited_text = Some("人工".to_owned());
        assert_eq!(effective_text(&item), "人工");
    }

    #[test]
    fn blocks_target_eight_to_twenty_seconds() {
        let segments = (0..20)
            .map(|index| {
                segment(
                    index,
                    index * 1_000,
                    (index + 1) * 1_000,
                    if index == 9 {
                        "一句结束。"
                    } else {
                        "内容"
                    },
                )
            })
            .collect::<Vec<_>>();
        let blocks = build_blocks(&segments);
        assert_eq!(blocks.len(), 2);
        assert!(blocks
            .iter()
            .all(|block| block.end_ms - block.start_ms <= 20_000));
        assert_eq!(blocks[0].segment_ids.len(), 10);
    }

    #[test]
    fn short_tail_merges_only_when_combined_block_stays_within_limit() {
        let segments = (0..15)
            .map(|index| {
                segment(
                    index,
                    index * 1_000,
                    (index + 1) * 1_000,
                    if index == 9 {
                        "一句结束。"
                    } else {
                        "内容"
                    },
                )
            })
            .collect::<Vec<_>>();
        let blocks = build_blocks(&segments);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].end_ms, 15_000);
    }

    #[test]
    fn dense_hour_long_transcript_mostly_produces_reading_blocks_in_target_range() {
        let segments = (0..3_600)
            .map(|index| {
                segment(
                    index,
                    index * 1_000,
                    (index + 1) * 1_000,
                    if index % 10 == 9 {
                        "一句结束。"
                    } else {
                        "内容"
                    },
                )
            })
            .collect::<Vec<_>>();
        let blocks = build_blocks(&segments);
        let in_range = blocks
            .iter()
            .filter(|block| (8_000..=20_000).contains(&(block.end_ms - block.start_ms)))
            .count();
        assert!(in_range as f64 / blocks.len() as f64 >= 0.9);
    }
}
