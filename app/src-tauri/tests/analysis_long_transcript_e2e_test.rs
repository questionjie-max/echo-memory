// 长录音分析端到端验证：需要本机 Ollama 与已安装模型，故默认忽略。
// 覆盖 analyze_long 分块 + 合并路径——这是 JSON 截断修复的主战场，也是唯一
// 没有任何单元测试覆盖的分析路径。运行：
//   ECHO_E2E=1 ECHO_ANALYSIS_MODEL=qwen2.5:7b cargo test --features mcp-bin \
//     --test analysis_long_transcript_e2e_test -- --ignored --nocapture
use echo_memory_lib::analysis::OllamaAdapter;
use std::time::Instant;

fn e2e_enabled() -> bool {
    std::env::var("ECHO_E2E").ok().as_deref() == Some("1")
}

/// 构造一段真实长度的会议逐字稿：分块分析在 4500 字处触发，
/// 这里刻意超过它，让请求走 analyze_long 而不是单次分析。
fn long_meeting_transcript(segments: usize) -> String {
    let speakers = ["张三", "李四"];
    let topics = [
        "发布节奏需要重新排期，灰度阶段先控制在两小时以内，避免用户侧出现不可回滚的变更",
        "接口定义频繁变化是当前返工的主要来源，需要在评审前冻结契约再进入开发",
        "客户留存的预警阈值还没有定，建议先按活跃度和关键人变动两个维度观察一个季度",
        "上季度的转化数据比预期低，需要排查是渠道质量下降还是落地页改版带来的影响",
        "本地模型的内存占用在长录音上偏高，需要评估分块大小与并发度的取舍",
        "文档导入的解析失败率偏高，主要是扫描件没有文字层，需要增加人工复核入口",
    ];
    let open_questions = [
        "这个方案对老版本资料库的兼容性还没有验证，需要确认迁移路径",
        "定价是否包含增值服务还没有结论，需要等财务那边的成本测算",
    ];
    (0..segments)
        .map(|index| {
            let speaker = speakers[index % speakers.len()];
            let mut text = topics[index % topics.len()].to_owned();
            // 让内容有区分度，避免被去重逻辑合并成重复观点。
            text.push_str(&format!("。这是第 {index} 段讨论。"));
            if index % 7 == 3 {
                text.push_str("我们决定按这个方向执行。");
            }
            if index % 5 == 2 {
                text.push_str("请张三在下周前给出结论。");
            }
            if index % 11 == 4 {
                text.push_str(open_questions[index % open_questions.len()]);
            }
            format!("[{}][{}] {}", index, speaker, text)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
#[ignore]
fn long_transcript_analysis_completes_through_chunked_merge() {
    if !e2e_enabled() {
        eprintln!("跳过：需要 ECHO_E2E=1 与运行中的本机 Ollama");
        return;
    }
    let model = std::env::var("ECHO_ANALYSIS_MODEL").unwrap_or_else(|_| "qwen2.5:7b".to_owned());
    let transcript = long_meeting_transcript(240);
    let characters = transcript.chars().count();
    println!("逐字稿 {characters} 字（分块阈值 4500），模型 {model}");

    let adapter = OllamaAdapter::detect(&model).expect("Ollama 不可用或模型未安装");
    let started = Instant::now();
    let mut steps = Vec::new();
    let draft = adapter
        .analyze_with_template_progress(&transcript, None, |current, total| {
            steps.push((current, total));
            Ok(())
        })
        .expect("长逐字稿分析失败");
    let elapsed = started.elapsed();

    println!("进度回调: {steps:?}");
    println!("耗时 {elapsed:?}");
    println!("摘要长度 {}", draft.summary.chars().count());
    println!("关键观点 {} 条", draft.key_points.len());
    println!("决策 {} 条", draft.decisions.len());
    println!("待办 {} 条", draft.action_items.len());
    println!("未解决问题 {} 条", draft.open_questions.len());
    println!("quality_warning: {:?}", draft.quality_warning);

    assert!(
        !draft.summary.trim().is_empty(),
        "合并后摘要为空，说明分块结果没有正确合并"
    );
    assert!(
        draft.key_points.len() >= 3,
        "关键观点少于 3 条（{}），合并阶段可能丢失了内容",
        draft.key_points.len()
    );
}
