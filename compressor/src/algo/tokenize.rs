//! 句子切分 + jieba 中文分词

use jieba_rs::Jieba;
use once_cell::sync::Lazy;
use std::sync::OnceLock;

static JIEBA: OnceLock<Jieba> = OnceLock::new();

fn jieba() -> &'static Jieba {
    JIEBA.get_or_init(Jieba::new)
}

/// 一个被切出来的句子，附带分词结果
pub struct Sentence {
    pub text: String,
    pub words: Vec<String>,
}

static SENTENCE_END: Lazy<Vec<char>> = Lazy::new(|| {
    vec!['。', '！', '？', '；', '.', '!', '?', ';', '\n']
});

/// 按中英文句末标点 + 换行切分句子
pub fn split_sentences(text: &str) -> Vec<Sentence> {
    let jieba = jieba();
    let mut sentences = Vec::new();
    let mut current = String::new();

    for ch in text.chars() {
        current.push(ch);
        if SENTENCE_END.contains(&ch) {
            push_sentence(&mut sentences, jieba, &current);
            current.clear();
        }
    }
    push_sentence(&mut sentences, jieba, &current);
    sentences
}

fn push_sentence(out: &mut Vec<Sentence>, jieba: &Jieba, raw: &str) {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return;
    }
    let words: Vec<String> = jieba
        .cut(trimmed, true)
        .iter()
        .map(|s| s.to_string())
        .filter(|s| !s.trim().is_empty())
        .collect();
    out.push(Sentence {
        text: trimmed.to_string(),
        words,
    });
}
