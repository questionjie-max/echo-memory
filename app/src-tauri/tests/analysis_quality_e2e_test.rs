//! 分析质量结构闸门（v0.9）：对齐 docs/analysis-quality.md §2 的自动断言部分。
//! 需要本机 Ollama 与已安装模型，默认忽略：
//!   ECHO_E2E=1 ECHO_ANALYSIS_MODEL=qwen2.5:7b cargo test --features mcp-bin \
//!     --test analysis_quality_e2e_test -- --ignored --nocapture
use echo_memory_lib::analysis::{
    analysis_is_blocking, finalize_analysis_draft, AnalysisDraft, OllamaAdapter,
};
use echo_memory_lib::types::TranscriptSegment;

fn e2e_enabled() -> bool {
    std::env::var("ECHO_E2E").ok().as_deref() == Some("1")
}

fn adapter() -> OllamaAdapter {
    let model = std::env::var("ECHO_ANALYSIS_MODEL").unwrap_or_else(|_| "qwen2.5:7b".to_owned());
    OllamaAdapter::detect(&model).expect("Ollama 不可用或模型未安装")
}

/// 逐字稿行格式与 `run_analysis` 组装的一致：`[序号][说话人] 文本`。
/// 测试里用同样的序号反推片段，把模型给出的编号别名解析回真实 ID 后逐条校验。
fn segments_from_transcript(transcript: &str) -> Vec<TranscriptSegment> {
    transcript
        .lines()
        .filter(|line| !line.trim().is_empty())
        .enumerate()
        .map(|(index, line)| TranscriptSegment {
            id: format!("s{index}"),
            record_id: "record".to_owned(),
            sequence: index as i64,
            speaker_label: None,
            start_ms: index as i64 * 10_000,
            end_ms: (index as i64 + 1) * 10_000,
            original_text: line.to_owned(),
            normalized_text: None,
            normalization_version: None,
            edited_text: None,
        })
        .collect()
}

/// 走与 run_analysis 相同的收口路径（别名解析 → 指令栏目裁剪 → 引用校验）。
fn verify(draft: AnalysisDraft, transcript: &str, instruction: Option<&str>) -> AnalysisDraft {
    let segments = segments_from_transcript(transcript);
    finalize_analysis_draft(&draft, &segments, instruction, None)
}

/// 长会议：>4500 字，触发分块 + 合并路径；内容带有明确的话题转换（章节素材）。
fn long_meeting_transcript() -> String {
    let topics = [
        "发布节奏需要重新排期，灰度阶段先控制在两小时以内，避免用户侧出现不可回滚的变更",
        "接口定义频繁变化是当前返工的主要来源，需要在评审前冻结契约再进入开发",
        "客户留存的预警阈值还没有定，建议先按活跃度和关键人变动两个维度观察一个季度",
        "上季度的转化数据比预期低，需要排查是渠道质量下降还是落地页改版带来的影响",
        "本地模型的内存占用在长录音上偏高，需要评估分块大小与并发度的取舍",
        "文档导入的解析失败率偏高，主要是扫描件没有文字层，需要增加人工复核入口",
    ];
    (0..130)
        .map(|index| {
            let speaker = if index % 2 == 0 { "张三" } else { "李四" };
            let mut text = topics[index % topics.len()].to_owned();
            text.push_str(&format!("。这是第 {index} 段讨论。"));
            if index % 7 == 3 {
                text.push_str("我们决定按这个方向执行。");
            }
            if index % 5 == 2 {
                text.push_str("请张三在下周前给出结论。");
            }
            format!("[{index}][{speaker}] {text}")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// 带明确待办、但没有"已确认决策"的短会；配「只保留待办事项」指令使用。
fn todo_transcript() -> String {
    let lines = [
        "今天主要同步一下这周的进展，先从我这边开始。",
        "客户端的新版本已经打包完成，测试同事这两天会在真机上过一遍。",
        "服务端的缓存层还在观察，暂时没有调整计划。",
        "请李四在周三之前把灰度名单整理出来，按地区分组发到群里。",
        "还麻烦王五下周一把埋点方案补全，缺的部分找数据组要口径。",
        "另外张三月底前把合同模板重新过一遍，法务那边有两条新意见。",
        "大概就是这些，有问题随时在群里说。",
    ];
    lines
        .iter()
        .enumerate()
        .map(|(index, text)| {
            let speaker = if index % 2 == 0 { "张三" } else { "李四" };
            format!("[{index}][{speaker}] {text}")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// 纯闲聊：没有决策、没有待办、没有未决问题——用来验证"不编造"。
fn chitchat_transcript() -> String {
    let lines = [
        "昨天那家新开的咖啡馆你们去了吗？",
        "去了，拿铁一般，不过环境挺安静的。",
        "我最近在考虑换一把椅子，坐久了腰不太舒服。",
        "宜家那款评价还行，周末可以去试试。",
        "对了，最近天气转凉了，记得加衣服。",
        "好的，谢谢提醒。",
    ];
    lines
        .iter()
        .enumerate()
        .map(|(index, text)| {
            let speaker = if index % 2 == 0 { "甲" } else { "乙" };
            format!("[{index}][{speaker}] {text}")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
#[ignore]
fn long_transcript_analysis_keeps_evidence_and_chapter_order() {
    if !e2e_enabled() {
        eprintln!("跳过：需要 ECHO_E2E=1 与运行中的本机 Ollama");
        return;
    }
    let transcript = long_meeting_transcript();
    println!("长文分析：{} 字", transcript.chars().count());
    let draft = adapter()
        .analyze_with_template_progress(&transcript, None, |_, _| Ok(()))
        .expect("长文分析失败");
    let verified = verify(draft, &transcript, None);

    println!("摘要 {} 字", verified.summary.chars().count());
    println!("关键观点 {} 条", verified.key_points.len());
    println!("章节 {} 个", verified.chapters.len());
    println!("金句 {} 条", verified.highlights.len());
    println!("quality_warning: {:?}", verified.quality_warning);

    assert!(!verified.summary.trim().is_empty(), "摘要为空");
    assert!(
        verified.key_points.len() >= 3,
        "关键观点少于 3 条（{}），分块合并可能丢失内容",
        verified.key_points.len()
    );
    assert!(
        !analysis_is_blocking(&verified),
        "长文分析出现 Blocking 质量问题：{:?}",
        verified.quality_warning
    );
    for item in &verified.key_points {
        assert!(
            !item.citation_segment_ids.is_empty() && item.start_ms.is_some(),
            "关键观点缺少可验证引用：{}",
            item.text
        );
    }
    // 章节（有则必查）：引用完整、时间严格递增、可跳播。
    assert!(verified.chapters.len() <= 8, "章节超过 8 个");
    let mut previous_start = None;
    for chapter in &verified.chapters {
        assert!(
            !chapter.citation_segment_ids.is_empty(),
            "章节缺少可验证引用：{}",
            chapter.title
        );
        let start = chapter.start_ms.expect("章节缺少起始时间");
        if let Some(previous) = previous_start {
            assert!(start > previous, "章节时间必须严格递增");
        }
        previous_start = Some(start);
    }
    if verified.chapters.is_empty() {
        eprintln!(
            "提示：本次长文未产出章节（模型能力项，按 docs/analysis-quality.md §3 人工复核）"
        );
    }
    // 金句（有则必查）：引用与坐标完整（逐字性已在校验内强制）。
    for highlight in &verified.highlights {
        assert!(
            !highlight.citation_segment_ids.is_empty() && highlight.start_ms.is_some(),
            "金句缺少可验证引用：{}",
            highlight.quote_text
        );
    }
}

#[test]
#[ignore]
fn instruction_keeps_only_requested_section() {
    if !e2e_enabled() {
        eprintln!("跳过：需要 ECHO_E2E=1 与运行中的本机 Ollama");
        return;
    }
    let transcript = todo_transcript();
    let instruction = "只保留待办事项，按负责人分组";
    println!("带补充信息的短会分析：{instruction}");
    let draft = adapter()
        .analyze_with_template_instruction_progress(&transcript, None, Some(instruction), |_, _| {
            Ok(())
        })
        .expect("带补充信息的分析失败");
    let verified = verify(draft, &transcript, Some(instruction));

    println!("摘要：{}", verified.summary);
    println!("关键观点 {} 条", verified.key_points.len());
    println!("决策 {} 条", verified.decisions.len());
    println!("待办 {} 条", verified.action_items.len());

    assert!(
        verified.key_points.is_empty(),
        "补充要求排除了关键观点，但模型仍返回了 {} 条",
        verified.key_points.len()
    );
    assert!(
        verified.decisions.is_empty(),
        "补充要求排除了决策栏目，但模型仍返回了 {} 条",
        verified.decisions.len()
    );
    assert!(
        !verified.action_items.is_empty(),
        "素材里存在明确待办，但待办栏目为空"
    );
    for item in &verified.action_items {
        assert!(
            !item.citation_segment_ids.is_empty() && item.start_ms.is_some(),
            "待办缺少可验证引用：{}",
            item.text
        );
    }
    assert!(
        !analysis_is_blocking(&verified),
        "带补充信息的分析出现 Blocking：{:?}",
        verified.quality_warning
    );
}

#[test]
#[ignore]
fn sparse_material_does_not_invent_decisions_todos_or_questions() {
    if !e2e_enabled() {
        eprintln!("跳过：需要 ECHO_E2E=1 与运行中的本机 Ollama");
        return;
    }
    let transcript = chitchat_transcript();
    println!("材料不足场景：纯闲聊逐字稿");
    let draft = adapter()
        .analyze_with_template_progress(&transcript, None, |_, _| Ok(()))
        .expect("短素材分析失败");
    let verified = verify(draft, &transcript, None);

    println!("摘要：{}", verified.summary);
    println!("决策 {} 条", verified.decisions.len());
    println!("待办 {} 条", verified.action_items.len());
    println!("未解决问题 {} 条", verified.open_questions.len());

    assert!(
        verified.decisions.is_empty(),
        "闲聊素材没有决策，模型却返回了 {} 条",
        verified.decisions.len()
    );
    assert!(
        verified.action_items.is_empty(),
        "闲聊素材没有待办，模型却返回了 {} 条",
        verified.action_items.len()
    );
    assert!(
        verified.open_questions.is_empty(),
        "闲聊素材没有未决问题，模型却返回了 {} 条",
        verified.open_questions.len()
    );
}
