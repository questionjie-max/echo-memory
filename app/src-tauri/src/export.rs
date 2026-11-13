use crate::analysis::{
    AnalysisChapterDraft, AnalysisCustomSectionDraft, AnalysisDraft, AnalysisHighlightDraft,
    AnalysisItemDraft,
};
use crate::error::{AppError, AppResult};
use crate::library::ManagedLibrary;
use crate::transcript::build_blocks;
use crate::types::{RecordBrief, TranscriptSegment};
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

pub fn export_record(
    library: &ManagedLibrary,
    record_id: &str,
    destination: &Path,
    format: &str,
) -> AppResult<PathBuf> {
    validate_format(format)?;
    let content = render_record(library, record_id, format)?;
    atomic_write(destination, content.as_bytes())?;
    Ok(destination.to_path_buf())
}
/// 渲染单条记录的 Markdown 全文（含逐字稿与分析），供产出文件夹复用。
pub fn record_markdown(library: &ManagedLibrary, record_id: &str) -> AppResult<String> {
    render_record(library, record_id, "md")
}
pub fn export_knowledge_base(
    library: &ManagedLibrary,
    project_id: Option<&str>,
    unfiled_only: bool,
    destination: &Path,
    format: &str,
) -> AppResult<PathBuf> {
    validate_format(format)?;
    fs::create_dir_all(destination)?;
    let folder_name = available_export_folder(destination, "回声记忆知识库导出");
    let temporary = destination.join(format!(".echo-memory-export-{}", Uuid::new_v4()));
    fs::create_dir(&temporary)?;
    let result = export_knowledge_base_into(library, project_id, unfiled_only, &temporary, format);
    if let Err(error) = result {
        let _ = fs::remove_dir_all(&temporary);
        return Err(error);
    }    if let Err(error) = fs::rename(&temporary, &folder_name) {
        let _ = fs::remove_dir_all(&temporary);
        return Err(error.into());
    }    Ok(folder_name)
}
fn export_knowledge_base_into(
    library: &ManagedLibrary,
    project_id: Option<&str>,
    unfiled_only: bool,
    destination: &Path,
    format: &str,
) -> AppResult<()> {
    let records = library
        .repository()
        .list_records(project_id, unfiled_only)?;
    let mut index = if format == "md" {
        "# 回声记忆知识库导出\n\n".to_owned()
    } else {
        "回声记忆知识库导出\n\n".to_owned()
    };
    for record in records {
        let file_name = format!(
            "{}-{}.{}",
            sanitize_file_name(&record.title),
            &record.id[..8.min(record.id.len())],
            format
        );
        let path = destination.join(&file_name);
        let content = render_record(library, &record.id, format)?;
        atomic_write(&path, content.as_bytes())?;
        if format == "md" {
            index.push_str(&format!("- [{}]({})\n", record.title, file_name));
        } else {
            index.push_str(&format!("- {}: {}\n", record.title, file_name));
        }    }    atomic_write(
        &destination.join(format!("索引.{format}")),
        index.as_bytes(),
    )?;
    Ok(())
}
fn available_export_folder(parent: &Path, base_name: &str) -> PathBuf {
    let direct = parent.join(base_name);
    if !direct.exists() {
        return direct;
    }    for suffix in 2..10_000 {
        let candidate = parent.join(format!("{base_name} {suffix}"));
        if !candidate.exists() {
            return candidate;
        }    }    parent.join(format!("{base_name}-{}", Uuid::new_v4()))
}
fn render_record(library: &ManagedLibrary, record_id: &str, format: &str) -> AppResult<String> {
    let repository = library.repository();
    let record = repository.get_record(record_id)?;
    let segments = repository.list_transcript_segments(record_id)?;
    // 块9：纪要与导出同源（同一份 content_json），保证「所见即所导」——
    // 面板在生成失败/重新生成时仍显示旧文档，导出同样带上它；
    // 只有空文档不渲染。SRT 只需逐字稿，跳过分析/纪要查询。
    Ok(if format == "srt" {
        render_srt(&segments)
    } else {
        let analysis = repository
            .latest_analysis(record_id)?
            .and_then(|stored| serde_json::from_str::<AnalysisDraft>(&stored.content_json).ok());
        let minutes = repository
            .get_minutes(record_id)?
            .filter(|row| row.content_json != "{}")
            .and_then(|row| {
                serde_json::from_str::<crate::minutes::MinutesDocument>(&row.content_json).ok()
            });
        if format == "md" {
            render_markdown(&record, &segments, analysis.as_ref(), minutes.as_ref())
        } else {
            render_text(&record, &segments, analysis.as_ref(), minutes.as_ref())
        }    })
}
fn render_markdown(
    record: &RecordBrief,
    segments: &[TranscriptSegment],
    analysis: Option<&AnalysisDraft>,
    minutes: Option<&crate::minutes::MinutesDocument>,
) -> String {
    let mut output = format!(
        "# {}\n\n- 日期：{}\n- 知识库：{}\n- 时长：{}\n\n",
        record.title,
        record.imported_at,
        record.project_name.as_deref().unwrap_or("未归档"),
        format_time(record.audio_duration_ms)
    );
    if let Some(minutes) = minutes {
        append_minutes_markdown(&mut output, minutes);
    }    if let Some(analysis) = analysis {
        output.push_str("## 摘要\n\n");
        output.push_str(&analysis.summary);
        output.push_str("\n\n");
        append_markdown_chapters(&mut output, &analysis.chapters);
        append_markdown_items(&mut output, "关键观点", &analysis.key_points);
        append_markdown_items(&mut output, "决策", &analysis.decisions);
        append_markdown_items(&mut output, "待办", &analysis.action_items);
        append_markdown_items(&mut output, "未解决问题", &analysis.open_questions);
        for section in &analysis.custom_sections {
            append_custom_markdown(&mut output, section);
        }        append_markdown_highlights(&mut output, &analysis.highlights);
    }    output.push_str("## 逐字稿\n\n");
    for block in build_blocks(segments) {
        output.push_str(&format!(
            "**{} · {}**  \n{}\n\n",
            format_time(block.start_ms),
            block.speaker_label.as_deref().unwrap_or("说话人未知"),
            block.text
        ));
    }    output
}
fn render_text(
    record: &RecordBrief,
    segments: &[TranscriptSegment],
    analysis: Option<&AnalysisDraft>,
    minutes: Option<&crate::minutes::MinutesDocument>,
) -> String {
    let markdown = render_markdown(record, segments, analysis, minutes);
    markdown
        .lines()
        .map(|line| {
            line.trim_start_matches('#')
                .trim_start()
                .replace("**", "")
                .replace("  ", "")
        })
        .collect::<Vec<_>>()
        .join("\n")
}
/// 纪要段（块9）：与纪要面板同一数据源，结论/待办逐条带时间戳。
fn append_minutes_markdown(output: &mut String, document: &crate::minutes::MinutesDocument) {
    if document.topics.is_empty()
        && document.conclusions.is_empty()
        && document.action_items.is_empty()
        && document.glossary.is_empty()
    {
        return;
    }    output.push_str("## 会议纪要\n\n");
    if !document.topics.is_empty() {
        output.push_str("### 议题\n\n");
        for topic in &document.topics {
            output.push_str(&format!(
                "- **{}**（{}）：{}\n",
                topic.title.trim(),
                topic
                    .start_ms
                    .map(format_time)
                    .unwrap_or_else(|| "—".to_owned()),
                topic.summary.trim()
            ));
        }        output.push('\n');
    }    for (title, items) in [
        ("结论", &document.conclusions),
        ("待办", &document.action_items),
    ] {
        if items.is_empty() {
            continue;
        }        output.push_str(&format!("### {title}\n\n"));
        for item in items {
            output.push_str(&format!(
                "- {}（{}）\n",
                item.text.trim(),
                format_time(item.evidence.start_ms)
            ));
            if !item.evidence.quote_text.trim().is_empty() {
                output.push_str(&format!(
                    "  - 出处：「{}」（{}）\n",
                    item.evidence.quote_text.trim(),
                    format_time(item.evidence.start_ms)
                ));
            }            if let Some(owner) = item.owner.as_deref().filter(|owner| !owner.trim().is_empty()) {
                output.push_str(&format!("  - 负责人：{owner}\n"));
            }        }        output.push('\n');
    }    if !document.glossary.is_empty() {
        output.push_str("### 术语表\n\n");
        for entry in &document.glossary {
            if entry.variants.is_empty() {
                output.push_str(&format!("- {}\n", entry.term));
            } else {
                output.push_str(&format!(
                    "- {}（变体：{}）\n",
                    entry.term,
                    entry.variants.join("、")
                ));
            }        }        output.push('\n');
    }}
/// SRT 字幕（块9）：segment 粒度，时间轴与逐字稿一致，说话人作前缀。
pub fn render_srt(segments: &[TranscriptSegment]) -> String {
    let mut output = String::new();
    for (index, segment) in segments.iter().enumerate() {
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
        output.push_str(&format!(
            "{}\n{} --> {}\n{}{}\n\n",
            index + 1,
            format_srt_time(segment.start_ms),
            format_srt_time(segment.end_ms),
            speaker,
            text.trim()
        ));
    }    output
}
/// SRT 时间格式：HH:MM:SS,mmm（毫秒用逗号分隔）。
pub fn format_srt_time(ms: i64) -> String {
    let total_ms = ms.max(0);
    let hours = total_ms / 3_600_000;
    let minutes = (total_ms % 3_600_000) / 60_000;
    let seconds = (total_ms % 60_000) / 1_000;
    let millis = total_ms % 1_000;
    format!("{hours:02}:{minutes:02}:{seconds:02},{millis:03}")
}
fn append_markdown_items(output: &mut String, title: &str, items: &[AnalysisItemDraft]) {
    output.push_str(&format!("## {title}\n\n"));
    if items.is_empty() {
        output.push_str("未识别出可靠内容。\n\n");
        return;
    }    for item in items {
        output.push_str(&format!("- {}\n", item.text));
        if !item.quote_text.is_empty() {
            output.push_str(&format!(
                "  - 引用：\"{}\"（{}）\n",
                item.quote_text,
                format_time(item.start_ms.unwrap_or(0))
            ));
        }    }    output.push('\n');
}
/// 章节是导航栏目：空章节不写「未识别出可靠内容」，直接省略。
fn append_markdown_chapters(output: &mut String, chapters: &[AnalysisChapterDraft]) {
    if chapters.is_empty() {
        return;
    }    output.push_str("## 章节\n\n");
    for chapter in chapters {
        output.push_str(&format!(
            "- **{}**（{}）",
            chapter.title.trim(),
            format_time(chapter.start_ms.unwrap_or(0))
        ));
        if !chapter.summary.trim().is_empty() {
            output.push_str(&format!("：{}", chapter.summary.trim()));
        }        output.push('\n');
    }    output.push('\n');
}
/// 金句同样是导航栏目，空则省略。
fn append_markdown_highlights(output: &mut String, highlights: &[AnalysisHighlightDraft]) {
    if highlights.is_empty() {
        return;
    }    output.push_str("## 金句\n\n");
    for highlight in highlights {
        output.push_str(&format!(
            "- “{}”（{}）\n",
            highlight.quote_text.trim(),
            format_time(highlight.start_ms.unwrap_or(0))
        ));
    }    output.push('\n');
}
fn append_custom_markdown(output: &mut String, section: &AnalysisCustomSectionDraft) {
    output.push_str(&format!("## {}\n\n", section.title));
    if section.format == "paragraph" {
        output.push_str(if section.text.trim().is_empty() {
            "未识别出可靠内容。"
        } else {
            section.text.trim()
        });
        output.push_str("\n\n");
    } else {
        append_markdown_items_body(output, &section.items);
    }}
fn append_markdown_items_body(output: &mut String, items: &[AnalysisItemDraft]) {
    if items.is_empty() {
        output.push_str("未识别出可靠内容。\n\n");
        return;
    }    for item in items {
        output.push_str(&format!("- {}", item.text));
        if !item.quote_text.is_empty() {
            output.push_str(&format!(
                "（引用：\"{}\"，{}）",
                item.quote_text,
                format_time(item.start_ms.unwrap_or(0))
            ));
        }        output.push('\n');
    }    output.push('\n');
}
pub fn atomic_write_public(destination: &Path, content: &[u8]) -> AppResult<()> {
    atomic_write(destination, content)
}
fn atomic_write(destination: &Path, content: &[u8]) -> AppResult<()> {
    let parent = destination
        .parent()
        .ok_or_else(|| AppError::Invalid("无法确定导出目录".to_owned()))?;
    fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(".echo-memory-{}.tmp", Uuid::new_v4()));
    if let Err(error) =
        fs::write(&temporary, content).and_then(|_| fs::rename(&temporary, destination))
    {
        let _ = fs::remove_file(&temporary);
        return Err(error.into());
    }    Ok(())
}
fn validate_format(format: &str) -> AppResult<()> {
    if matches!(format, "md" | "txt" | "srt") {
        Ok(())
    } else {
        Err(AppError::Invalid("仅支持 Markdown、TXT 或 SRT 导出".to_owned()))
    }}
fn sanitize_file_name(title: &str) -> String {
    let sanitized = title
        .chars()
        .map(|character| {
            if matches!(character, '/' | ':' | '\0') {
                '_'
            } else {
                character
            }        })
        .collect::<String>();
    let trimmed = sanitized.trim().trim_matches('.');
    if trimmed.is_empty() {
        "未命名录音".to_owned()
    } else {
        trimmed.chars().take(80).collect()
    }}
fn format_time(milliseconds: i64) -> String {
    let seconds = milliseconds.max(0) / 1_000;
    format!("{}:{:02}", seconds / 60, seconds % 60)
}
/// 块7：单条记录「导出包」——一个文件夹装下这次记录的全量交付：
/// 逐字稿（md + srt）、纪要（与面板同一份 content_json 渲染）、manifest（元数据 +
/// 纪要确认状态分布 + 所用模型 + 导出时间）。
///
/// 口径说明：
/// - 纪要与面板同源：同一份 minutes.content_json 渲染（块9 起的「所见即所导」延续）；
/// - 未确认条目在导出中不改变语义：照常渲染（既有导出口径），manifest 里如实报告
///   provisional/confirmed 分布，绝不悄悄变成「已确认」；
/// - 知识库 auto-export（dock.rs 分析完成自动落盘）是另一条单文件 md 流，
///   与导出包互不耦合、不同口径带上——导出包只由用户显式触发（本注释即排除声明；
///   auto-export 侧无导出包调用点，dock.rs 仅单文件 md 流）。
/// - 文件夹而非 zip：zip 依赖在本仓库只用于读 docx，没有现成的 zip 导出代码路径，
///   文件夹零新增依赖面且可直接打开（任务书允许的保守选择）。
pub fn export_record_package(
    library: &ManagedLibrary,
    record_id: &str,
    destination: &Path,
) -> AppResult<PathBuf> {
    let repository = library.repository();
    let record = repository.get_record(record_id)?;
    let segments = repository.list_transcript_segments(record_id)?;
    if segments.is_empty() {
        return Err(AppError::Invalid(
            "该记录还没有逐字稿，无法导出包".to_owned(),
        ));
    }    let minutes_row = repository.get_minutes(record_id)?;
    let minutes_document: Option<crate::minutes::MinutesDocument> = minutes_row
        .as_ref()
        .filter(|row| row.content_json != "{}")
        .and_then(|row| serde_json::from_str(&row.content_json).ok());
    let item_rows = repository.list_minutes_items(record_id)?;
    let confirmed_count = item_rows
        .iter()
        .filter(|item| item.status == "confirmed")
        .count();
    let provisional_count = item_rows.len() - confirmed_count;
    let alignment_offset_ms = repository.alignment_offset_for_record(record_id)?;

    // 逐字稿 md：与全文导出同一段渲染（说话人 + 时间块）。
    let mut transcript_md = format!("# {} · 逐字稿\n\n", record.title);
    for block in build_blocks(&segments) {
        transcript_md.push_str(&format!(
            "**{} · {}**  \n{}\n\n",
            format_time(block.start_ms),
            block.speaker_label.as_deref().unwrap_or("说话人未知"),
            block.text
        ));
    }
    // 纪要 md：与面板同一份 content_json 渲染（复用块9 的渲染器）。
    let minutes_md = minutes_document
        .as_ref()
        .map(|document| {
            let mut output = format!("# {} · 会议纪要\n\n", record.title);
            append_minutes_markdown(&mut output, document);
            output
        })
        .unwrap_or_else(|| "# 会议纪要\n\n该记录还没有生成纪要。\n".to_owned());

    let manifest = serde_json::json!({
        "record": {
            "id": record.id,
            "title": record.title,
            "sourceType": record.source_type,
            "importedAt": record.imported_at,
            "durationMs": record.audio_duration_ms,
            "audioHash": record.audio_hash,
            "projectId": record.project_id,
            "projectName": record.project_name,
            "archivedAt": record.archived_at,
            "importConfirmedAt": record.import_confirmed_at,
        },
        "alignmentOffsetMs": alignment_offset_ms,
        "minutes": {
            "status": minutes_row.as_ref().map(|row| row.status.as_str()),
            "provider": minutes_row.as_ref().and_then(|row| row.provider.clone()),
            "model": minutes_row.as_ref().and_then(|row| row.model.clone()),
            "droppedUnverifiable": minutes_document.as_ref().map(|document| document.dropped_unverifiable),
            "itemStatuses": { "confirmed": confirmed_count, "provisional": provisional_count },
        },
        "files": ["逐字稿.md", "逐字稿.srt", "纪要.md"],
        "exportedAt": chrono::Utc::now().to_rfc3339(),
    });

    fs::create_dir_all(destination)?;
    let folder = available_export_folder(destination, &format!("导出包-{}", record.title));
    let temporary = destination.join(format!(".echo-memory-package-{}", Uuid::new_v4()));
    fs::create_dir(&temporary)?;
    let write = || -> AppResult<()> {
        fs::write(temporary.join("逐字稿.md"), transcript_md)?;
        fs::write(temporary.join("逐字稿.srt"), render_srt(&segments))?;
        fs::write(temporary.join("纪要.md"), minutes_md)?;
        fs::write(
            temporary.join("manifest.json"),
            serde_json::to_string_pretty(&manifest)
                .map_err(|error| AppError::Invalid(format!("manifest 序列化失败：{error}")))?,
        )?;
        Ok(())
    };
    if let Err(error) = write() {
        let _ = fs::remove_dir_all(&temporary);
        return Err(error);
    }    fs::rename(&temporary, &folder)?;
    Ok(folder)
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::TranscriptSegmentInput;

    #[test]
    fn srt_time_format_and_segment_rendering() {
        assert_eq!(format_srt_time(0), "00:00:00,000");
        assert_eq!(format_srt_time(62_500), "00:01:02,500");
        assert_eq!(format_srt_time(3_723_000), "01:02:03,000");
        assert_eq!(format_srt_time(-5), "00:00:00,000", "负值夹紧为零");
        let segments = vec![
            TranscriptSegment {
                id: "s1".into(),
                record_id: "r".into(),
                sequence: 0,
                speaker_label: Some("客户".into()),
                start_ms: 62_000,
                end_ms: 65_000,
                original_text: "希望资料只保存在本机".into(),
                normalized_text: None,
                normalization_version: None,
                edited_text: None,
            },
            TranscriptSegment {
                id: "s2".into(),
                record_id: "r".into(),
                sequence: 1,
                speaker_label: None,
                start_ms: 0,
                end_ms: 1_000,
                original_text: "好的".into(),
                normalized_text: None,
                normalization_version: None,
                edited_text: None,
            },
        ];
        let srt = render_srt(&segments);
        assert!(srt.starts_with("1\n00:01:02,000 --> 00:01:05,000\n[客户] 希望资料只保存在本机\n\n"));
        assert!(srt.contains("2\n00:00:00,000 --> 00:00:01,000\n好的"), "无说话人时不加前缀");
        // 序号连续、时间轴与逐字稿一致（乱序输入按原序导出）。
        assert_eq!(srt.matches("-->").count(), 2);
    }
    #[test]
    fn markdown_export_includes_minutes_with_matching_timestamps() {
        let root = std::env::temp_dir().join(format!("echo-export-minutes-{}", Uuid::new_v4()));
        let library = ManagedLibrary::open(&root).unwrap();
        let repository = library.repository();
        let record = repository
            .create_record(
                "复盘会",
                None,
                Path::new("audio/review.m4a"),
                "minutes-hash",
                12_000,
            )
            .unwrap();
        repository
            .save_transcript(
                &record.id,
                "whisper.cpp",
                "small",
                &[TranscriptSegmentInput {
                    start_ms: 0,
                    end_ms: 6_000,
                    speaker_label: Some("说话人 1".into()),
                    original_text: "我们把第三季度的产品复盘安排在下周三下午两点".into(),
                }],
            )
            .unwrap();
        repository.ensure_minutes_pending(&record.id).unwrap();
        let document = crate::minutes::MinutesDocument {
            topics: vec![crate::minutes::MinutesTopic {
                id: "t1".into(),
                title: "复盘安排".into(),
                summary: "定在下周三".into(),
                start_ms: Some(0),
                end_ms: Some(6_000),
            }],
            conclusions: vec![crate::minutes::MinutesItem {
                id: "c1".into(),
                kind: crate::minutes::MinutesItemKind::Conclusion,
                text: "复盘时间确定".into(),
                owner: Some("老张".into()),
                evidence: crate::minutes::MinutesEvidence {
                    start_ms: 0,
                    end_ms: 6_000,
                    quote_text: "我们把第三季度的产品复盘安排在下周三下午两点".into(),
                },
            }],
            action_items: vec![],
            glossary: vec![crate::minutes::GlossaryEntry {
                term: "灰度".into(),
                variants: vec!["灰度发布".into()],
            }],
            dropped_unverifiable: 0,
        };
        repository
            .save_minutes_document(&record.id, "fake", "fake", &document)
            .unwrap();

        let markdown = render_record(&library, &record.id, "md").unwrap();
        assert!(markdown.contains("## 会议纪要"));
        assert!(markdown.contains("- **复盘安排**（0:00）：定在下周三"));
        assert!(markdown.contains("- 复盘时间确定（0:00）"));
        assert!(markdown.contains("「我们把第三季度的产品复盘安排在下周三下午两点」"), "结论出处与逐字稿一致");
        assert!(markdown.contains("负责人：老张"));
        assert!(markdown.contains("- 灰度（变体：灰度发布）"));

        // SRT 导出：文件可写、内容与逐字稿一致。
        let destination = root.join("out.srt");
        let path = export_record(&library, &record.id, &destination, "srt").unwrap();
        assert!(path.is_file());
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("00:00:00,000 --> 00:00:06,000"));
        assert!(content.contains("[说话人 1] 我们把第三季度的产品复盘安排在下周三下午两点"));
        assert!(!content.contains("会议纪要"), "SRT 只含逐字稿，不含纪要段");

        // failed 态：面板仍显示旧文档，导出同样带上（所见即所导）。
        repository.fail_minutes(&record.id, "模拟失败").unwrap();
        let markdown_after_fail = render_record(&library, &record.id, "md").unwrap();
        assert!(markdown_after_fail.contains("## 会议纪要"), "failed 保留旧文档时导出与面板一致");
        let _ = std::fs::remove_dir_all(root);
    }
    #[test]
    fn minutes_markdown_covers_action_items_and_topic_fallback() {
        // 待办渲染（含负责人与出处）与议题无时间戳的 "—" 兜底。
        let document = crate::minutes::MinutesDocument {
            topics: vec![crate::minutes::MinutesTopic {
                id: "t".into(),
                title: "无时间议题".into(),
                summary: String::new(),
                start_ms: None,
                end_ms: None,
            }],
            conclusions: vec![],
            action_items: vec![crate::minutes::MinutesItem {
                id: "a".into(),
                kind: crate::minutes::MinutesItemKind::Action,
                text: "发纪要".into(),
                owner: Some("小李".into()),
                evidence: crate::minutes::MinutesEvidence {
                    start_ms: 65_000,
                    end_ms: 66_000,
                    quote_text: "纪要我来发".into(),
                },
            }],
            glossary: vec![],
            dropped_unverifiable: 0,
        };
        let mut output = String::new();
        append_minutes_markdown(&mut output, &document);
        assert!(output.contains("- **无时间议题**（—）"), "无时间戳议题用 — 兜底");
        assert!(output.contains("### 待办"));
        assert!(output.contains("- 发纪要（1:05）"));
        assert!(output.contains("出处：「纪要我来发」"));
        assert!(output.contains("负责人：小李"));
    }
    #[test]
    fn file_names_are_sanitized() {
        assert_eq!(sanitize_file_name("客户/访谈:一"), "客户_访谈_一");
        assert_eq!(sanitize_file_name("..."), "未命名录音");
    }
    #[test]
    fn exports_chinese_transcript_analysis_and_citations() {
        let root = std::env::temp_dir().join(format!("echo-export-test-{}", Uuid::new_v4()));
        let library = ManagedLibrary::open(&root).unwrap();
        let record = library
            .repository()
            .create_record(
                "客户访谈",
                None,
                Path::new("audio/interview.m4a"),
                "export-hash",
                65_000,
            )
            .unwrap();
        let (version, segments) = library
            .repository()
            .save_transcript(
                &record.id,
                "whisper.cpp",
                "small",
                &[TranscriptSegmentInput {
                    start_ms: 62_000,
                    end_ms: 65_000,
                    speaker_label: Some("客户".into()),
                    original_text: "希望资料只保存在本机".into(),
                }],
            )
            .unwrap();
        library
            .repository()
            .save_analysis(
                &record.id,
                &version.id,
                "qwen",
                &AnalysisDraft {
                    summary: "客户重视本地隐私。".into(),
                    key_points: vec![AnalysisItemDraft {
                        owner: None,
                        text: "本地保存是核心需求".into(),
                        citation_segment_ids: vec![segments[0].id.clone()],
                        quote_text: "希望资料只保存在本机".into(),
                        start_ms: Some(62_000),
                        end_ms: Some(65_000),
                    }],
                    decisions: vec![],
                    action_items: vec![],
                    open_questions: vec![],
                    custom_sections: vec![AnalysisCustomSectionDraft {
                        key: "needs".into(),
                        title: "核心需求".into(),
                        format: "paragraph".into(),
                        text: "保持纯本地处理。".into(),
                        items: vec![],
                    }],
                    chapters: vec![],
                    highlights: vec![],
                    quality_warning: None,
                    quality_severity: None,
                },
            )
            .unwrap();

        let markdown_path = root.join("exports/客户访谈.md");
        export_record(&library, &record.id, &markdown_path, "md").unwrap();
        let markdown = fs::read_to_string(&markdown_path).unwrap();
        assert!(markdown.contains("客户重视本地隐私"));
        assert!(markdown.contains("希望资料只保存在本机"));
        assert!(markdown.contains("1:02"));
        assert!(markdown.contains("核心需求"));

        let export_parent = root.join("folder-exports");
        let folder = export_knowledge_base(&library, None, false, &export_parent, "txt").unwrap();
        assert_eq!(folder.parent(), Some(export_parent.as_path()));
        assert!(folder.join("索引.txt").is_file());
        assert!(!fs::read_dir(&export_parent)
            .unwrap()
            .filter_map(Result::ok)
            .any(|entry| entry
                .file_name()
                .to_string_lossy()
                .starts_with(".echo-memory-export-")));
        let _ = fs::remove_dir_all(root);
    }}

#[cfg(test)]
mod bulk_export_tests {
    use super::*;
    use crate::types::TranscriptSegmentInput;
    use uuid::Uuid;

    fn seeded_record(repository: &crate::db::repository::LibraryRepository, title: &str) -> crate::types::RecordBrief {
        let dir = std::env::temp_dir().join(format!("echo-bulk-export-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("sample.wav");
        std::fs::write(&path, [0_u8; 2048]).unwrap();
        let hash = title.bytes().fold(0xcbf29ce484222325_u64, |acc, b| (acc ^ b as u64).wrapping_mul(0x100000001b3));
        repository
            .create_record(title, None, &path, &format!("{hash:x}"), 12_000)
            .unwrap()
    }
    fn with_transcript(repository: &crate::db::repository::LibraryRepository, record_id: &str) {
        repository
            .save_transcript(
                record_id,
                "whisper.cpp",
                "small",
                &[TranscriptSegmentInput {
                    start_ms: 0,
                    end_ms: 6_000,
                    speaker_label: Some("说话人 1".into()),
                    original_text: "我们把第三季度的产品复盘安排在下周三下午两点".into(),
                }],
            )
            .unwrap();
    }
    /// v1.4 包7：批量导出的核心语义——逐条子文件夹、单条失败不中断、如实回报。
    #[test]
    fn bulk_export_creates_per_record_folders_and_reports_skips() {
        let root = std::env::temp_dir().join(format!("echo-bulk-export-lib-{}", Uuid::new_v4()));
        let library = ManagedLibrary::open(&root).unwrap();
        let repository = library.repository();
        let r1 = seeded_record(repository, "会议甲");
        let r2 = seeded_record(repository, "会议乙");
        let r3 = seeded_record(repository, "会议丙");
        with_transcript(repository, &r1.id);
        with_transcript(repository, &r2.id);
        // r3 没有逐字稿：应被跳过而不影响其他两条

        let destination = root.join("out");
        std::fs::create_dir_all(&destination).unwrap();

        // 直接调批量逻辑（命令层仅加票据门，编排在 export 侧）
        let mut exported = Vec::new();
        let mut skipped = Vec::new();
        for record_id in [&r1.id, &r2.id, &r3.id] {
            match export_record_package(&library, record_id, &destination) {
                Ok(folder) => exported.push(folder),
                Err(error) => skipped.push((record_id.clone(), error.to_string())),
            }        }        assert_eq!(exported.len(), 2, "两条有逐字稿的记录应导出");
        assert_eq!(skipped.len(), 1, "没有逐字稿的记录应被跳过");
        assert!(skipped[0].1.contains("逐字稿"), "跳过原因要说明：{}", skipped[0].1);
        // 每条记录独立子文件夹
        assert_ne!(exported[0], exported[1]);
        for folder in &exported {
            assert!(folder.join("逐字稿.md").is_file());
            assert!(folder.join("manifest.json").is_file());
        }        let _ = std::fs::remove_dir_all(root);
    }}