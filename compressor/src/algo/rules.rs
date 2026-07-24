//! 规则清洗：去 MD 符号 + 合并断句 + 同义词替换 + 删填充词
//!
//! 与现有 index.html 的 compressMD 规则对齐，并额外做语义级精简。

use once_cell::sync::Lazy;
use regex::Regex;

// === MD 符号相关 ===
static RE_CODE_FENCE: Lazy<Regex> = Lazy::new(|| Regex::new(r"```[a-zA-Z]*\r?\n?").unwrap());
static RE_INLINE_CODE: Lazy<Regex> = Lazy::new(|| Regex::new(r"`([^`\n]+)`").unwrap());
static RE_HEADING: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?m)^#{1,6}\s+").unwrap());
static RE_SEPARATOR: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?m)^---+\s*$").unwrap());
static RE_LIST: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?m)^[*+\-]\s+").unwrap());
static RE_QUOTE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?m)^>\s?").unwrap());
static RE_BOLD: Lazy<Regex> = Lazy::new(|| Regex::new(r"\*\*([^*\n]+)\*\*").unwrap());

// === 标点相关 ===
static RE_SPACE_BEFORE_PUNCT: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\s+([，。！？；,\.!?;:、])").unwrap());
static RE_LEADING_PUNCT: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?m)^[，。！？；,\.!?;:、]\s*").unwrap());
static RE_CONSEC_PUNCT: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"([，。！？；,\.!?;:、])[，。！？；,\.!?;:、]+").unwrap());

// === 最终合并 ===
static RE_MULTI_NEWLINE: Lazy<Regex> = Lazy::new(|| Regex::new(r"[\r\n]+").unwrap());
static RE_MULTI_SPACE: Lazy<Regex> = Lazy::new(|| Regex::new(r"[ \t]+").unwrap());

/// 应用规则清洗管线
pub fn apply(text: &str) -> String {
    let mut t = text.to_string();

    // 1. 代码围栏
    t = RE_CODE_FENCE.replace_all(&t, "").to_string();
    t = t.replace("```", "");
    // 2. 行内反引号 `code` → code
    t = RE_INLINE_CODE.replace_all(&t, "$1").to_string();
    // 3. MD 结构符号
    t = RE_HEADING.replace_all(&t, "").to_string();
    t = RE_SEPARATOR.replace_all(&t, "").to_string();
    t = RE_LIST.replace_all(&t, "").to_string();
    t = RE_QUOTE.replace_all(&t, "").to_string();
    // 4. 粗体
    t = RE_BOLD.replace_all(&t, "$1").to_string();
    // 5. 流程箭头 ↓ → 内联 →
    t = t.replace("\r\n↓\r\n", " → ");
    t = t.replace("\r\n↓\n", " → ");
    t = t.replace("\n↓\r\n", " → ");
    t = t.replace("\n↓\n", " → ");
    t = t.replace('↓', "→");
    // 6. 断句合并
    t = t.replace("：\r\n", "");
    t = t.replace("：\n", "");
    t = t.replace("。\r\n", "。");
    t = t.replace("。\n", "。");
    // 7. 标点前空格
    t = RE_SPACE_BEFORE_PUNCT.replace_all(&t, "$1").to_string();

    // 8. 同义词替换（保守版，只保留语义无损的）
    t = apply_synonyms(&t);
    // 9. 删填充词
    t = remove_fillers(&t);
    // 10. 删填充词后留下的句首标点
    t = RE_LEADING_PUNCT.replace_all(&t, "").to_string();
    // 11. 连续标点合并为第一个
    t = RE_CONSEC_PUNCT.replace_all(&t, "$1").to_string();

    // 12. 最终合并空白
    t = RE_MULTI_NEWLINE.replace_all(&t, " ").to_string();
    t = RE_MULTI_SPACE.replace_all(&t, " ").to_string();
    t.trim().to_string();
    // 13. trim 后再次清句首标点（防止合并空白后产生新的句首标点）
    t = t.trim_start_matches(|c: char| matches!(c, '，' | '。' | '！' | '？' | '；' | ',' | '.' | '!' | '?' | ';' | ':')).to_string();
    t
}

fn apply_synonyms(t: &str) -> String {
    let mut result = t.to_string();
    for (long, short) in SYNONYMS {
        result = result.replace(long, short);
    }
    result
}

fn remove_fillers(t: &str) -> String {
    let mut result = t.to_string();
    for filler in FILLERS {
        result = result.replace(filler, "");
    }
    result
}

/// 同义词替换表（保守，仅保留语义无损的）
const SYNONYMS: &[(&str, &str)] = &[
    ("但是", "但"),
    ("因为", "因"),
    ("所以", "故"),
    ("如果", "若"),
    ("虽然", "虽"),
    ("能够", "能"),
    ("已经", "已"),
    ("进行", "做"),
    ("使用", "用"),
    ("需要", "需"),
    ("通过", "经"),
    ("以及", "及"),
    ("并且", "并"),
    ("或者", "或"),
    ("然后", "后"),
    ("非常", "很"),
    ("比较", "较"),
    ("因此", "故"),
    ("然而", "但"),
    ("此外", "又"),
    ("另外", "又"),
    ("不仅", "且"),
    ("而且", "且"),
    ("这个", "此"),
    ("这些", "此"),
    ("可以", "可"),
    ("认为", "认"),
    ("开始", "始"),
    ("结束", "终"),
    ("目前", "现"),
    ("现在", "现"),
    ("之间", "间"),
];

/// 填充词/废话短语表
const FILLERS: &[&str] = &[
    "实际上",
    "基本上",
    "一般来说",
    "总的来说",
    "可以说",
    "也就是说",
    "换句话说",
    "换言之",
    "众所周知",
    "毋庸置疑",
    "毫无疑问",
    "当然了",
    "当然啦",
    "总而言之",
    "综上所述",
    "由此可见",
    "不难看出",
    "不难发现",
    "需要指出的是",
    "必须指出的是",
    "应该指出的是",
    "值得注意的是",
    "有趣的是",
    "重要的是",
    "首先",
    "其次",
    "再次",
    "最后",
    "事实上",
    "其实",
];
