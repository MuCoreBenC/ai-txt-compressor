//! 算法层：渐进式压缩管线
//!
//! 阶段 1: 规则清洗（去 MD / 填充词 / 同义词 / 单位统一）
//! 阶段 2: 句内成分压缩（保句删词，不丢句子）
//! 阶段 3: TextRank 兜底（仅在前面不够时才删整句）

pub mod rules;
pub mod sentence_compress;
pub mod stopwords;
pub mod textrank;
pub mod tokenize;

/// 压缩结果（含各阶段字数，供 verbose 日志用）
pub struct CompressStage {
    pub text: String,
    pub after_rules: usize,
    pub after_sentence_compress: usize,
    pub after_textrank: usize,
    pub used_textrank: bool,
}

/// 渐进式压缩：返回最终文本
pub fn compress(text: &str, target_chars: usize) -> String {
    compress_with_stages(text, target_chars).text
}

/// 渐进式压缩：返回文本 + 阶段统计
pub fn compress_with_stages(text: &str, target_chars: usize) -> CompressStage {
    // 阶段 1：规则清洗
    let s1 = rules::apply(text);
    let s1_chars = s1.chars().count();
    if s1_chars <= target_chars {
        return CompressStage {
            text: s1,
            after_rules: s1_chars,
            after_sentence_compress: s1_chars,
            after_textrank: s1_chars,
            used_textrank: false,
        };
    }

    // 阶段 2：句内成分压缩（保句删词）
    let sentences = tokenize::split_sentences(&s1);
    let s2: String = sentences
        .iter()
        .map(|s| sentence_compress::compress_sentence(&s.text, 0.7))
        .collect::<Vec<_>>()
        .join("");
    let s2_chars = s2.chars().count();
    if s2_chars <= target_chars {
        return CompressStage {
            text: s2,
            after_rules: s1_chars,
            after_sentence_compress: s2_chars,
            after_textrank: s2_chars,
            used_textrank: false,
        };
    }

    // 阶段 3：TextRank 兜底（删整句，仅在前两阶段不够时）
    let sentences2 = tokenize::split_sentences(&s2);
    let (final_text, after_textrank) = if sentences2.is_empty() {
        (s2.clone(), s2_chars)
    } else {
        let selected = textrank::select(&sentences2, target_chars);
        if selected.is_empty() {
            (s2.clone(), s2_chars)
        } else {
            let n = selected.chars().count();
            (selected, n)
        }
    };

    CompressStage {
        text: final_text,
        after_rules: s1_chars,
        after_sentence_compress: s2_chars,
        after_textrank,
        used_textrank: true,
    }
}
