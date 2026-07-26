// Task 3 实现：信息损失（句子级）评分

use std::collections::HashSet;

use crate::scoring::InformationLoss;

/// 按句末标点 `[。！？\n]` 拆分句子。
///
/// - 句末标点 `。！？` 保留在句尾
/// - 换行 `\n` 作为分隔符但不保留
/// - 过滤空串和纯空白串
/// - 英文文本若无这些标点，整段作为一个句子返回
pub fn split_sentences(text: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        if ch == '。' || ch == '！' || ch == '？' {
            current.push(ch);
            let trimmed = current.trim();
            if !trimmed.is_empty() {
                result.push(trimmed.to_string());
            }
            current.clear();
        } else if ch == '\n' {
            let trimmed = current.trim();
            if !trimmed.is_empty() {
                result.push(trimmed.to_string());
            }
            current.clear();
        } else {
            current.push(ch);
        }
    }
    let trimmed = current.trim();
    if !trimmed.is_empty() {
        result.push(trimmed.to_string());
    }
    result
}

/// 文本分词。
///
/// - 中文：按单字切片（每个汉字一个 token）
/// - 英文：按空白和标点切片，每个英文单词/数字串一个 token
/// - 英文统一转小写以方便匹配
/// - 标点和空白作为分隔符，不产出 token
pub fn tokenize(text: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() {
            current.push(ch.to_ascii_lowercase());
        } else {
            if !current.is_empty() {
                result.push(current.clone());
                current.clear();
            }
            // 非 ASCII 字母数字（如中文）单独成 token；标点/空白忽略
            if ch.is_alphanumeric() {
                result.push(ch.to_string());
            }
        }
    }
    if !current.is_empty() {
        result.push(current);
    }
    result
}

/// 计算 Jaccard 相似度 = |A ∩ B| / |A ∪ B|。
///
/// a 或 b 为空时返回 0.0。
pub fn jaccard(a: &[String], b: &[String]) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let set_a: HashSet<&String> = a.iter().collect();
    let set_b: HashSet<&String> = b.iter().collect();
    let union = set_a.union(&set_b).count();
    if union == 0 {
        return 0.0;
    }
    let intersection = set_a.intersection(&set_b).count();
    intersection as f64 / union as f64
}

/// 计算原文到压缩文本的句子级信息损失。
///
/// 对每个原句，tokenize 后计算与所有压缩句的最大 Jaccard 相似度。
/// 最大相似度 < 0.3 → 标记为"丢失"。
pub fn compute(original: &str, compressed: &str) -> InformationLoss {
    let orig_sentences = split_sentences(original);
    let comp_tokenized: Vec<Vec<String>> = split_sentences(compressed)
        .iter()
        .map(|s| tokenize(s))
        .collect();

    let mut lost_sentences = Vec::new();
    for sent in &orig_sentences {
        let tokens = tokenize(sent);
        let mut max_sim = 0.0;
        for comp_tokens in &comp_tokenized {
            let sim = jaccard(&tokens, comp_tokens);
            if sim > max_sim {
                max_sim = sim;
            }
        }
        if max_sim < 0.3 {
            lost_sentences.push(sent.clone());
        }
    }

    let total = orig_sentences.len();
    let lost = lost_sentences.len();
    let loss_rate = if total == 0 {
        0.0
    } else {
        lost as f64 / total as f64
    };

    InformationLoss {
        lost_sentence_count: lost,
        total_sentence_count: total,
        loss_rate,
        lost_sentences,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_split_sentences_chinese() {
        let s = "第一句。第二句！第三句？";
        let v = super::split_sentences(s);
        assert_eq!(v, vec!["第一句。", "第二句！", "第三句？"]);
    }

    #[test]
    fn test_split_sentences_with_newline() {
        let s = "第一段\n第二段";
        let v = super::split_sentences(s);
        assert_eq!(v, vec!["第一段", "第二段"]);
    }

    #[test]
    fn test_tokenize_chinese() {
        let v = super::tokenize("我是文本");
        assert_eq!(v, vec!["我", "是", "文", "本"]);
    }

    #[test]
    fn test_tokenize_mixed() {
        let v = super::tokenize("Qwen3 是 model");
        // 应包含 "qwen3", "是", "model"（注意大小写处理：建议转小写）
        assert!(v.contains(&"qwen3".to_string()));
        assert!(v.contains(&"是".to_string()));
        assert!(v.contains(&"model".to_string()));
    }

    #[test]
    fn test_jaccard_identical() {
        let a = vec!["a".to_string(), "b".to_string()];
        assert!((super::jaccard(&a, &a) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn test_jaccard_disjoint() {
        let a = vec!["a".to_string()];
        let b = vec!["b".to_string()];
        assert!((super::jaccard(&a, &b) - 0.0).abs() < 1e-9);
    }

    #[test]
    fn test_jaccard_half_overlap() {
        let a = vec!["a".to_string(), "b".to_string()];
        let b = vec!["b".to_string(), "c".to_string()];
        // 交集 {b}=1, 并集 {a,b,c}=3 → 1/3
        assert!((super::jaccard(&a, &b) - 1.0 / 3.0).abs() < 1e-9);
    }

    #[test]
    fn test_compute_one_lost() {
        // 原文 4 句，压缩 2 句，第 3 句完全无匹配
        let original = "苹果是水果。香蕉也是水果。今天天气真好适合出去玩。葡萄是水果。";
        let compressed = "苹果是水果。香蕉也是水果。葡萄是水果。";
        let result = super::compute(original, compressed);
        assert_eq!(result.total_sentence_count, 4);
        assert_eq!(result.lost_sentence_count, 1);
        assert!((result.loss_rate - 0.25).abs() < 1e-9);
        assert_eq!(result.lost_sentences.len(), 1);
        assert!(result.lost_sentences[0].contains("今天天气"));
    }
}
