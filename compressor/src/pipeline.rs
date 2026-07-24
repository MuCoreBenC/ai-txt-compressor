//! 混合管线编排：原文 → 算法压缩 → 模型压缩 → 结果

use serde::{Deserialize, Serialize};
use std::time::Instant;

#[derive(Debug, Clone, Deserialize)]
pub struct CompressOptions {
    pub ratio: f32,
    pub no_model: bool,
    pub model: String,
    pub verbose: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct CompressResult {
    pub original: usize,
    pub compressed: usize,
    pub ratio: f32,
    pub text: String,
    pub stages: Stages,
}

#[derive(Debug, Clone, Serialize)]
pub struct Stages {
    pub after_algo: usize,
    pub after_model: usize,
}

pub async fn compress(text: &str, opts: &CompressOptions) -> anyhow::Result<CompressResult> {
    let original_chars = text.chars().count();
    if original_chars == 0 {
        return Ok(CompressResult {
            original: 0,
            compressed: 0,
            ratio: 0.0,
            text: String::new(),
            stages: Stages {
                after_algo: 0,
                after_model: 0,
            },
        });
    }

    let target_chars = ((original_chars as f32) * opts.ratio).ceil() as usize;
    let target_chars = target_chars.max(1);
    // 算法阶段目标更宽松（1.5x），给模型留进一步压缩的空间
    let algo_target = ((target_chars as f32) * 1.5).ceil() as usize;

    // === Stage 1: 算法压缩 ===
    let t0 = Instant::now();
    let algo_output = crate::algo::compress(text, algo_target);
    let after_algo = algo_output.chars().count();
    let algo_ms = t0.elapsed().as_millis();

    if opts.verbose {
        eprintln!(
            "[algo] {} → {} chars (algo_target {}, final_target {}, {:.1}% of original, {}ms)",
            original_chars,
            after_algo,
            algo_target,
            target_chars,
            after_algo as f32 / original_chars as f32 * 100.0,
            algo_ms
        );
    }

    // === Stage 2: 模型压缩（可选） ===
    // 触发条件：未指定 --no-model，且 algo 输出尚未达到最终目标，且文本足够长（>30 字）
    let should_run_model = !opts.no_model
        && after_algo > target_chars
        && after_algo > 30;

    let (final_text, after_model) = if !should_run_model {
        if opts.verbose {
            if opts.no_model {
                eprintln!("[model] skipped (--no-model)");
            } else if after_algo <= target_chars {
                eprintln!("[model] skipped: algo output already at/below final target");
            } else {
                eprintln!("[model] skipped: text too short for model compression");
            }
        }
        (algo_output.clone(), after_algo)
    } else {
        let t1 = Instant::now();
        match crate::model::ollama::OllamaClient::new(&opts.model)
            .compress(&algo_output, target_chars)
            .await
        {
            Ok(out) => {
                let after_model = out.chars().count();
                let model_ms = t1.elapsed().as_millis();
                // 如果模型输出没有比算法输出短至少 5%，说明模型没起到作用，回退用算法输出
                let improvement = 1.0 - after_model as f32 / after_algo as f32;
                if improvement < 0.05 {
                    if opts.verbose {
                        eprintln!(
                            "[model] {} → {} chars ({:.1}% of original, {}ms) — improvement too small ({:.1}%), fallback to algo",
                            after_algo,
                            after_model,
                            after_model as f32 / original_chars as f32 * 100.0,
                            model_ms,
                            improvement * 100.0
                        );
                    }
                    (algo_output.clone(), after_algo)
                } else {
                    if opts.verbose {
                        eprintln!(
                            "[model] {} → {} chars ({:.1}% of original, {}ms, -{:.1}%)",
                            after_algo,
                            after_model,
                            after_model as f32 / original_chars as f32 * 100.0,
                            model_ms,
                            improvement * 100.0
                        );
                    }
                    (out, after_model)
                }
            }
            Err(e) => {
                eprintln!("[model] failed, fallback to algo output: {}", e);
                (algo_output.clone(), after_algo)
            }
        }
    };

    Ok(CompressResult {
        original: original_chars,
        compressed: after_model,
        ratio: after_model as f32 / original_chars as f32,
        text: final_text,
        stages: Stages {
            after_algo,
            after_model,
        },
    })
}
