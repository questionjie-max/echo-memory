//! 转写退化检测：在转写结果落库之前判断「这次输出是否不可用」。
//!
//! 背景（2026-09 质量审查，P0）：whisper.cpp 在真实长录音上会产生三类退化输出——
//! 1. 英文/外语循环（"top of the top…"）；
//! 2. 外语标注（"(speaking in foreign language)" 反复）；
//! 3. 中文亂碼（形如「但是西偏套有需要无所谓的」的胡言乱语）。
//! 前两类可以在文本层直接识别；第三类需要解码器指标（no_speech/logprob），
//! 留待阶段一流水线指标埋点后补。
//!
//! 检测失败时调用方应放弃落库：失败路径会保留此前的转写版本，
//! 记录状态回到 completed——最坏结果是「显式失败」而不是「静默数据损坏」。

/// 一次转写输出的体检报告。
#[derive(Debug, Clone, PartialEq)]
pub struct DegenerationReport {
    /// 是否判定为退化输出（不可用）。
    pub degenerate: bool,
    /// 判定依据（人类可读，会直接展示给用户）。
    pub reasons: Vec<String>,
    /// 中日韩字符占全部字母数字字符的比例。
    pub cjk_ratio: f64,
    /// 三字符串 repetition rate（出现次数 >1 的三字母/汉字组合数 / 总数）。
    pub trigram_repeat_ratio: f64,
    /// 参与检测的字符总量（去除标点空白后）。
    pub char_count: usize,
}

impl Default for DegenerationReport {
    fn default() -> Self {
        Self {
            degenerate: false,
            reasons: Vec::new(),
            cjk_ratio: 1.0,
            trigram_repeat_ratio: 0.0,
            char_count: 0,
        }
    }
}

/// 只保留文字与数字，去掉标点和空白——CER/占比口径统一。
fn keep_alnum(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric() || matches!(*c, '\u{4e00}'..='\u{9fff}'))
        .collect()
}

fn is_cjk(c: char) -> bool {
    matches!(c, '\u{4e00}'..='\u{9fff}')
}

/// 三字符串 repetition rate：滑窗统计，出现次数 ≥2 的组合视为重复。
fn trigram_repeat_ratio(chars: &[char]) -> f64 {
    if chars.len() < 6 {
        return 0.0;
    }
    let mut seen: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
    let mut repeated = 0usize;
    let mut total = 0usize;
    for window in chars.windows(3) {
        let key: String = window.iter().collect();
        total += 1;
        let count = seen.entry(key).or_insert(0);
        *count += 1;
        if *count == 2 {
            repeated += 1;
        }
    }
    if total == 0 {
        return 0.0;
    }
    repeated as f64 / total as f64
}

/// 中文内容占比（CJK 字符 / 全部字母数字字符）。
fn cjk_ratio(chars: &[char]) -> f64 {
    if chars.is_empty() {
        return 1.0;
    }
    let cjk = chars.iter().filter(|c| is_cjk(**c)).count();
    cjk as f64 / chars.len() as f64
}

/// 判断一次转写输出是否退化。
///
/// `expected_language` 是用户在设置里选择的语言（"zh"/"en"/"auto"…）；
/// "auto" 或无法识别的取值不做语言断言，只查重复率。
/// `segments` 是本次转写的全部片段文本（按顺序拼接判断）。
pub fn assess_transcript_degeneration(
    segments: &[String],
    expected_language: &str,
) -> DegenerationReport {
    let joined: String = segments.concat();
    let chars: Vec<char> = keep_alnum(&joined).chars().collect();
    let char_count = chars.len();
    let mut report = DegenerationReport {
        char_count,
        cjk_ratio: cjk_ratio(&chars),
        trigram_repeat_ratio: trigram_repeat_ratio(&chars),
        ..Default::default()
    };

    // 输出为空：不算「退化」但也不可用——交由调用方按空结果处理，这里只标记。
    if char_count == 0 {
        report.reasons.push("转写结果为空".to_owned());
        report.degenerate = true;
        return report;
    }

    // 规则一：语言不符。设置为中文时，CJK 占比过低即判退化。
    let expects_chinese = expected_language.starts_with("zh");
    if expects_chinese && report.cjk_ratio < 0.3 {
        report.reasons.push(format!(
            "输出语言与设置语言不符（中文内容仅占 {:.0}%）",
            report.cjk_ratio * 100.0
        ));
    }

    // 规则二：三字符串重复率过高——循环幻觉的典型特征。
    if report.trigram_repeat_ratio > 0.3 {
        report.reasons.push(format!(
            "输出存在大量重复片段（重复率 {:.0}%）",
            report.trigram_repeat_ratio * 100.0
        ));
    }

    report.degenerate = !report.reasons.is_empty();
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    fn normal_zh() -> Vec<String> {
        vec![
            "今天我们把第三季度的产品复盘安排在下周三下午两点".to_owned(),
            "李明负责整理用户反馈，王芳对接设计团队".to_owned(),
            "这次复盘的重点是转写准确率和模型下载流程".to_owned(),
        ]
    }

    #[test]
    fn normal_chinese_passes() {
        let report = assess_transcript_degeneration(&normal_zh(), "zh");
        assert!(!report.degenerate, "{report:?}");
        assert!(report.reasons.is_empty());
        assert!(report.cjk_ratio > 0.9);
    }

    #[test]
    fn english_loop_is_caught() {
        // 真实失败样本的形态：英文短语无限循环。
        let mut looped = Vec::new();
        for _ in 0..40 {
            looped.push(
                "So what I wanted to say is that I was able to go to the top of the top".to_owned(),
            );
        }
        let report = assess_transcript_degeneration(&looped, "zh");
        assert!(report.degenerate);
        assert!(
            report.reasons.iter().any(|r| r.contains("语言"))
                || report.reasons.iter().any(|r| r.contains("重复"))
        );
    }

    #[test]
    fn foreign_language_marker_is_caught() {
        let mut markers = Vec::new();
        for _ in 0..30 {
            markers.push("(speaking in foreign language)".to_owned());
        }
        let report = assess_transcript_degeneration(&markers, "zh");
        assert!(report.degenerate);
        assert!(report.cjk_ratio < 0.05);
    }

    #[test]
    fn normal_chinese_is_not_flagged_as_repetitive() {
        // 会议里 legitimately 会重复词（「转写」「模型」），但三字符串重复率应远低于阈值。
        let report = assess_transcript_degeneration(&normal_zh(), "zh");
        assert!(
            report.trigram_repeat_ratio < 0.1,
            "重复率异常高: {}",
            report.trigram_repeat_ratio
        );
    }

    #[test]
    fn empty_output_is_flagged() {
        let report = assess_transcript_degeneration(&[], "zh");
        assert!(report.degenerate);
        assert!(report.reasons.iter().any(|r| r.contains("为空")));
    }

    #[test]
    fn auto_language_skips_language_assertion() {
        // 语言为 auto 时不因英文输出判退化（只查重复）。
        let report = assess_transcript_degeneration(
            &["Hello everyone welcome to the review".to_owned()],
            "auto",
        );
        assert!(!report.degenerate, "{report:?}");
    }

    #[test]
    fn short_but_valid_transcript_passes() {
        let report = assess_transcript_degeneration(&["你好世界".to_owned()], "zh");
        assert!(!report.degenerate, "{report:?}");
    }
}
