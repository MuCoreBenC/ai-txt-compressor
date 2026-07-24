//! TextRank 抽取式摘要：自实现
//!
//! 句子相似度 = |W_i ∩ W_j| / (log|W_i| + log|W_j|)
//! 迭代公式：WS(V_i) = (1-d) + d * Σ (w_ji / Σ w_jk) * WS(V_j)

use std::collections::HashSet;

use super::stopwords::is_stopword;
use super::tokenize::Sentence;

/// 选出最重要的句子拼到 target_chars 字数以内，保持原文顺序
pub fn select(sentences: &[Sentence], target_chars: usize) -> String {
    let n = sentences.len();
    if n == 0 {
        return String::new();
    }
    if n == 1 {
        return sentences[0].text.clone();
    }

    // 1. 每句的词集合（去停用词）
    let word_sets: Vec<HashSet<&str>> = sentences
        .iter()
        .map(|s| {
            s.words
                .iter()
                .filter(|w| !is_stopword(w))
                .map(|w| w.as_str())
                .collect()
        })
        .collect();

    // 2. 构建相似度矩阵（对称）
    let mut sim = vec![0.0_f32; n * n];
    for i in 0..n {
        for j in (i + 1)..n {
            let intersection = word_sets[i].intersection(&word_sets[j]).count() as f32;
            let denom = (word_sets[i].len() as f32).ln() + (word_sets[j].len() as f32).ln();
            let s = if denom > 0.0 && intersection > 0.0 {
                intersection / denom
            } else {
                0.0
            };
            sim[i * n + j] = s;
            sim[j * n + i] = s;
        }
    }

    // 3. 行和（用于归一化）
    let mut row_sum = vec![0.0_f32; n];
    for i in 0..n {
        for j in 0..n {
            row_sum[i] += sim[i * n + j];
        }
    }

    // 4. Power iteration
    let d = 0.85_f32;
    let mut scores = vec![1.0_f32; n];
    for _ in 0..30 {
        let mut new_scores = vec![1.0_f32; n];
        for i in 0..n {
            let mut sum = 0.0;
            for j in 0..n {
                if i != j && row_sum[j] > 0.0 {
                    sum += sim[j * n + i] / row_sum[j] * scores[j];
                }
            }
            new_scores[i] = (1.0 - d) + d * sum;
        }
        scores = new_scores;
    }

    // 5. 按分数降序排序，贪心选句直到达到目标字数
    let mut indexed: Vec<(usize, f32)> = scores.iter().copied().enumerate().collect();
    indexed.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    let mut selected_indices: Vec<usize> = Vec::new();
    let mut total_chars = 0usize;
    for &(i, _) in &indexed {
        let sentence_chars = sentences[i].text.chars().count();
        if total_chars + sentence_chars > target_chars {
            // 如果还一个都没选，至少选一句（最高分的），避免空输出
            if selected_indices.is_empty() {
                selected_indices.push(i);
            }
            continue;
        }
        selected_indices.push(i);
        total_chars += sentence_chars;
        if total_chars >= target_chars {
            break;
        }
    }

    // 6. 恢复原文顺序拼接
    selected_indices.sort();
    selected_indices
        .iter()
        .map(|&i| sentences[i].text.as_str())
        .collect::<Vec<_>>()
        .join("")
}
