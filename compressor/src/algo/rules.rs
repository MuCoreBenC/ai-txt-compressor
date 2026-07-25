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

// === 模式化规则 ===
/// 百分之N → N%（仅阿拉伯数字版，中文数字需单独解析）
static RE_PERCENT: Lazy<Regex> = Lazy::new(|| Regex::new(r"百分之(\d+)").unwrap());
/// 程度副词 + 后接标点：删除副词，保留标点（Rust regex 不支持 lookahead，用捕获组保留标点）
static RE_DEGREE_ADV: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?:非常|十分|极其|相当|格外|尤为|甚为|极为)([，。、！？；])").unwrap());

/// 单位映射表（长串先替换，避免"公里每小时"被"公里"吃掉）
fn replace_units(t: &str) -> String {
    let mut r = t.to_string();
    // 长串先替换，避免"公里每小时"被"公里"吃掉
    r = r.replace("公里每小时", "km/h");
    r = r.replace("公里", "km");
    r = r.replace("公斤", "kg");
    r = r.replace("千克", "kg");
    r = r.replace("毫升", "ml");
    r = r.replace("厘米", "cm");
    r = r.replace("毫米", "mm");
    r
}

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

    // 8. 模式化规则：百分比 → N%，单位统一，删句末程度副词
    t = RE_PERCENT.replace_all(&t, "$1%").to_string();
    t = replace_units(&t);
    t = RE_DEGREE_ADV.replace_all(&t, "$1").to_string();

    // 9. 同义词替换
    t = apply_synonyms(&t);
    // 10. 删填充词
    t = remove_fillers(&t);
    // 11. 删填充词后留下的句首标点
    t = RE_LEADING_PUNCT.replace_all(&t, "").to_string();
    // 12. 连续标点合并为第一个
    t = RE_CONSEC_PUNCT.replace_all(&t, "$1").to_string();

    // 13. 最终合并空白
    t = RE_MULTI_NEWLINE.replace_all(&t, " ").to_string();
    t = RE_MULTI_SPACE.replace_all(&t, " ").to_string();
    t.trim().to_string();
    // 14. trim 后再次清句首标点（防止合并空白后产生新的句首标点）
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
/// 顺序：长串在前，避免短串吃掉长串前缀（如"截至目前为止"先于"截至"）
const SYNONYMS: &[(&str, &str)] = &[
    // 长短语（先替换）
    ("目前为止", "至今"),
    ("到目前为止", "至今"),
    ("截至目前为止", "至今"),
    ("截至目前", "至今"),
    ("除此之外", "此外"),
    ("在此之外", "此外"),
    ("在此之中", "其中"),
    ("没有任何", "无"),
    ("已经完成了", "完成"),
    ("已经开始", "已开始"),
    ("开始进行", "开始"),
    ("继续进行", "继续"),
    ("正在进行", "正在"),
    ("目前正在", "正在"),
    ("进行了", "进行"),
    ("做出了", "做出"),
    ("提出了", "提出"),
    ("表示了", "表示"),
    ("强调了", "强调"),
    ("指出了", "指出"),
    ("进行讨论", "讨论"),
    ("进行研究", "研究"),
    ("进行分析", "分析"),
    ("进行调查", "调查"),
    ("认为应该", "应"),
    ("不能够", "不能"),
    // 中长词
    ("大约", "约"),
    ("差不多", "约"),
    ("大概", "约"),
    ("因此", "故"),
    ("然而", "但"),
    ("不仅", "且"),
    ("而且", "且"),
    ("另外", "又"),
    ("这些", "此"),
    ("这个", "此"),
    // 短词（后替换）
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
    ("此外", "又"),
    ("可以", "可"),
    ("认为", "认"),
    ("开始", "始"),
    ("结束", "终"),
    ("目前", "现"),
    ("现在", "现"),
    ("之间", "间"),
    // 范围词（删尾缀，留数字）
    ("左右", ""),
    ("上下", ""),
];

/// 填充词/废话短语表
/// 顺序：长串在前，避免短串提前吃掉长串（如"值得注意的是"先于"值得"）
const FILLERS: &[&str] = &[
    // 长短语（先删）
    "从某种意义上来说",
    "值得一提的是",
    "需要注意的是",
    "有必要指出",
    "毋庸置疑",
    "毫无疑问",
    "众所周知",
    "不言而喻",
    "不可否认",
    "理所当然",
    "显而易见",
    "由此看来",
    "与此同时",
    "在此期间",
    "就目前来看",
    "从目前来看",
    "相比之下",
    "顺带一提",
    "顺便说一下",
    "综上所述",
    "总而言之",
    "总的来说",
    "一般来说",
    "换句话说",
    "也就是说",
    "换言之",
    "由此可见",
    "不难看出",
    "不难发现",
    "需要指出的是",
    "必须指出的是",
    "应该指出的是",
    "值得注意的是",
    "有趣的是",
    "重要的是",
    "据了解",
    // 短词（后删）
    "实际上",
    "基本上",
    "可以说",
    "当然了",
    "当然啦",
    "事实上",
    "其实",
    "首先",
    "其次",
    "再次",
    "最后",
];
