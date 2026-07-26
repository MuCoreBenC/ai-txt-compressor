//! 评分模块：对压缩结果进行多维度评分
//!
//! 维度：
//! - 压缩比（compression_ratio）
//! - 语义相似度（semantic_score，基于 Ollama embedding）
//! - 信息损失（information_loss，句子级）
//! - 事实保留（fact_retention，按类型统计）

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScoreRequest {
    pub original: String,
    pub compressed: String,
    pub original_chars: usize,
    pub compressed_chars: usize,
    pub target_ratio: Option<f64>,
    pub embedding_model: Option<String>,
    pub ollama_base: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScoreReport {
    pub compression_ratio: f64,
    pub semantic_score: Option<f64>,
    pub information_loss: InformationLoss,
    pub fact_retention: FactRetention,
    pub total_score: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InformationLoss {
    pub lost_sentence_count: usize,
    pub total_sentence_count: usize,
    pub loss_rate: f64,
    pub lost_sentences: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FactRetention {
    pub total_facts: usize,
    pub retained_facts: usize,
    pub retention_rate: f64,
    pub lost_facts: Vec<String>,
    pub facts_by_type: FactsByType,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FactsByType {
    pub number: Vec<String>,
    pub unit: Vec<String>,
    pub path: Vec<String>,
    pub version: Vec<String>,
    pub error_code: Vec<String>,
    pub proper_noun: Vec<String>,
}

// 声明子模块（具体实现稍后在 Task 2-5 添加）
pub mod compression_ratio;
pub mod info_loss;
pub mod fact_retention;
pub mod semantic;

/// 评分编排器：依次调用四个维度子模块并汇总总分
///
/// 语义相似度为可选项（需要同时提供 `embedding_model` 与 `ollama_base`），
/// 缺失时不阻断评分，总分按剩余三维归一化。
pub async fn compute_score(req: ScoreRequest) -> ScoreReport {
    // 1. 压缩率
    let compression_ratio =
        compression_ratio::compute(req.original_chars, req.compressed_chars);

    // 2. 信息丢失
    let information_loss = info_loss::compute(&req.original, &req.compressed);

    // 3. 事实保留
    let fact_retention = fact_retention::compute(&req.original, &req.compressed);

    // 4. 语义保持度（可选，可能为 None）
    let semantic_score = match (&req.embedding_model, &req.ollama_base) {
        (Some(m), Some(b)) if !m.is_empty() && !b.is_empty() => {
            semantic::compute(&req.original, &req.compressed, m, b).await
        }
        _ => None,
    };

    // 5. 计算总分
    let total_score = compute_total_score(
        compression_ratio,
        semantic_score,
        &information_loss,
        &fact_retention,
        req.target_ratio,
    );

    ScoreReport {
        compression_ratio,
        semantic_score,
        information_loss,
        fact_retention,
        total_score,
    }
}

/// 总分计算：各维度归一化到 0-1 后加权求和，再映射到 0-100
///
/// 默认权重：
/// - 压缩率 20%
/// - 语义 40%
/// - 信息丢失 20%
/// - 事实保留 20%
///
/// 语义为 None 时，剩余三维权重之和 0.60，除以 0.60 归一化到 1.0
/// （即 None 不会拉低总分）。
fn compute_total_score(
    compression_ratio: f64,
    semantic_score: Option<f64>,
    info_loss: &InformationLoss,
    fact_retention: &FactRetention,
    target_ratio: Option<f64>,
) -> f64 {
    // 压缩率得分：越接近 target_ratio 越高
    let target = target_ratio.unwrap_or(0.5);
    let ratio_score = (1.0 - (compression_ratio - target).abs()).max(0.0).min(1.0);

    // 信息丢失得分：1 - loss_rate
    let info_score = 1.0 - info_loss.loss_rate;

    // 事实保留得分：retention_rate
    let fact_score = fact_retention.retention_rate;

    // 加权计算
    let (total, divisor) = match semantic_score {
        Some(sem) => (
            ratio_score * 0.20 + sem * 0.40 + info_score * 0.20 + fact_score * 0.20,
            1.0,
        ),
        None => (
            ratio_score * 0.20 + info_score * 0.20 + fact_score * 0.20,
            0.60,
        ),
    };

    // 归一化到 0-100
    let score = (total / divisor) * 100.0;
    // 四舍五入到整数
    score.round()
}
