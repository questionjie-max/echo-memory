use echo_memory_lib::document::import_document;
use echo_memory_lib::library::ManagedLibrary;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn temp_root(name: &str) -> PathBuf {
    let root = std::env::temp_dir()
        .join("echo_memory_document_import_tests")
        .join(format!("{name}_{}", COUNTER.fetch_add(1, Ordering::SeqCst)));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

fn write_docx(path: &Path) {
    let file = File::create(path).unwrap();
    let mut archive = ZipWriter::new(file);
    archive
        .start_file("word/document.xml", SimpleFileOptions::default())
        .unwrap();
    archive
        .write_all(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>
<w:p><w:r><w:t>第一段 &amp; 结论</w:t></w:r></w:p>
<w:p><w:r><w:t>第二段会议内容</w:t><w:instrText>字段指令不应导入</w:instrText><w:delText>已删除内容不应导入</w:delText></w:r></w:p>
</w:body></w:document>"#
                .as_bytes(),
        )
        .unwrap();
    archive.finish().unwrap();
}

#[test]
fn txt_import_creates_completed_searchable_document_record() {
    let root = temp_root("txt");
    let source = root.join("产品会议.txt");
    fs::write(
        &source,
        "\u{feff}会议结论：下周发布新版本。\n\n负责人：小王。\n",
    )
    .unwrap();
    let library = ManagedLibrary::open(root.join("library")).unwrap();

    let result = import_document(&library, &source, None, false).unwrap();
    assert!(!result.duplicate);
    assert_eq!(result.duration_ms, 0);

    let record = library.repository().get_record(&result.record_id).unwrap();
    assert_eq!(record.source_type, "document");
    assert_eq!(record.status, "completed");
    assert!(record.has_transcript);
    assert_eq!(
        serde_json::to_value(&record).unwrap()["sourceType"],
        "document"
    );
    let version = library
        .repository()
        .latest_transcript_version(&record.id)
        .unwrap();
    assert_eq!(version.status, "completed");
    assert_eq!(version.provider, "document-import");
    assert!(library
        .repository()
        .search("下周发布", None, false, 10)
        .unwrap()
        .iter()
        .any(|item| item.record_id == record.id));
}

#[test]
fn duplicate_requires_confirmation_before_creating_another_record() {
    let root = temp_root("duplicate");
    let source = root.join("same.md");
    fs::write(&source, "# 同一份文档\n\n需要确认重复导入。\n").unwrap();
    let library = ManagedLibrary::open(root.join("library")).unwrap();

    let first = import_document(&library, &source, None, false).unwrap();
    let duplicate = import_document(&library, &source, None, false).unwrap();
    assert!(duplicate.duplicate);
    assert_eq!(duplicate.record_id, first.record_id);

    let confirmed = import_document(&library, &source, None, true).unwrap();
    assert!(!confirmed.duplicate);
    assert_ne!(confirmed.record_id, first.record_id);
    assert_eq!(
        library
            .repository()
            .list_records(None, false)
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn markdown_and_docx_are_converted_to_plain_text() {
    let root = temp_root("formats");
    let markdown = root.join("notes.markdown");
    fs::write(&markdown, "# 标题\n\n这是 **重点**，包含 `代码`。\n").unwrap();
    let docx = root.join("meeting.docx");
    write_docx(&docx);
    let library = ManagedLibrary::open(root.join("library")).unwrap();

    let md = import_document(&library, &markdown, None, false).unwrap();
    let md_text = library
        .repository()
        .list_transcript_segments(&md.record_id)
        .unwrap()
        .into_iter()
        .map(|segment| segment.original_text)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(md_text.contains("标题"));
    assert!(md_text.contains("这是 重点，包含 代码。"));
    assert!(!md_text.contains("**"));

    let word = import_document(&library, &docx, None, false).unwrap();
    let word_text = library
        .repository()
        .list_transcript_segments(&word.record_id)
        .unwrap()
        .into_iter()
        .map(|segment| segment.original_text)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(word_text.contains("第一段 & 结论"));
    assert!(word_text.contains("第二段会议内容"));
    assert!(!word_text.contains("字段指令不应导入"));
    assert!(!word_text.contains("已删除内容不应导入"));
}

#[test]
fn rejects_unsupported_and_empty_documents() {
    let root = temp_root("invalid");
    let library = ManagedLibrary::open(root.join("library")).unwrap();
    let pdf = root.join("note.pdf");
    fs::write(&pdf, "not a document").unwrap();
    assert!(import_document(&library, &pdf, None, false).is_err());

    let empty = root.join("empty.txt");
    fs::write(&empty, " \n\n ").unwrap();
    assert!(import_document(&library, &empty, None, false).is_err());
}

#[test]
fn utf16_text_and_markdown_html_are_preserved_as_readable_text() {
    let root = temp_root("encoding_and_html");
    let utf16 = root.join("utf16.txt");
    let mut bytes = vec![0xFF, 0xFE];
    for unit in "UTF-16 会议结论：继续推进。".encode_utf16() {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    fs::write(&utf16, bytes).unwrap();
    let markdown = root.join("html.md");
    fs::write(
        &markdown,
        "# HTML 记录\n\n<div>正文 &amp; 补充内容</div>\n\n普通段落。\n",
    )
    .unwrap();
    let library = ManagedLibrary::open(root.join("library")).unwrap();

    let utf16_result = import_document(&library, &utf16, None, false).unwrap();
    let utf16_text = library
        .repository()
        .list_transcript_segments(&utf16_result.record_id)
        .unwrap()
        .into_iter()
        .map(|segment| segment.original_text)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(utf16_text.contains("UTF-16 会议结论：继续推进。"));

    let markdown_result = import_document(&library, &markdown, None, false).unwrap();
    let markdown_text = library
        .repository()
        .list_transcript_segments(&markdown_result.record_id)
        .unwrap()
        .into_iter()
        .map(|segment| segment.original_text)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(markdown_text.contains("正文 & 补充内容"));
    assert!(markdown_text.contains("普通段落。"));
    assert!(!markdown_text.contains("<div>"));
}

#[test]
fn duplicate_detection_is_scoped_to_the_target_knowledge_base() {
    let root = temp_root("duplicate_scope");
    let source = root.join("shared.txt");
    fs::write(&source, "同一份文件可以分别放入不同知识库。\n").unwrap();
    let library = ManagedLibrary::open(root.join("library")).unwrap();
    let project_a = library.repository().create_project("知识库 A").unwrap();
    let project_b = library.repository().create_project("知识库 B").unwrap();

    let first = import_document(&library, &source, Some(&project_a.id), false).unwrap();
    let other_scope = import_document(&library, &source, Some(&project_b.id), false).unwrap();
    assert!(!other_scope.duplicate);
    assert_ne!(other_scope.record_id, first.record_id);

    let same_scope = import_document(&library, &source, Some(&project_b.id), false).unwrap();
    assert!(same_scope.duplicate);
    assert_eq!(same_scope.record_id, other_scope.record_id);
}

#[test]
fn rejects_documents_larger_than_twenty_megabytes() {
    let root = temp_root("oversize");
    let source = root.join("oversize.txt");
    fs::write(&source, vec![b'a'; 20 * 1024 * 1024 + 1]).unwrap();
    let library = ManagedLibrary::open(root.join("library")).unwrap();

    let error = import_document(&library, &source, None, false).unwrap_err();
    assert!(error.to_string().contains("20 MB"));
}
