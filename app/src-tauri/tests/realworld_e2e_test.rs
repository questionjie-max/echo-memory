// 真实环境端到端测试：需要 ECHO_E2E=1 + 真实 Whisper 模型（默认探测路径）+ 本机 Ollama。
// 覆盖 v0.4.0 全链路：导入 → 本地转写 → 本地分析 → Dock 对话 → 模板草稿 → 产出文件夹导出。
use echo_memory_lib::commands::{analyze_with_library, transcribe_with_library};
use echo_memory_lib::dock;
use echo_memory_lib::library::ManagedLibrary;
use std::path::PathBuf;

const AUDIO: &str = "/tmp/echo-e2e/meeting.wav";

fn e2e_enabled() -> bool {
    std::env::var("ECHO_E2E").ok().as_deref() == Some("1") && std::path::Path::new(AUDIO).is_file()
}

#[test]
#[ignore]
fn full_pipeline_from_real_speech_to_output_folder() {
    if !e2e_enabled() {
        eprintln!("跳过：需要 ECHO_E2E=1 且 {AUDIO} 存在");
        return;
    }
    let root = std::env::temp_dir().join(format!("echo-e2e-lib-{}", uuid::Uuid::new_v4()));
    let library = ManagedLibrary::open(root.clone()).unwrap();
    let output_folder = std::env::temp_dir().join(format!("echo-e2e-out-{}", uuid::Uuid::new_v4()));

    // 配置产出文件夹 + 打开自动导出。
    library
        .repository()
        .set_setting_value("output_folder", &output_folder.to_string_lossy())
        .unwrap();
    library
        .repository()
        .set_setting_value("auto_export_analysis", "true")
        .unwrap();

    // 1) 导入真实语音。
    let ingest = library
        .import_audio(&PathBuf::from(AUDIO), None, false)
        .expect("导入失败");
    assert!(!ingest.duplicate);

    // 2) 真实 Whisper 转写（Meetily small 模型，默认探测路径）。
    transcribe_with_library(&library, &ingest.record_id).expect("转写失败");
    let segments = library
        .repository()
        .list_transcript_segments(&ingest.record_id)
        .unwrap();
    assert!(!segments.is_empty(), "转写结果为空");
    let transcript = segments
        .iter()
        .map(|segment| echo_memory_lib::transcript::effective_text(segment))
        .collect::<Vec<_>>()
        .join("");
    eprintln!(
        "转写片段数：{}，开头：{}",
        segments.len(),
        &transcript.chars().take(60).collect::<String>()
    );

    // 3) 真实 Ollama 分析（qwen2.5:7b）。
    analyze_with_library(&library, &ingest.record_id).expect("分析失败");
    let analysis = library
        .repository()
        .latest_analysis(&ingest.record_id)
        .unwrap()
        .expect("分析缺失");
    assert!(
        analysis.content_json.contains("summary"),
        "分析 JSON 缺少摘要字段"
    );
    eprintln!("分析已生成（{} 字符）", analysis.content_json.len());

    // 4) 自动导出钩子应已落盘（run_analysis 内部触发）。
    let outputs = dock::list_recent_outputs(&output_folder, 5);
    assert!(!outputs.is_empty(), "自动导出未生效");
    eprintln!("自动产出：{}", outputs[0].file_name);

    // 4.5) AI 校对（v0.3.0 功能，依赖 raw_generate 修复后真实生效）。
    let corrected = echo_memory_lib::analysis::correct_transcript_with_library(
        &library,
        &ingest.record_id,
    )
    .expect("AI 校对失败");
    assert!(corrected > 0, "校对未修改任何片段");
    eprintln!("AI 校对修正了 {corrected} 个片段");

    // 5) Dock 本地对话（自由模式）。
    let reply = dock::ask_local(
        &library,
        dock::MODE_FREE,
        &[],
        "用一句话说明你能做什么",
        None,
    )
    .expect("Dock 对话失败");
    assert!(reply.chars().count() > 4, "Dock 回复异常：{reply}");
    eprintln!("Dock 回复：{}", &reply.chars().take(50).collect::<String>());

    // 6) 总结模式注入当前记录上下文。
    let summary = dock::ask_local(
        &library,
        dock::MODE_SUMMARY,
        &[],
        "这次录音里决定了什么？",
        Some(&ingest.record_id),
    )
    .expect("总结模式失败");
    assert!(summary.chars().count() > 8);
    eprintln!(
        "总结回复：{}",
        &summary.chars().take(60).collect::<String>()
    );

    // 7) 模板草稿生成（真实本地模型 + JSON schema）。
    let draft = dock::generate_template_draft(
        &library,
        &[dock::WizardMessage {
            role: "user".to_owned(),
            content: "客户访谈模板，重点挖掘客户痛点和竞品对比，需要一个跟进计划栏目".to_owned(),
        }],
    )
    .expect("模板草稿生成失败");
    assert!(
        draft.sections.len() >= 2,
        "栏目不足：{}/{:?}",
        draft.sections.len(),
        draft.name
    );
    eprintln!(
        "模板草稿「{}」含 {} 个栏目",
        draft.name,
        draft.sections.len()
    );

    // 8) 逐字稿手动导出 + 重名去重。
    let first = dock::export_record_kind(&library, &ingest.record_id, "transcript").unwrap();
    let second = dock::export_record_kind(&library, &ingest.record_id, "transcript").unwrap();
    assert_ne!(first, second, "重名文件未去重");
    let content = std::fs::read_to_string(&first).unwrap();
    assert!(content.starts_with("---"), "缺少 frontmatter");

    let _ = std::fs::remove_dir_all(root);
    let _ = std::fs::remove_dir_all(output_folder);
}

#[test]
#[ignore]
fn template_draft_against_real_ollama() {
    if !e2e_enabled() {
        eprintln!("跳过：需要 ECHO_E2E=1");
        return;
    }
    let root = std::env::temp_dir().join(format!("echo-e2e-tpl-{}", uuid::Uuid::new_v4()));
    let library = ManagedLibrary::open(root.clone()).unwrap();
    let draft = dock::generate_template_draft(
        &library,
        &[
            dock::WizardMessage {
                role: "user".to_owned(),
                content: "客户访谈模板，重点挖掘客户痛点和竞品对比，需要一个跟进计划栏目"
                    .to_owned(),
            },
            dock::WizardMessage {
                role: "assistant".to_owned(),
                content: "收到，会包含痛点、竞品对比与跟进计划栏目。".to_owned(),
            },
        ],
    )
    .expect("模板草稿生成失败");
    eprintln!(
        "草稿「{}」：{:?}",
        draft.name,
        draft
            .sections
            .iter()
            .map(|s| s.title.clone())
            .collect::<Vec<_>>()
    );
    let _ = std::fs::remove_dir_all(root);
}
