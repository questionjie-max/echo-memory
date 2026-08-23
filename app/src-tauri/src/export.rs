use crate::analysis::{AnalysisCustomSectionDraft, AnalysisDraft, AnalysisItemDraft};
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
    }
    if let Err(error) = fs::rename(&temporary, &folder_name) {
        let _ = fs::remove_dir_all(&temporary);
        return Err(error.into());
    }
    Ok(folder_name)
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
        }
    }
    atomic_write(
        &destination.join(format!("索引.{format}")),
        index.as_bytes(),
    )?;
    Ok(())
}

fn available_export_folder(parent: &Path, base_name: &str) -> PathBuf {
    let direct = parent.join(base_name);
    if !direct.exists() {
        return direct;
    }
    for suffix in 2..10_000 {
        let candidate = parent.join(format!("{base_name} {suffix}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    parent.join(format!("{base_name}-{}", Uuid::new_v4()))
}

fn render_record(library: &ManagedLibrary, record_id: &str, format: &str) -> AppResult<String> {
    let repository = library.repository();
    let record = repository.get_record(record_id)?;
    let segments = repository.list_transcript_segments(record_id)?;
    let analysis = repository
        .latest_analysis(record_id)?
        .and_then(|stored| serde_json::from_str::<AnalysisDraft>(&stored.content_json).ok());
    Ok(if format == "md" {
        render_markdown(&record, &segments, analysis.as_ref())
    } else {
        render_text(&record, &segments, analysis.as_ref())
    })
}

fn render_markdown(
    record: &RecordBrief,
    segments: &[TranscriptSegment],
    analysis: Option<&AnalysisDraft>,
) -> String {
    let mut output = format!(
        "# {}\n\n- 日期：{}\n- 知识库：{}\n- 时长：{}\n\n",
        record.title,
        record.imported_at,
        record.project_name.as_deref().unwrap_or("未归档"),
        format_time(record.audio_duration_ms)
    );
    if let Some(analysis) = analysis {
        output.push_str("## 摘要\n\n");
        output.push_str(&analysis.summary);
        output.push_str("\n\n");
        append_markdown_items(&mut output, "关键观点", &analysis.key_points);
        append_markdown_items(&mut output, "决策", &analysis.decisions);
        append_markdown_items(&mut output, "待办", &analysis.action_items);
        append_markdown_items(&mut output, "未解决问题", &analysis.open_questions);
        for section in &analysis.custom_sections {
            append_custom_markdown(&mut output, section);
        }
    }
    output.push_str("## 逐字稿\n\n");
    for block in build_blocks(segments) {
        output.push_str(&format!(
            "**{} · {}**  \n{}\n\n",
            format_time(block.start_ms),
            block.speaker_label.as_deref().unwrap_or("说话人未知"),
            block.text
        ));
    }
    output
}

fn render_text(
    record: &RecordBrief,
    segments: &[TranscriptSegment],
    analysis: Option<&AnalysisDraft>,
) -> String {
    let markdown = render_markdown(record, segments, analysis);
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

fn append_markdown_items(output: &mut String, title: &str, items: &[AnalysisItemDraft]) {
    output.push_str(&format!("## {title}\n\n"));
    if items.is_empty() {
        output.push_str("未识别出可靠内容。\n\n");
        return;
    }
    for item in items {
        output.push_str(&format!("- {}\n", item.text));
        if !item.quote_text.is_empty() {
            output.push_str(&format!(
                "  - 引用：\"{}\"（{}）\n",
                item.quote_text,
                format_time(item.start_ms.unwrap_or(0))
            ));
        }
    }
    output.push('\n');
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
    }
}

fn append_markdown_items_body(output: &mut String, items: &[AnalysisItemDraft]) {
    if items.is_empty() {
        output.push_str("未识别出可靠内容。\n\n");
        return;
    }
    for item in items {
        output.push_str(&format!("- {}", item.text));
        if !item.quote_text.is_empty() {
            output.push_str(&format!(
                "（引用：\"{}\"，{}）",
                item.quote_text,
                format_time(item.start_ms.unwrap_or(0))
            ));
        }
        output.push('\n');
    }
    output.push('\n');
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
    }
    Ok(())
}

fn validate_format(format: &str) -> AppResult<()> {
    if matches!(format, "md" | "txt") {
        Ok(())
    } else {
        Err(AppError::Invalid("仅支持 Markdown 或 TXT 导出".to_owned()))
    }
}

fn sanitize_file_name(title: &str) -> String {
    let sanitized = title
        .chars()
        .map(|character| {
            if matches!(character, '/' | ':' | '\0') {
                '_'
            } else {
                character
            }
        })
        .collect::<String>();
    let trimmed = sanitized.trim().trim_matches('.');
    if trimmed.is_empty() {
        "未命名录音".to_owned()
    } else {
        trimmed.chars().take(80).collect()
    }
}

fn format_time(milliseconds: i64) -> String {
    let seconds = milliseconds.max(0) / 1_000;
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::TranscriptSegmentInput;

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
                    quality_warning: None,
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
    }
}
