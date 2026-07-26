//! 重复检测器
//!
//! 提供两种检测器：
//! - `LoopDetector`：N-gram 字符级检测，用于 content（最终输出）
//! - `ThinkingLoopDetector`：长段落哈希检测，用于 thinking/reasoning_content（思考链）
//!
//! 设计原因：
//! 思考链允许反复推导、审视同一结论，简单的 N-gram 会误判正常思考为循环。
//! Qwen3 thinking 模式已知会陷入"整段重复"的循环（几百上千字反复输出），
//! 用长段落哈希检测能精准识别这种模式，不误伤正常推导。

use std::collections::HashMap;

// ==================== N-gram 检测器（用于 content） ====================

/// N-gram 重复检测器
pub struct LoopDetector {
    /// N-gram 长度（字符级，默认 32）
    n: usize,
    /// 重复阈值（默认 5：同一 N-gram 出现 5 次即判定循环）
    threshold: usize,
    /// 滑动窗口字符缓冲（仅保留最近 window_size 个字符）
    buf: String,
    /// 滑动窗口大小（字符数）
    window_size: usize,
    /// N-gram → 出现次数
    ngram_counts: HashMap<String, usize>,
    /// 已处理的字符总数
    total_chars: usize,
    /// 最大字符数限制（硬上限，默认 50000）
    max_chars: usize,
}

impl LoopDetector {
    /// 创建默认参数的检测器（N=32, threshold=5, max_chars=50000）
    /// - N=32：用较长的 N-gram 避免短句式（如"让我想想"、"接下来"）误判
    /// - threshold=5：允许少量重复，同一 32 字符片段重复 5 次才判定循环
    /// - max_chars=50000：硬上限，防止极端情况内存爆炸
    pub fn new() -> Self {
        Self {
            n: 32,
            threshold: 5,
            buf: String::new(),
            window_size: 4000,
            ngram_counts: HashMap::new(),
            total_chars: 0,
            max_chars: 50000,
        }
    }

    /// 喂入新字符增量，返回是否检测到循环
    pub fn feed(&mut self, s: &str) -> bool {
        for c in s.chars() {
            self.buf.push(c);
            self.total_chars += 1;

            if self.buf.chars().count() >= self.n {
                let ngram: String = self.buf.chars().rev().take(self.n).collect::<Vec<_>>().into_iter().rev().collect();
                *self.ngram_counts.entry(ngram).or_insert(0) += 1;
            }

            if self.buf.chars().count() >= self.n {
                let recent_ngram: String = self.buf.chars().rev().take(self.n).collect::<Vec<_>>().into_iter().rev().collect();
                if let Some(&count) = self.ngram_counts.get(&recent_ngram) {
                    if count >= self.threshold {
                        return true;
                    }
                }
            }

            if self.total_chars >= self.max_chars {
                return true;
            }

            if self.buf.chars().count() > self.window_size {
                if self.buf.chars().count() >= self.n {
                    let old_ngram: String = self.buf.chars().take(self.n).collect();
                    if let Some(count) = self.ngram_counts.get_mut(&old_ngram) {
                        *count = count.saturating_sub(1);
                        if *count == 0 {
                            self.ngram_counts.remove(&old_ngram);
                        }
                    }
                }
                if let Some(first_char) = self.buf.chars().next() {
                    let len = first_char.len_utf8();
                    self.buf.drain(..len);
                }
            }
        }
        false
    }

    pub fn reset(&mut self) {
        self.buf.clear();
        self.ngram_counts.clear();
        self.total_chars = 0;
    }

    #[cfg(test)]
    pub fn set_max_chars(&mut self, max: usize) {
        self.max_chars = max;
    }
}

impl Default for LoopDetector {
    fn default() -> Self {
        Self::new()
    }
}

// ==================== 长段落哈希检测器（用于 thinking） ====================

/// 思考链循环检测器：基于"长段落哈希"识别整段重复
///
/// 算法：
/// 1. 将流入的思考链按 `segment_len` 字符切分为段落
/// 2. 对每个段落计算哈希，记录出现次数
/// 3. 当某个段落哈希出现 ≥ `threshold` 次时判定为循环
///
/// 与 N-gram 的区别：
/// - N-gram 检测短片段重复（32 字符），会误判"让我想想"等常见句式
/// - 段落检测要求整段（默认 200 字符）完全相同才计数，
///   正常推导不会出现整段雷同，只有真正循环才会触发
///
/// Qwen3 thinking 循环的特征是几百上千字的内容反复输出，
/// 这种模式会被段落哈希精准捕获。
pub struct ThinkingLoopDetector {
    /// 段落长度（字符级，默认 200）
    segment_len: usize,
    /// 重复阈值（默认 3：同一段落哈希出现 3 次判定循环）
    threshold: usize,
    /// 当前未满 segment_len 的待切分缓冲
    buf: String,
    /// 段落哈希 → 出现次数
    hash_counts: HashMap<u64, usize>,
    /// 已处理的字符总数
    total_chars: usize,
    /// 最大字符数限制（硬上限，默认 80000，思考链允许较长）
    max_chars: usize,
}

impl ThinkingLoopDetector {
    /// 创建默认参数的检测器
    /// - segment_len=200：段落长度，正常推导不会整段雷同
    /// - threshold=3：同一 200 字符段落出现 3 次判定循环
    /// - max_chars=80000：思考链硬上限（约 8 万字）
    pub fn new() -> Self {
        Self {
            segment_len: 200,
            threshold: 3,
            buf: String::new(),
            hash_counts: HashMap::new(),
            total_chars: 0,
            max_chars: 80000,
        }
    }

    /// 喂入新字符增量，返回是否检测到循环
    pub fn feed(&mut self, s: &str) -> bool {
        self.buf.push_str(s);
        self.total_chars += s.chars().count();

        // 硬上限检查
        if self.total_chars >= self.max_chars {
            return true;
        }

        // 按 segment_len 切分段落
        while self.buf.chars().count() >= self.segment_len {
            // 取出前 segment_len 个字符作为段落
            let segment: String = self.buf.chars().take(self.segment_len).collect();
            // 从 buf 移除已切分的段落
            let drain_len: usize = self.buf.chars().take(self.segment_len).map(|c| c.len_utf8()).sum();
            self.buf.drain(..drain_len);

            // 计算段落哈希
            let hash = self.hash_segment(&segment);
            *self.hash_counts.entry(hash).or_insert(0) += 1;

            // 检查是否达到阈值
            if let Some(&count) = self.hash_counts.get(&hash) {
                if count >= self.threshold {
                    return true;
                }
            }
        }
        false
    }

    /// 计算段落哈希（用 std 默认 SipHash，足够区分段落，性能优于字符串比较）
    fn hash_segment(&self, s: &str) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        s.hash(&mut hasher);
        hasher.finish()
    }

    pub fn reset(&mut self) {
        self.buf.clear();
        self.hash_counts.clear();
        self.total_chars = 0;
    }

    #[cfg(test)]
    pub fn set_max_chars(&mut self, max: usize) {
        self.max_chars = max;
    }
}

impl Default for ThinkingLoopDetector {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_no_loop_short_text() {
        let mut d = LoopDetector::new();
        let text = "这是一个测试句子，用来验证检测器不会误报。";
        assert!(!d.feed(text));
    }

    #[test]
    fn test_detect_loop() {
        let mut d = LoopDetector::new();
        let chunk = "这是用来测试循环检测器的一长段中文文本内容应该超过三十二个字符以触发检测";
        assert!(!d.feed(chunk));
        assert!(!d.feed(chunk));
        assert!(!d.feed(chunk));
        assert!(!d.feed(chunk));
        assert!(d.feed(chunk));
    }

    #[test]
    fn test_max_chars_limit() {
        let mut d = LoopDetector::new();
        d.set_max_chars(50);
        let text = "abcdefghijklmnopqrstuvwxyz";
        assert!(!d.feed(text));
        assert!(d.feed(text));
    }

    // ===== ThinkingLoopDetector 测试 =====

    #[test]
    fn test_thinking_no_loop_normal_reasoning() {
        // 正常的递进式思考，每段内容不同，不应触发
        let mut d = ThinkingLoopDetector::new();
        let reasoning = "首先分析问题：用户希望压缩文本。\
                         接着考虑方法：可以用抽取式或生成式。\
                         然后选择策略：先规则清洗再模型精修。\
                         最后验证结果：检查字数和语义保留。";
        assert!(!d.feed(reasoning));
    }

    #[test]
    fn test_thinking_detect_repeated_long_segment() {
        // 同一段 200 字符内容重复 3 次，应触发（threshold=3）
        let mut d = ThinkingLoopDetector::new();
        // 构造恰好 200 字符的段落，repeat(3) 后切出 3 个完全相同的段落
        // 5 组 "abcdefghijklmnopqrstuvwxyz0123456789"（36 字符）= 180，+ "abcdefghijklmnopqrst"（20 字符）= 200
        let unit = "abcdefghijklmnopqrstuvwxyz0123456789\
                    abcdefghijklmnopqrstuvwxyz0123456789\
                    abcdefghijklmnopqrstuvwxyz0123456789\
                    abcdefghijklmnopqrstuvwxyz0123456789\
                    abcdefghijklmnopqrstuvwxyz0123456789\
                    abcdefghijklmnopqrst";
        assert_eq!(unit.chars().count(), 200, "unit 必须恰好 200 字符");
        let repeated = unit.repeat(3); // 600 字符，切出 3 个相同段落，第 3 个触发
        assert!(d.feed(&repeated));
    }

    #[test]
    fn test_thinking_max_chars_limit() {
        let mut d = ThinkingLoopDetector::new();
        d.set_max_chars(100);
        // 构造超过 100 字符的文本（110 字符）
        let text = "abcdefghijklmnopqrstuvwxyz0123456789\
                    abcdefghijklmnopqrstuvwxyz0123456789\
                    abcdefghijklmnopqrstuvwxyz0123456789\
                    abcd";
        assert_eq!(text.chars().count(), 112);
        assert!(d.feed(text));
    }

    #[test]
    fn test_thinking_normal_progression_not_flagged() {
        // 模拟真实的逐步推导，每段都不同
        let mut d = ThinkingLoopDetector::new();
        let segments = [
            "第一步，我需要理解用户的输入文本在讲什么内容。这段文本是关于软件工程的项目计划评审。",
            "第二步，分析需要保留的核心信息。包括评分、问题点、修改建议等关键内容。",
            "第三步，确定压缩策略。采用结构化压缩，保留评分和四大修改点的主要论点。",
            "第四步，开始撰写压缩后的文本。注意保持语义不变，字数控制在目标范围内。",
            "第五步，检查输出。确认字数达标，核心语义保留，没有遗漏重要信息。",
        ];
        for seg in segments.iter() {
            assert!(!d.feed(seg), "正常推导不应触发: {}", seg);
        }
    }
}
