//! 算法层：规则清洗 + jieba 分词 + TextRank 抽取式摘要

pub mod rules;
pub mod stopwords;
pub mod textrank;
pub mod tokenize;

/// 算法压缩入口：规则清洗 → TextRank 抽取
///
/// `target_chars` 是目标字数上限。若清洗后已达标，直接返回；
/// 否则用 TextRank 选最重要句子拼到目标字数。
pub fn compress(text: &str, target_chars: usize) -> String {
    // 1. 规则清洗：去 MD 符号、合并断句、同义词替换、删填充词
    let after_rules = rules::apply(text);
    let after_rules_chars = after_rules.chars().count();

    // 2. 若清洗后已经达到目标，直接返回
    if after_rules_chars <= target_chars {
        return after_rules;
    }

    // 3. 句子切分 + jieba 分词
    let sentences = tokenize::split_sentences(&after_rules);
    if sentences.is_empty() {
        return after_rules;
    }

    // 4. TextRank 选句
    let selected = textrank::select(&sentences, target_chars);
    if selected.is_empty() {
        return after_rules;
    }
    selected
}
