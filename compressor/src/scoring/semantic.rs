//! Task 5 实现：语义相似度（Ollama embedding）评分
//!
//! 调用 Ollama `/api/embeddings` 端点，分别对原文与压缩结果取向量，
//! 计算 cosine 相似度，作为语义保持度得分（0-1）。
//!
//! 任一环节失败（网络/JSON 解析/空向量）返回 None，不阻断评分。

use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Ollama embeddings 请求体
#[derive(Serialize)]
struct EmbedRequest<'a> {
    model: &'a str,
    prompt: &'a str,
}

/// Ollama embeddings 响应体
#[derive(Deserialize)]
struct EmbedResponse {
    embedding: Vec<f64>,
}

/// 调用 Ollama embedding 模型计算原文与压缩结果的语义相似度
///
/// - `original`：原文文本
/// - `compressed`：压缩结果文本
/// - `embedding_model`：embedding 模型名（如 `bge-m3`、`nomic-embed-text`）
/// - `ollama_base`：Ollama 服务基址（如 `http://localhost:11434`）
///
/// 任一环节失败返回 None，不阻断评分。
/// 返回值范围为 [0.0, 1.0]（cosine 经 clamp 处理，负值映射为 0）。
pub async fn compute(
    original: &str,
    compressed: &str,
    embedding_model: &str,
    ollama_base: &str,
) -> Option<f64> {
    if embedding_model.is_empty() || ollama_base.is_empty() {
        return None;
    }

    let client = Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .ok()?;

    let url = format!("{}/api/embeddings", ollama_base.trim_end_matches('/'));

    // 取原文向量
    let req_orig = EmbedRequest {
        model: embedding_model,
        prompt: original,
    };
    let resp_orig: EmbedResponse = client
        .post(&url)
        .json(&req_orig)
        .send()
        .await
        .ok()?
        .json()
        .await
        .ok()?;

    // 取压缩结果向量
    let req_comp = EmbedRequest {
        model: embedding_model,
        prompt: compressed,
    };
    let resp_comp: EmbedResponse = client
        .post(&url)
        .json(&req_comp)
        .send()
        .await
        .ok()?
        .json()
        .await
        .ok()?;

    Some(cosine_similarity(&resp_orig.embedding, &resp_comp.embedding))
}

/// 计算两个向量的 cosine 相似度，结果 clamp 到 [0.0, 1.0]
///
/// - 长度不一致或空向量返回 0.0
/// - 任一向量 norm 为 0 返回 0.0
/// - cosine 实际范围 [-1, 1]，但 embedding 在文本相似度任务上很少出现负值，
///   直接 clamp 到 [0, 1] 不会失真，且符合 spec 中 semanticScore 0-1 的约定
fn cosine_similarity(a: &[f64], b: &[f64]) -> f64 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot: f64 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f64 = a.iter().map(|x| x * x).sum::<f64>().sqrt();
    let norm_b: f64 = b.iter().map(|x| x * x).sum::<f64>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    let sim = dot / (norm_a * norm_b);
    sim.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cosine_identical() {
        let a = vec![1.0, 2.0, 3.0];
        let sim = cosine_similarity(&a, &a);
        assert!((sim - 1.0).abs() < 1e-9);
    }

    #[test]
    fn test_cosine_orthogonal() {
        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0];
        let sim = cosine_similarity(&a, &b);
        assert!(sim.abs() < 1e-9);
    }

    #[test]
    fn test_cosine_empty() {
        let sim = cosine_similarity(&[], &[]);
        assert!((sim - 0.0).abs() < 1e-9);
    }

    #[test]
    fn test_cosine_different_length() {
        let a = vec![1.0, 2.0];
        let b = vec![1.0];
        let sim = cosine_similarity(&a, &b);
        assert!((sim - 0.0).abs() < 1e-9);
    }

    #[test]
    fn test_cosine_zero_vector() {
        let a = vec![0.0, 0.0];
        let b = vec![1.0, 2.0];
        let sim = cosine_similarity(&a, &b);
        assert!((sim - 0.0).abs() < 1e-9);
    }

    #[test]
    fn test_cosine_negative_clamped_to_zero() {
        // cosine 实际可能略低于 0（极端情形），应被 clamp 到 0
        let a = vec![1.0, -1.0];
        let b = vec![-1.0, 1.0];
        let sim = cosine_similarity(&a, &b);
        assert!((sim - 0.0).abs() < 1e-9);
    }

    #[test]
    fn test_cosine_positive_case() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![1.0, 1.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        // cos = 1 / (1 * sqrt(2)) ≈ 0.7071
        assert!((sim - 0.7071067811865476).abs() < 1e-9);
    }

    #[tokio::test]
    async fn test_compute_empty_model() {
        // 空 embedding_model 名应直接返回 None
        let result = compute("test", "test", "", "http://localhost:11434").await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_compute_empty_base() {
        let result = compute("test", "test", "bge-m3", "").await;
        assert!(result.is_none());
    }
}
