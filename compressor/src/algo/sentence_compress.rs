//! 句内成分压缩：删低信息词，保留主谓宾骨架（不删整句）
//!
//! 简化版依存剪枝：jieba 分词后按停用词+修饰词表过滤，
//! 无 POS 标注也能拿到可接受效果。完全不丢句子结构。

use jieba_rs::Jieba;
use std::sync::OnceLock;

use super::stopwords;

static JIEBA: OnceLock<Jieba> = OnceLock::new();

fn jieba() -> &'static Jieba {
    JIEBA.get_or_init(Jieba::new)
}

/// 句内压缩：删低信息词，保留骨架
/// target_ratio 当前未使用（单层过滤已够），保留参数供未来多轮压缩扩展
pub fn compress_sentence(text: &str, _target_ratio: f32) -> String {
    let jieba = jieba();
    let words = jieba.cut(text, true);

    let mut kept: Vec<&str> = Vec::with_capacity(words.len());
    for w in words {
        if is_low_info(w) {
            continue;
        }
        kept.push(w);
    }

    let joined = kept.join("");

    // 清理删除后残留的双标点 / 句首标点
    let cleaned = joined
        .replace("  ", " ")
        .replace(" ，", "，")
        .replace(" 。", "。")
        .replace("，，", "，")
        .replace("。。", "。")
        .replace("，。", "。")
        .trim_start_matches(|c: char| matches!(c, '，' | '。' | '！' | '？'))
        .trim()
        .to_string();

    if cleaned.is_empty() {
        // 极端情况：全被删，返回原文避免空句
        text.trim().to_string()
    } else {
        cleaned
    }
}

fn is_low_info(word: &str) -> bool {
    let w = word.trim();
    if w.is_empty() {
        return true;
    }
    if stopwords::is_stopword(w) {
        return true;
    }
    MODIFIERS.contains(&w)
}

/// 修饰词表：无 POS 标注时近似判断"可删成分"
const MODIFIERS: &[&str] = &[
    // 程度副词
    "非常", "十分", "极其", "相当", "格外", "尤为", "甚为", "极为",
    "特别", "尤其", "最", "更", "越", "太",
    // 时间修饰（与正文无关的时间副词）
    "刚刚", "刚才", "突然", "忽然", "慢慢地", "渐渐地", "逐渐",
    // 量度修饰（"很X"类冗余）
    "很大", "很小", "很高", "很低", "很多", "很少", "很快", "很慢",
    "很大程度", "很大程度上",
    // 冗余指代
    "那个", "这个", "这种", "那种", "这样", "那样",
    // 虚化动词（与"做"重复，rules.rs 已替换为"做"，这里兜底）
    "加以", "予以",
    // 语气词
    "吧", "呢", "啊", "哦", "呀", "嘛", "嗯", "啦", "喽", "哎", "唉", "嗨", "喂",
];
