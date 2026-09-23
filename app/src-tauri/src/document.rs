//! Managed document import. Parsing and persistence are entirely local.

use crate::error::{AppError, AppResult};
use crate::library::ManagedLibrary;
use crate::types::{IngestResult, RecordBrief, TranscriptSegmentInput};
use chrono::{Datelike, Utc};
use pulldown_cmark::{Event as MarkdownEvent, Parser, TagEnd};
use quick_xml::events::Event as XmlEvent;
use quick_xml::Reader;
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use uuid::Uuid;
use zip::ZipArchive;

const DOCUMENT_EXTENSIONS: [&str; 4] = ["md", "markdown", "txt", "docx"];
const SEGMENT_CHAR_LIMIT: usize = 800;
const MAX_SOURCE_BYTES: u64 = 20 * 1024 * 1024;
const MAX_DOCX_XML_BYTES: u64 = 16 * 1024 * 1024;
const MAX_TEXT_CHARACTERS: usize = 2_000_000;

pub fn import_document(
    library: &ManagedLibrary,
    source: &Path,
    project_id: Option<&str>,
    duplicate_confirmed: bool,
) -> AppResult<IngestResult> {
    let extension = supported_extension(source)?;
    validate_source_file(source)?;
    let hash = sha256_file(source)?;
    if let Some(existing) = library
        .repository()
        .find_record_by_source_hash("document", &hash, project_id)?
    {
        if !duplicate_confirmed {
            return Ok(duplicate_result(existing));
        }
    }

    let text = match extension.as_str() {
        "txt" => parse_text_file(source)?,
        "md" | "markdown" => parse_markdown(source)?,
        "docx" => parse_docx(source)?,
        _ => unreachable!(),
    };
    let text = normalize_text(&text);
    if text.is_empty() {
        return Err(AppError::Import("文档没有可导入的文字内容".into()));
    }
    if text.chars().count() > MAX_TEXT_CHARACTERS {
        return Err(AppError::Import("文档文字内容过长，请拆分后再导入".into()));
    }
    let segments = build_segments(&text);
    let title = source
        .file_stem()
        .and_then(|name| name.to_str())
        .filter(|name| !name.trim().is_empty())
        .ok_or_else(|| AppError::Import("文件名无效".into()))?;

    let now = Utc::now();
    let record_id = Uuid::new_v4().to_string();
    let relative_path = PathBuf::from("documents")
        .join(format!("{:04}", now.year()))
        .join(format!("{:02}", now.month()))
        .join(format!("{record_id}.{extension}"));
    let destination = library.root().join(&relative_path);
    let parent = destination
        .parent()
        .ok_or_else(|| AppError::Import("无法确定受管理目录".into()))?;
    fs::create_dir_all(parent)?;
    fs::copy(source, &destination)?;

    let result = library.repository().create_document_record(
        &record_id,
        title,
        project_id,
        &relative_path,
        &hash,
        &extension,
        &segments,
    );
    let record = match result {
        Ok(record) => record,
        Err(error) => {
            let _ = fs::remove_file(destination);
            return Err(error);
        }
    };

    if let Ok(settings) = library.repository().knowledge_settings() {
        let _ = library
            .repository()
            .refresh_knowledge_index_counts(&settings.embedding_model);
    }
    Ok(IngestResult {
        record_id: record.id,
        hash,
        duplicate: false,
        duration_ms: 0,
        title: record.title,
    })
}

fn duplicate_result(record: RecordBrief) -> IngestResult {
    IngestResult {
        record_id: record.id,
        hash: record.audio_hash,
        duplicate: true,
        duration_ms: 0,
        title: record.title,
    }
}

fn supported_extension(path: &Path) -> AppResult<String> {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| AppError::Import("仅支持 Markdown、TXT、Word (.docx) 文档".into()))?;
    if DOCUMENT_EXTENSIONS.contains(&extension.as_str()) {
        Ok(extension)
    } else {
        Err(AppError::Import(
            "仅支持 Markdown、TXT、Word (.docx) 文档".into(),
        ))
    }
}

fn validate_source_file(path: &Path) -> AppResult<()> {
    let metadata = path.metadata()?;
    if !metadata.is_file() {
        return Err(AppError::Import("请选择一个有效的文档文件".into()));
    }
    if metadata.len() > MAX_SOURCE_BYTES {
        return Err(AppError::Import(
            "文档不能超过 20 MB，请拆分后再导入".into(),
        ));
    }
    Ok(())
}

fn read_limited(path: &Path, max_bytes: u64) -> AppResult<Vec<u8>> {
    let mut bytes = Vec::new();
    File::open(path)?
        .take(max_bytes + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max_bytes {
        return Err(AppError::Import("文档过大，请拆分后再导入".into()));
    }
    Ok(bytes)
}

fn parse_text_file(path: &Path) -> AppResult<String> {
    decode_text(read_limited(path, MAX_SOURCE_BYTES)?)
}

fn decode_text(bytes: Vec<u8>) -> AppResult<String> {
    if let Some(content) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8(content.to_vec())
            .map_err(|_| AppError::Import("TXT/Markdown 文档编码无效".into()));
    }
    if let Some(content) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return decode_utf16(content, true);
    }
    if let Some(content) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return decode_utf16(content, false);
    }
    String::from_utf8(bytes).map_err(|_| {
        AppError::Import("TXT/Markdown 文档需使用 UTF-8，或带 BOM 的 UTF-16 编码".into())
    })
}

fn decode_utf16(bytes: &[u8], little_endian: bool) -> AppResult<String> {
    if !bytes.len().is_multiple_of(2) {
        return Err(AppError::Import("UTF-16 文档字节长度无效".into()));
    }
    let units = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| {
            if little_endian {
                u16::from_le_bytes(*pair)
            } else {
                u16::from_be_bytes(*pair)
            }
        })
        .collect::<Vec<_>>();
    char::decode_utf16(units)
        .collect::<Result<String, _>>()
        .map_err(|_| AppError::Import("UTF-16 文档包含无效字符".into()))
}

fn parse_markdown(path: &Path) -> AppResult<String> {
    let markdown = parse_text_file(path)?;
    let mut text = String::new();
    for event in Parser::new(&markdown) {
        match event {
            MarkdownEvent::Text(value) | MarkdownEvent::Code(value) => text.push_str(&value),
            MarkdownEvent::Html(value) | MarkdownEvent::InlineHtml(value) => {
                text.push_str(&html_fragment_text(&value));
            }
            MarkdownEvent::SoftBreak | MarkdownEvent::HardBreak => text.push('\n'),
            MarkdownEvent::End(
                TagEnd::Paragraph
                | TagEnd::Heading(_)
                | TagEnd::Item
                | TagEnd::CodeBlock
                | TagEnd::BlockQuote(_),
            ) => text.push('\n'),
            _ => {}
        }
    }
    Ok(text)
}

fn html_fragment_text(fragment: &str) -> String {
    let mut output = String::new();
    let mut tag = String::new();
    let mut inside_tag = false;
    for character in fragment.chars() {
        match character {
            '<' if !inside_tag => {
                inside_tag = true;
                tag.clear();
            }
            '>' if inside_tag => {
                inside_tag = false;
                let name = tag
                    .trim()
                    .trim_start_matches('/')
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .trim_end_matches('/')
                    .to_ascii_lowercase();
                if matches!(
                    name.as_str(),
                    "br" | "p" | "div" | "li" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6"
                ) && !output.ends_with('\n')
                {
                    output.push('\n');
                }
            }
            _ if inside_tag => tag.push(character),
            _ => output.push(character),
        }
    }
    decode_common_entities(&output)
}

fn decode_common_entities(text: &str) -> String {
    text.replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
}

fn parse_docx(path: &Path) -> AppResult<String> {
    let file = File::open(path)?;
    let mut archive = ZipArchive::new(file)
        .map_err(|error| AppError::Import(format!("无法打开 Word 文档: {error}")))?;
    let document = archive
        .by_name("word/document.xml")
        .map_err(|_| AppError::Import("Word 文档缺少 word/document.xml".into()))?;
    if document.size() > MAX_DOCX_XML_BYTES {
        return Err(AppError::Import("Word 文档正文过大，请拆分后再导入".into()));
    }
    let mut xml_bytes = Vec::new();
    document
        .take(MAX_DOCX_XML_BYTES + 1)
        .read_to_end(&mut xml_bytes)
        .map_err(|error| AppError::Import(format!("无法读取 Word 文档: {error}")))?;
    if xml_bytes.len() as u64 > MAX_DOCX_XML_BYTES {
        return Err(AppError::Import("Word 文档正文过大，请拆分后再导入".into()));
    }
    let xml = String::from_utf8(xml_bytes)
        .map_err(|_| AppError::Import("Word 文档正文编码无效".into()))?;

    let mut reader = Reader::from_str(&xml);
    reader.config_mut().trim_text(false);
    let mut text = String::new();
    let mut in_text_node = false;
    loop {
        match reader.read_event() {
            Ok(XmlEvent::Start(element)) if element.local_name().as_ref() == b"t" => {
                in_text_node = true;
            }
            Ok(XmlEvent::End(element)) if element.local_name().as_ref() == b"t" => {
                in_text_node = false;
            }
            Ok(XmlEvent::Text(value)) if in_text_node => {
                let value = value
                    .decode()
                    .map_err(|error| AppError::Import(format!("Word 文档文字无效: {error}")))?;
                text.push_str(&value);
            }
            Ok(XmlEvent::GeneralRef(reference)) if in_text_node => {
                let reference = reference
                    .decode()
                    .map_err(|error| AppError::Import(format!("Word 文档实体无效: {error}")))?;
                match reference.as_ref() {
                    "amp" => text.push('&'),
                    "lt" => text.push('<'),
                    "gt" => text.push('>'),
                    "quot" => text.push('"'),
                    "apos" => text.push('\''),
                    value if value.starts_with("#x") => {
                        if let Ok(code) = u32::from_str_radix(&value[2..], 16) {
                            if let Some(character) = char::from_u32(code) {
                                text.push(character);
                            }
                        }
                    }
                    value if value.starts_with('#') => {
                        if let Ok(code) = value[1..].parse::<u32>() {
                            if let Some(character) = char::from_u32(code) {
                                text.push(character);
                            }
                        }
                    }
                    _ => {}
                }
            }
            Ok(XmlEvent::Empty(element)) => match element.local_name().as_ref() {
                b"tab" => text.push('\t'),
                b"br" | b"cr" => text.push('\n'),
                _ => {}
            },
            Ok(XmlEvent::End(element)) if element.local_name().as_ref() == b"p" => text.push('\n'),
            Ok(XmlEvent::Eof) => break,
            Err(error) => return Err(AppError::Import(format!("无法解析 Word 文档内容: {error}"))),
            _ => {}
        }
    }
    Ok(text)
}

fn normalize_text(text: &str) -> String {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut lines = Vec::new();
    let mut previous_blank = true;
    for line in normalized.lines() {
        let line = line.trim();
        if line.is_empty() {
            if !previous_blank {
                lines.push(String::new());
            }
            previous_blank = true;
        } else {
            lines.push(line.to_owned());
            previous_blank = false;
        }
    }
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    lines.join("\n")
}

fn build_segments(text: &str) -> Vec<TranscriptSegmentInput> {
    let mut chunks = Vec::new();
    let mut current = String::new();
    for paragraph in text.split("\n\n").filter(|part| !part.trim().is_empty()) {
        let paragraph_len = paragraph.chars().count();
        let separator_len = usize::from(!current.is_empty()) * 2;
        if !current.is_empty()
            && current.chars().count() + separator_len + paragraph_len > SEGMENT_CHAR_LIMIT
        {
            chunks.push(std::mem::take(&mut current));
        }
        if paragraph_len > SEGMENT_CHAR_LIMIT {
            if !current.is_empty() {
                chunks.push(std::mem::take(&mut current));
            }
            let chars = paragraph.chars().collect::<Vec<_>>();
            for part in chars.chunks(SEGMENT_CHAR_LIMIT) {
                chunks.push(part.iter().collect());
            }
        } else {
            if !current.is_empty() {
                current.push_str("\n\n");
            }
            current.push_str(paragraph);
        }
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
        .into_iter()
        .enumerate()
        .map(|(sequence, original_text)| TranscriptSegmentInput {
            start_ms: sequence as i64 * 1_000,
            end_ms: (sequence as i64 + 1) * 1_000,
            speaker_label: None,
            original_text,
        })
        .collect()
}

fn sha256_file(path: &Path) -> AppResult<String> {
    let mut reader = BufReader::new(File::open(path)?);
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    let mut total = 0_u64;
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        total += count as u64;
        if total > MAX_SOURCE_BYTES {
            return Err(AppError::Import(
                "文档不能超过 20 MB，请拆分后再导入".into(),
            ));
        }
        hasher.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}
