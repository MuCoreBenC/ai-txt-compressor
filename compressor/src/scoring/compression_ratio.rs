//! 压缩率评分维度
//!
//! 计算压缩率 = result_chars / original_chars
//! 该维度始终可计算，不需要任何外部依赖。

/// 计算压缩率 = result_chars / original_chars
///
/// - `original_chars` 为 0 时返回 0.0（避免除零）
/// - 不做 clamp，允许 > 1.0（如压缩结果反而变长，应如实反映）
pub fn compute(original_chars: usize, result_chars: usize) -> f64 {
    if original_chars == 0 {
        return 0.0;
    }
    result_chars as f64 / original_chars as f64
}

#[cfg(test)]
mod tests {
    use super::compute;

    fn approx_eq(actual: f64, expected: f64) -> bool {
        (actual - expected).abs() < 1e-9
    }

    #[test]
    fn equal_length_returns_one() {
        // 等长：4000 -> 4000 => 1.0
        let actual = compute(4000, 4000);
        assert!(approx_eq(actual, 1.0), "expected 1.0, got {}", actual);
    }

    #[test]
    fn half_compressed_returns_half() {
        // 半压缩：4000 -> 2000 => 0.5
        let actual = compute(4000, 2000);
        assert!(approx_eq(actual, 0.5), "expected 0.5, got {}", actual);
    }

    #[test]
    fn sixty_five_percent_compressed() {
        // 65% 压缩：4000 -> 2600 => 0.65
        let actual = compute(4000, 2600);
        assert!(approx_eq(actual, 0.65), "expected 0.65, got {}", actual);
    }

    #[test]
    fn empty_result_returns_zero() {
        // 空结果：4000 -> 0 => 0.0
        let actual = compute(4000, 0);
        assert!(approx_eq(actual, 0.0), "expected 0.0, got {}", actual);
    }

    #[test]
    fn zero_original_returns_zero() {
        // 原文为 0：0 -> 0 => 0.0（避免除零）
        let actual = compute(0, 0);
        assert!(approx_eq(actual, 0.0), "expected 0.0, got {}", actual);
    }

    #[test]
    fn result_longer_than_original_returns_above_one() {
        // 压缩结果更长：100 -> 200 => 2.0（不 clamp，如实反映）
        let actual = compute(100, 200);
        assert!(approx_eq(actual, 2.0), "expected 2.0, got {}", actual);
    }
}
