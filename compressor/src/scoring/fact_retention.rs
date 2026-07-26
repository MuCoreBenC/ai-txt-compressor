// Task 4 实现：事实保留（按类型）评分
//
// 6 类正则抽取器：number / unit / path / version / error_code / proper_noun
// 类型优先级（避免同一实体被多算）：单位 > 路径 > 版本号 > 错误码 > 专有名词 > 数字
// 数字类会排除已被高优先级类型覆盖的字符区间（按 byte range 重叠判定）

use regex::Regex;
use std::collections::HashSet;
use std::sync::OnceLock;

use crate::scoring::{FactRetention, FactsByType};

// ---- 正则缓存（OnceLock 避免每次调用重新编译） ----

static RE_NUMBER: OnceLock<Regex> = OnceLock::new();
static RE_UNIT: OnceLock<Regex> = OnceLock::new();
static RE_PATH: OnceLock<Regex> = OnceLock::new();
static RE_VERSION: OnceLock<Regex> = OnceLock::new();
static RE_ERROR_CODE: OnceLock<Regex> = OnceLock::new();
static RE_PROPER_NOUN: OnceLock<Regex> = OnceLock::new();

fn re_number() -> &'static Regex {
    RE_NUMBER.get_or_init(|| Regex::new(r"\d+(?:\.\d+)?").unwrap())
}

fn re_unit() -> &'static Regex {
    // 长单位在前，避免短单位先匹配（如 "分钟" 优先于 "分"）
    RE_UNIT.get_or_init(|| {
        Regex::new(r"\d+(?:\.\d+)?\s*(?:分钟|小时|秒|天|字|个|条|MB|GB|KB|ms|token|B|b)").unwrap()
    })
}

fn re_path() -> &'static Regex {
    RE_PATH.get_or_init(|| {
        Regex::new(r"(?:compressor/src/[\w./-]+|[\w./-]+\.(?:rs|toml|html|js|json|md|sh))").unwrap()
    })
}

fn re_version() -> &'static Regex {
    RE_VERSION.get_or_init(|| Regex::new(r"v?\d+\.\d+(?:\.\d+)?").unwrap())
}

fn re_error_code() -> &'static Regex {
    // E\d+ 错误码；4xx / 5xx 三位 HTTP 状态码
    RE_ERROR_CODE.get_or_init(|| Regex::new(r"\bE\d+\b|\b[45]\d{2}\b").unwrap())
}

fn re_proper_noun() -> &'static Regex {
    // 首字母大写英文词（可能含数字），如 Qwen3 / DeepSeek / Ollama / M1
    RE_PROPER_NOUN.get_or_init(|| Regex::new(r"\b[A-Z][a-zA-Z]*\d*\b").unwrap())
}

// ---- 类型别名：实体 = (字符串, 起始字节, 结束字节) ----

type Entity = (String, usize, usize);

// ---- 内部抽取器（带位置，用于优先级过滤） ----

fn extract_numbers_pos(text: &str) -> Vec<Entity> {
    re_number()
        .find_iter(text)
        .map(|m| (m.as_str().to_string(), m.start(), m.end()))
        .collect()
}

fn extract_units_pos(text: &str) -> Vec<Entity> {
    re_unit()
        .find_iter(text)
        .map(|m| {
            // 归一化：去掉数字与单位之间的空格，便于 contains 匹配
            let normalized = m.as_str().replace(' ', "");
            (normalized, m.start(), m.end())
        })
        .collect()
}

fn extract_paths_pos(text: &str) -> Vec<Entity> {
    re_path()
        .find_iter(text)
        .map(|m| (m.as_str().to_string(), m.start(), m.end()))
        .collect()
}

fn extract_versions_pos(text: &str) -> Vec<Entity> {
    re_version()
        .find_iter(text)
        .map(|m| (m.as_str().to_string(), m.start(), m.end()))
        .collect()
}

fn extract_error_codes_pos(text: &str) -> Vec<Entity> {
    re_error_code()
        .find_iter(text)
        .map(|m| (m.as_str().to_string(), m.start(), m.end()))
        .collect()
}

fn extract_proper_nouns_pos(text: &str) -> Vec<Entity> {
    re_proper_noun()
        .find_iter(text)
        .map(|m| (m.as_str().to_string(), m.start(), m.end()))
        .collect()
}

// ---- 公共抽取 API（去重 + 排序，返回字符串列表） ----

pub fn extract_numbers(text: &str) -> Vec<String> {
    dedupe_sort(&extract_numbers_pos(text))
}

pub fn extract_units(text: &str) -> Vec<String> {
    dedupe_sort(&extract_units_pos(text))
}

pub fn extract_paths(text: &str) -> Vec<String> {
    dedupe_sort(&extract_paths_pos(text))
}

pub fn extract_versions(text: &str) -> Vec<String> {
    dedupe_sort(&extract_versions_pos(text))
}

pub fn extract_error_codes(text: &str) -> Vec<String> {
    dedupe_sort(&extract_error_codes_pos(text))
}

pub fn extract_proper_nouns(text: &str) -> Vec<String> {
    dedupe_sort(&extract_proper_nouns_pos(text))
}

fn dedupe_sort(items: &[Entity]) -> Vec<String> {
    let set: HashSet<String> = items.iter().map(|(s, _, _)| s.clone()).collect();
    let mut v: Vec<String> = set.into_iter().collect();
    v.sort();
    v
}

// ---- 主入口：计算事实保留率 ----

pub fn compute(original: &str, compressed: &str) -> FactRetention {
    // 1. 抽取所有类型（带位置）
    let units_pos = extract_units_pos(original);
    let paths_pos = extract_paths_pos(original);
    let versions_pos = extract_versions_pos(original);
    let error_codes_pos = extract_error_codes_pos(original);
    let proper_nouns_pos = extract_proper_nouns_pos(original);

    // 2. 收集高优先级类型覆盖的字符区间
    //    优先级：单位 > 路径 > 版本号 > 错误码 > 专名 > 数字
    let mut blocked: Vec<(usize, usize)> = Vec::new();
    blocked.extend(units_pos.iter().map(|(_, s, e)| (*s, *e)));
    blocked.extend(paths_pos.iter().map(|(_, s, e)| (*s, *e)));
    blocked.extend(versions_pos.iter().map(|(_, s, e)| (*s, *e)));
    blocked.extend(error_codes_pos.iter().map(|(_, s, e)| (*s, *e)));
    blocked.extend(proper_nouns_pos.iter().map(|(_, s, e)| (*s, *e)));

    // 3. 抽取数字，排除与高优先级区间重叠的数字（避免 "0.8B" 中的 "0.8" 被重复计数）
    let numbers_pos: Vec<Entity> = extract_numbers_pos(original)
        .into_iter()
        .filter(|(_, s, e)| !blocked.iter().any(|(bs, be)| *s < *be && *bs < *e))
        .collect();

    // 4. 各类型去重 + 排序（方便测试断言稳定）
    let numbers = dedupe_sort(&numbers_pos);
    let units = dedupe_sort(&units_pos);
    let paths = dedupe_sort(&paths_pos);
    let versions = dedupe_sort(&versions_pos);
    let error_codes = dedupe_sort(&error_codes_pos);
    let proper_nouns = dedupe_sort(&proper_nouns_pos);

    let facts_by_type = FactsByType {
        number: numbers.clone(),
        unit: units.clone(),
        path: paths.clone(),
        version: versions.clone(),
        error_code: error_codes.clone(),
        proper_noun: proper_nouns.clone(),
    };

    // 5. 汇总所有事实
    let mut all_facts: Vec<String> = Vec::new();
    all_facts.extend(numbers.iter().cloned());
    all_facts.extend(units.iter().cloned());
    all_facts.extend(paths.iter().cloned());
    all_facts.extend(versions.iter().cloned());
    all_facts.extend(error_codes.iter().cloned());
    all_facts.extend(proper_nouns.iter().cloned());

    let total_facts = all_facts.len();

    // 6. 检查保留：区分大小写的子串匹配
    let mut lost_facts: Vec<String> = all_facts
        .iter()
        .filter(|f| !compressed.contains(f.as_str()))
        .cloned()
        .collect();
    lost_facts.sort();

    let retained_facts = total_facts - lost_facts.len();
    let retention_rate = if total_facts == 0 {
        1.0
    } else {
        retained_facts as f64 / total_facts as f64
    };

    FactRetention {
        total_facts,
        retained_facts,
        retention_rate,
        lost_facts,
        facts_by_type,
    }
}

// ---- 单元测试 ----

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_numbers() {
        let v = extract_numbers("Qwen3 0.8B 6 分钟 4000字");
        assert!(v.contains(&"3".to_string()));
        assert!(v.contains(&"0.8".to_string()));
        assert!(v.contains(&"4000".to_string()));
    }

    #[test]
    fn test_extract_units() {
        let v = extract_units("6分钟 4000字 100MB");
        assert!(v.contains(&"6分钟".to_string()));
        assert!(v.contains(&"4000字".to_string()));
        assert!(v.contains(&"100MB".to_string()));
    }

    #[test]
    fn test_extract_paths() {
        let v = extract_paths("修改 compressor/src/pipeline.rs 和 server.rs");
        assert!(v.iter().any(|s| s.contains("pipeline.rs")));
        assert!(v.contains(&"server.rs".to_string()));
    }

    #[test]
    fn test_extract_versions() {
        let v = extract_versions("升级到 v1.2.3 和 2.0");
        assert!(v.contains(&"v1.2.3".to_string()));
        assert!(v.contains(&"2.0".to_string()));
    }

    #[test]
    fn test_extract_error_codes() {
        let v = extract_error_codes("遇到 E404 和 500 错误");
        assert!(v.contains(&"E404".to_string()));
        assert!(v.contains(&"500".to_string()));
    }

    #[test]
    fn test_extract_proper_nouns() {
        let v = extract_proper_nouns("Qwen3 和 DeepSeek 还有 Ollama");
        assert!(v.contains(&"Qwen3".to_string()));
        assert!(v.contains(&"DeepSeek".to_string()));
        assert!(v.contains(&"Ollama".to_string()));
    }

    #[test]
    fn test_compute_qwen_case() {
        let original = "Qwen3 0.8B 在 M1 上 6 分钟完成";
        let compressed = "Qwen3 模型";
        let result = compute(original, compressed);
        // 总事实数：Qwen3 / 0.8B / M1 / 6 分钟 = 4 个
        // 保留：Qwen3
        // 丢失：0.8B / M1 / 6 分钟
        assert!(result.retained_facts >= 1);
        assert!(result.lost_facts.len() >= 3);
        assert!(
            result.lost_facts.contains(&"0.8B".to_string())
                || result.lost_facts.contains(&"0.8".to_string())
        );
        assert!(result.lost_facts.contains(&"M1".to_string()));
        assert!(result.lost_facts.iter().any(|s| s.contains("分钟")));
    }
}
