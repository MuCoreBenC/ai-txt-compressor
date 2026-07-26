//! 混合管线编排：原文 → 算法压缩 → 模型压缩 → 结果
//!
//! 每次压缩都会构建 RunLog，记录所有阶段的字数、耗时、prompt、模型响应详情，
//! 供前端日志面板展示。

use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

// 心跳推送间隔与阶段切换阈值（避免硬编码字面量散落代码中）
const HEARTBEAT_INTERVAL_SEC: u64 = 10;
const HB_OLLAMA_LOAD_TO_EVAL_SEC: u64 = 30;
const HB_OLLAMA_EVAL_TO_GEN_SEC: u64 = 90;
const HB_API_WAIT_SEC: u64 = 30;

#[derive(Debug, Clone, Deserialize)]
pub struct CompressOptions {
    pub ratio: f32,
    pub no_model: bool,
    pub provider: String,
    pub model: String,
    pub api_key: Option<String>,
    pub base_url: Option<String>,
    pub reasoning_effort: Option<String>,
    /// 自定义 system 提示词（None 用默认）
    pub custom_system: Option<String>,
    /// 自定义 user 模板（None 用默认）
    /// 支持占位符：{text} / {target} / {orig} / {cut}
    pub custom_user_template: Option<String>,
    pub verbose: bool,
    /// 预设提示词 ID（"minimal"/"standard"/"strict_chars"），None 或 "standard" 用默认
    #[serde(default)]
    pub preset: Option<String>,
    /// 若提供则跳过算法阶段直接用此文本调模型（用于重试）
    #[serde(default)]
    pub text_algo: Option<String>,
    /// 显式覆盖目标字数（用于重试时保持原目标，不被 text_algo 长度影响）
    #[serde(default)]
    pub target_chars_override: Option<usize>,
    /// 显式覆盖目标字数（用于用户直接指定"压到 1000 字"）
    #[serde(default)]
    pub target_chars: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CompressResult {
    pub original: usize,
    pub compressed: usize,
    pub ratio: f32,
    pub text: String,
    pub text_algo: String,
    pub stages: Stages,
    /// 完整运行日志（供前端展示）
    pub log: RunLog,
}

#[derive(Debug, Clone, Serialize)]
pub struct Stages {
    pub after_algo: usize,
    pub after_model: usize,
}

/// 单条日志条目
#[derive(Debug, Clone, Serialize)]
pub struct LogEntry {
    /// 时间戳（相对开始的毫秒数）
    pub t: u64,
    /// 级别：info / warn / error
    pub level: String,
    /// 阶段：algo / model / fallback / done
    pub stage: String,
    /// 消息内容
    pub msg: String,
}

/// 模型调用详情
#[derive(Debug, Clone, Serialize)]
pub struct ModelCallDetail {
    pub provider: String,
    pub model: String,
    pub endpoint: String,
    pub reasoning_effort: Option<String>,
    pub system_prompt: String,
    pub user_prompt: String,
    /// 输入字数
    pub input_chars: usize,
    /// 输出字数
    pub output_chars: usize,
    pub elapsed_ms: u64,
    /// 输入 token 数（Ollama 用 prompt_eval_count，OpenAI 兼容用 prompt_tokens）
    pub prompt_tokens: u32,
    /// 输出 token 数
    pub completion_tokens: u32,
    /// 推理 token 数（部分厂商返回）
    pub reasoning_tokens: u32,
    /// 模型原始响应 JSON（截断到 4000 字符）
    pub raw_response: String,
    /// DeepSeek V4 thinking mode 的思考链文本（仅 thinking enabled 时有值，供日志面板单独展示）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_text: Option<String>,
    /// 是否成功
    pub success: bool,
    /// 失败原因（success=false 时有）
    pub error: Option<String>,
}

/// 完整运行日志
#[derive(Debug, Clone, Serialize)]
pub struct RunLog {
    /// 开始时间戳
    pub started_at: u64,
    /// 总耗时（毫秒）
    pub total_ms: u64,
    /// 原始字数
    pub original_chars: usize,
    pub final_chars: usize,
    /// 目标字数
    pub target_chars: usize,
    /// 算法阶段目标字数（启用模型时为 final_target × 1.5）
    pub algo_target: usize,
    pub ratio: f32,
    pub provider: String,
    pub model: String,
    pub no_model: bool,
    /// 日志条目列表（按时间排序）
    pub entries: Vec<LogEntry>,
    /// 模型调用详情（仅启用模型时有）
    pub model_call: Option<ModelCallDetail>,
    /// 是否触发了 fallback
    pub fallback_triggered: bool,
    /// fallback 原因
    pub fallback_reason: Option<String>,
}

// ==================== SSE 流式事件 ====================

/// SSE 推送给前端的事件（序列化为 JSON）
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub enum StreamEvent {
    Init {
        original_chars: usize,
        target_chars: usize,
        algo_target: usize,
        ratio: f32,
        provider: String,
        model: String,
        no_model: bool,
        t: u64,
    },
    AlgoStart {
        t: u64,
    },
    AlgoDone {
        original: usize,
        after_rules: usize,
        after_sentence_compress: usize,
        after_textrank: usize,
        used_textrank: bool,
        elapsed_ms: u64,
        /// 算法阶段输出文本（供前端重试时跳过算法阶段使用）
        algo_text: String,
        t: u64,
    },
    ModelStart {
        provider: String,
        model: String,
        reasoning_effort: Option<String>,
        input_chars: usize,
        system_prompt: String,
        user_prompt: String,
        t: u64,
    },
    /// 模型加载/推理中心跳事件：在 ModelStart 后到第一个 ModelDelta 之间周期性推送
    /// 让前端知道还在等待，避免误以为卡死
    ModelHeartbeat {
        /// 距离 ModelStart 的毫秒数
        elapsed_ms: u64,
        /// 状态提示文案
        phase: String,
        t: u64,
    },
    /// 模型思考链增量（DeepSeek reasoning_content / Ollama qwen3  satisfied 标签）
    /// 实时推送，前端在思考过程面板追加显示
    ModelReasoning {
        delta: String,
        t: u64,
    },
    ModelDelta {
        delta: String,
        t: u64,
    },
    ModelDone {
        output_chars: usize,
        elapsed_ms: u64,
        prompt_tokens: u32,
        completion_tokens: u32,
        reasoning_tokens: u32,
        success: bool,
        error: Option<String>,
        t: u64,
    },
    Fallback {
        reason: String,
        t: u64,
    },
    Done {
        final_text: String,
        final_chars: usize,
        total_ms: u64,
        t: u64,
    },
    Error {
        msg: String,
        t: u64,
    },
}

/// 事件推送通道类型
pub type LogSink = mpsc::Sender<StreamEvent>;

pub async fn compress(text: &str, opts: &CompressOptions) -> anyhow::Result<CompressResult> {
    let start = Instant::now();
    let started_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    let mut entries: Vec<LogEntry> = Vec::new();
    let mut model_call: Option<ModelCallDetail> = None;
    let mut fallback_triggered = false;
    let mut fallback_reason: Option<String> = None;

    let original_chars = text.chars().count();
    if original_chars == 0 {
        return Ok(CompressResult {
            original: 0,
            compressed: 0,
            ratio: 0.0,
            text: String::new(),
            text_algo: String::new(),
            stages: Stages {
                after_algo: 0,
                after_model: 0,
            },
            log: RunLog {
                started_at,
                total_ms: 0,
                original_chars: 0,
                final_chars: 0,
                target_chars: 0,
                algo_target: 0,
                ratio: 0.0,
                provider: opts.provider.clone(),
                model: opts.model.clone(),
                no_model: opts.no_model,
                entries: vec![LogEntry {
                    t: 0,
                    level: "warn".to_string(),
                    stage: "init".to_string(),
                    msg: "输入为空".to_string(),
                }],
                model_call: None,
                fallback_triggered: false,
                fallback_reason: None,
            },
        });
    }

    entries.push(LogEntry {
        t: 0,
        level: "info".to_string(),
        stage: "init".to_string(),
        msg: format!(
            "开始压缩：原文 {} 字，目标比例 {:.0}%，provider={}, model={}",
            original_chars,
            opts.ratio * 100.0,
            opts.provider,
            opts.model
        ),
    });

    let target_chars = match opts.target_chars {
        Some(t) if t > 0 => t,
        _ => ((original_chars as f32) * opts.ratio).ceil() as usize,
    };
    let target_chars = target_chars.max(1);
    let algo_target = if opts.no_model {
        target_chars
    } else {
        ((target_chars as f32) * 1.5).ceil() as usize
    };

    entries.push(LogEntry {
        t: start.elapsed().as_millis() as u64,
        level: "info".to_string(),
        stage: "algo".to_string(),
        msg: format!(
            "算法目标 {} 字（最终目标 {} 字，比例 {:.0}%）",
            algo_target,
            target_chars,
            opts.ratio * 100.0
        ),
    });

    // === Stage 1: 算法压缩（渐进式管线） ===
    let t0 = Instant::now();
    let stage = crate::algo::compress_with_stages(text, algo_target);
    let algo_output = stage.text;
    let after_algo = algo_output.chars().count();
    let algo_ms = t0.elapsed().as_millis() as u64;

    entries.push(LogEntry {
        t: start.elapsed().as_millis() as u64,
        level: "info".to_string(),
        stage: "algo".to_string(),
        msg: format!(
            "算法完成：{} → rules {} → sent_comp {} → textrank {} 字（耗时 {}ms，textrank_used={}）",
            original_chars,
            stage.after_rules,
            stage.after_sentence_compress,
            stage.after_textrank,
            algo_ms,
            stage.used_textrank
        ),
    });

    if opts.verbose {
        eprintln!(
            "[algo] {} → rules {} → sent {} → textrank {} chars (algo_target {}, final_target {}, {:.1}% of original, {}ms, textrank={})",
            original_chars,
            stage.after_rules,
            stage.after_sentence_compress,
            stage.after_textrank,
            algo_target,
            target_chars,
            after_algo as f32 / original_chars as f32 * 100.0,
            algo_ms,
            stage.used_textrank
        );
    }

    // === Stage 2: 模型压缩（可选） ===
    let should_run_model = !opts.no_model
        && after_algo > target_chars
        && after_algo > 30;

    let (final_text, after_model) = if !should_run_model {
        if opts.no_model {
            entries.push(LogEntry {
                t: start.elapsed().as_millis() as u64,
                level: "info".to_string(),
                stage: "model".to_string(),
                msg: "跳过模型（no_model=true）".to_string(),
            });
        } else if after_algo <= target_chars {
            entries.push(LogEntry {
                t: start.elapsed().as_millis() as u64,
                level: "info".to_string(),
                stage: "model".to_string(),
                msg: format!(
                    "跳过模型：算法输出 {} 字已 ≤ 最终目标 {} 字",
                    after_algo, target_chars
                ),
            });
        } else {
            entries.push(LogEntry {
                t: start.elapsed().as_millis() as u64,
                level: "warn".to_string(),
                stage: "model".to_string(),
                msg: format!("跳过模型：文本过短（{} 字 ≤ 30 字）", after_algo),
            });
        }
        (algo_output.clone(), after_algo)
    } else {
        let t1 = Instant::now();
        let prompt_params = crate::prompt::PromptParams {
            text: &algo_output,
            target_chars,
            custom_system: opts.custom_system.as_deref(),
            custom_user_template: opts.custom_user_template.as_deref(),
        };
        let (system, user) = crate::prompt::build_compress_messages_with(prompt_params);
        let reasoning = opts.reasoning_effort.as_deref().filter(|s| !s.is_empty());

        entries.push(LogEntry {
            t: start.elapsed().as_millis() as u64,
            level: "info".to_string(),
            stage: "model".to_string(),
            msg: format!(
                "调用模型：provider={}, model={}, reasoning={:?}, 输入 {} 字",
                opts.provider, opts.model, reasoning, after_algo
            ),
        });

        let endpoint = match opts.provider.as_str() {
            "deepseek" => "https://api.deepseek.com/chat/completions".to_string(),
            "custom" => {
                let base = opts.base_url.as_deref().unwrap_or("");
                if base.contains("/chat/completions") {
                    base.to_string()
                } else {
                    let trimmed = base.trim_end_matches('/');
                    if trimmed.ends_with("/v1") {
                        format!("{}/chat/completions", trimmed)
                    } else {
                        format!("{}/v1/chat/completions", trimmed)
                    }
                }
            }
            _ => "http://127.0.0.1:11434/api/chat".to_string(),
        };

        // 调用模型，返回 enum 分派
        let model_result: anyhow::Result<ModelOutputKind> = match opts.provider.as_str() {
            "deepseek" => {
                let key = opts.api_key.as_deref().unwrap_or("");
                crate::model::openai_compat::OpenAiCompatClient::deepseek(key, &opts.model)
                    .compress_full(&system, &user, reasoning)
                    .await
                    .map(ModelOutputKind::OpenAI)
            }
            "custom" => {
                let base = opts.base_url.as_deref().unwrap_or("");
                let key = opts.api_key.as_deref().unwrap_or("");
                if base.is_empty() {
                    Err(anyhow::anyhow!("自定义 provider 需提供 base_url"))
                } else {
                    crate::model::openai_compat::OpenAiCompatClient::from_base_url(base, key, &opts.model)
                        .compress_full(&system, &user, reasoning)
                        .await
                        .map(ModelOutputKind::OpenAI)
                }
            }
            "ollama" | _ => {
                crate::model::ollama::OllamaClient::new(&opts.model)
                    .compress_full(&system, &user)
                    .await
                    .map(ModelOutputKind::Ollama)
            }
        };

        let model_ms = t1.elapsed().as_millis() as u64;

        match model_result {
            Ok(out) => {
                let (content_str, prompt_tokens, completion_tokens, reasoning_tokens, raw_response, reasoning_text) = match &out {
                    ModelOutputKind::OpenAI(o) => (
                        o.content.clone(),
                        o.usage.prompt_tokens,
                        o.usage.completion_tokens,
                        o.usage.reasoning_tokens,
                        o.raw_response.clone(),
                        o.reasoning_text.clone(),
                    ),
                    ModelOutputKind::Ollama(o) => (
                        o.content.clone(),
                        o.prompt_eval_count,
                        o.eval_count,
                        0u32,
                        o.raw_response.clone(),
                        None,
                    ),
                };

                let after_model = content_str.chars().count();
                let improvement = 1.0 - after_model as f32 / after_algo as f32;

                model_call = Some(ModelCallDetail {
                    provider: opts.provider.clone(),
                    model: opts.model.clone(),
                    endpoint,
                    reasoning_effort: reasoning.map(|s| s.to_string()),
                    system_prompt: system.clone(),
                    user_prompt: user.clone(),
                    input_chars: after_algo,
                    output_chars: after_model,
                    elapsed_ms: model_ms,
                    prompt_tokens,
                    completion_tokens,
                    reasoning_tokens,
                    raw_response,
                    reasoning_text,
                    success: true,
                    error: None,
                });

                if improvement < 0.05 {
                    // 模型未显著缩短 → 回退二次算法
                    fallback_triggered = true;
                    fallback_reason = Some(format!(
                        "模型输出仅缩短 {:.1}%（{} → {} 字），小于 5% 阈值，回退二次算法",
                        improvement * 100.0, after_algo, after_model
                    ));
                    entries.push(LogEntry {
                        t: start.elapsed().as_millis() as u64,
                        level: "warn".to_string(),
                        stage: "fallback".to_string(),
                        msg: fallback_reason.clone().unwrap_or_default(),
                    });
                    let final_text = final_algo_pass(&algo_output, target_chars);
                    let final_len = final_text.chars().count();
                    entries.push(LogEntry {
                        t: start.elapsed().as_millis() as u64,
                        level: "info".to_string(),
                        stage: "fallback".to_string(),
                        msg: format!("回退二次算法压缩：{} → {} 字", after_algo, final_len),
                    });
                    (final_text, final_len)
                } else {
                    entries.push(LogEntry {
                        t: start.elapsed().as_millis() as u64,
                        level: "info".to_string(),
                        stage: "model".to_string(),
                        msg: format!(
                            "模型完成：{} → {} 字（耗时 {}ms，缩短 {:.1}%，prompt_tokens={}, completion_tokens={}）",
                            after_algo, after_model, model_ms, improvement * 100.0,
                            prompt_tokens, completion_tokens
                        ),
                    });
                    (content_str, after_model)
                }
            }
            Err(e) => {
                let err_msg = e.to_string();
                fallback_triggered = true;
                fallback_reason = Some(format!("模型调用失败：{}", err_msg));
                entries.push(LogEntry {
                    t: start.elapsed().as_millis() as u64,
                    level: "error".to_string(),
                    stage: "fallback".to_string(),
                    msg: format!("模型调用失败，回退算法：{}", err_msg),
                });
                model_call = Some(ModelCallDetail {
                    provider: opts.provider.clone(),
                    model: opts.model.clone(),
                    endpoint,
                    reasoning_effort: reasoning.map(|s| s.to_string()),
                    system_prompt: system,
                    user_prompt: user,
                    input_chars: after_algo,
                    output_chars: 0,
                    elapsed_ms: model_ms,
                    prompt_tokens: 0,
                    completion_tokens: 0,
                    reasoning_tokens: 0,
                    raw_response: String::new(),
                    reasoning_text: None,
                    success: false,
                    error: Some(err_msg),
                });
                let final_text = final_algo_pass(&algo_output, target_chars);
                let final_len = final_text.chars().count();
                entries.push(LogEntry {
                    t: start.elapsed().as_millis() as u64,
                    level: "info".to_string(),
                    stage: "fallback".to_string(),
                    msg: format!("回退二次算法压缩：{} → {} 字", after_algo, final_len),
                });
                (final_text, final_len)
            }
        }
    };

    let total_ms = start.elapsed().as_millis() as u64;
    entries.push(LogEntry {
        t: total_ms,
        level: "info".to_string(),
        stage: "done".to_string(),
        msg: format!(
            "完成：{} → {} 字（{:.1}% of original，耗时 {}ms）",
            original_chars,
            after_model,
            after_model as f32 / original_chars as f32 * 100.0,
            total_ms
        ),
    });

    Ok(CompressResult {
        original: original_chars,
        compressed: after_model,
        ratio: after_model as f32 / original_chars as f32,
        text: final_text,
        text_algo: algo_output,
        stages: Stages {
            after_algo,
            after_model,
        },
        log: RunLog {
            started_at,
            total_ms,
            original_chars,
            final_chars: after_model,
            target_chars,
            algo_target,
            ratio: opts.ratio,
            provider: opts.provider.clone(),
            model: opts.model.clone(),
            no_model: opts.no_model,
            entries,
            model_call,
            fallback_triggered,
            fallback_reason,
        },
    })
}

/// 把 algo 输出（可能 1.5x）进一步压到 final target
/// 复用渐进式管线，保证模型回退时也达标
fn final_algo_pass(text: &str, target_chars: usize) -> String {
    crate::algo::compress(text, target_chars)
}

/// 模型调用结果统一枚举（避免 trait object 限制）
pub enum ModelOutputKind {
    OpenAI(crate::model::openai_compat::ModelOutput),
    Ollama(crate::model::ollama::OllamaOutput),
}

// ==================== 流式压缩 ====================

/// 流式压缩：与 `compress` 逻辑一致，但通过 sink 推送 SSE 事件
/// - 模型阶段调用流式客户端（见 model::openai_compat::compress_stream / model::ollama::compress_stream）
/// - 支持 preset 预设提示词
/// - 支持 text_algo 跳过算法阶段（用于重试）
pub async fn compress_stream(
    text: &str,
    opts: &CompressOptions,
    mut sink: LogSink,
) -> anyhow::Result<CompressResult> {
    let start = Instant::now();
    let started_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    let mut entries: Vec<LogEntry> = Vec::new();
    let mut model_call: Option<ModelCallDetail> = None;
    let mut fallback_triggered = false;
    let mut fallback_reason: Option<String> = None;

    let original_chars = text.chars().count();
    if original_chars == 0 {
        let _ = sink
            .send(StreamEvent::Error {
                msg: "输入为空".to_string(),
                t: 0,
            })
            .await;
        return Ok(CompressResult {
            original: 0,
            compressed: 0,
            ratio: 0.0,
            text: String::new(),
            text_algo: String::new(),
            stages: Stages {
                after_algo: 0,
                after_model: 0,
            },
            log: RunLog {
                started_at,
                total_ms: 0,
                original_chars: 0,
                final_chars: 0,
                target_chars: 0,
                algo_target: 0,
                ratio: 0.0,
                provider: opts.provider.clone(),
                model: opts.model.clone(),
                no_model: opts.no_model,
                entries: vec![LogEntry {
                    t: 0,
                    level: "warn".to_string(),
                    stage: "init".to_string(),
                    msg: "输入为空".to_string(),
                }],
                model_call: None,
                fallback_triggered: false,
                fallback_reason: None,
            },
        });
    }

    entries.push(LogEntry {
        t: 0,
        level: "info".to_string(),
        stage: "init".to_string(),
        msg: format!(
            "开始压缩：原文 {} 字，目标比例 {:.0}%，provider={}, model={}",
            original_chars,
            opts.ratio * 100.0,
            opts.provider,
            opts.model
        ),
    });

    let target_chars = match opts.target_chars {
        Some(t) if t > 0 => t,
        _ => ((original_chars as f32) * opts.ratio).ceil() as usize,
    };
    let target_chars = target_chars.max(1);
    let algo_target = if opts.no_model {
        target_chars
    } else {
        ((target_chars as f32) * 1.5).ceil() as usize
    };

    let _ = sink
        .send(StreamEvent::Init {
            original_chars,
            target_chars,
            algo_target,
            ratio: opts.ratio,
            provider: opts.provider.clone(),
            model: opts.model.clone(),
            no_model: opts.no_model,
            t: 0,
        })
        .await;

    entries.push(LogEntry {
        t: start.elapsed().as_millis() as u64,
        level: "info".to_string(),
        stage: "algo".to_string(),
        msg: format!(
            "算法目标 {} 字（最终目标 {} 字，比例 {:.0}%）",
            algo_target,
            target_chars,
            opts.ratio * 100.0
        ),
    });

    // === Stage 1: 算法压缩 ===
    // text_algo 提供时跳过算法阶段（用于重试）
    let (algo_output, after_algo, _stage_stats) =
        if let Some(text_algo) = opts.text_algo.as_deref() {
            let chars = text_algo.chars().count();
            entries.push(LogEntry {
                t: start.elapsed().as_millis() as u64,
                level: "info".to_string(),
                stage: "algo".to_string(),
                msg: format!("跳过算法阶段（使用 text_algo 直接调模型，{} 字）", chars),
            });
            (text_algo.to_string(), chars, None)
        } else {
            let _ = sink
                .send(StreamEvent::AlgoStart {
                    t: start.elapsed().as_millis() as u64,
                })
                .await;
            let t0 = Instant::now();
            let stage = crate::algo::compress_with_stages(text, algo_target);
            let after_algo = stage.text.chars().count();
            let algo_ms = t0.elapsed().as_millis() as u64;

            let _ = sink
                .send(StreamEvent::AlgoDone {
                    original: original_chars,
                    after_rules: stage.after_rules,
                    after_sentence_compress: stage.after_sentence_compress,
                    after_textrank: stage.after_textrank,
                    used_textrank: stage.used_textrank,
                    elapsed_ms: algo_ms,
                    algo_text: stage.text.clone(),
                    t: start.elapsed().as_millis() as u64,
                })
                .await;

            entries.push(LogEntry {
                t: start.elapsed().as_millis() as u64,
                level: "info".to_string(),
                stage: "algo".to_string(),
                msg: format!(
                    "算法完成：{} → rules {} → sent_comp {} → textrank {} 字（耗时 {}ms，textrank_used={}）",
                    original_chars,
                    stage.after_rules,
                    stage.after_sentence_compress,
                    stage.after_textrank,
                    algo_ms,
                    stage.used_textrank
                ),
            });

            if opts.verbose {
                eprintln!(
                    "[algo] {} → rules {} → sent {} → textrank {} chars (algo_target {}, final_target {}, {:.1}% of original, {}ms, textrank={})",
                    original_chars,
                    stage.after_rules,
                    stage.after_sentence_compress,
                    stage.after_textrank,
                    algo_target,
                    target_chars,
                    after_algo as f32 / original_chars as f32 * 100.0,
                    algo_ms,
                    stage.used_textrank
                );
            }
            (stage.text.clone(), after_algo, Some(stage))
        };

    // === Stage 2: 模型压缩（可选） ===
    let should_run_model = !opts.no_model && after_algo > target_chars && after_algo > 30;

    let (final_text, after_model) = if !should_run_model {
        if opts.no_model {
            entries.push(LogEntry {
                t: start.elapsed().as_millis() as u64,
                level: "info".to_string(),
                stage: "model".to_string(),
                msg: "跳过模型（no_model=true）".to_string(),
            });
        } else if after_algo <= target_chars {
            entries.push(LogEntry {
                t: start.elapsed().as_millis() as u64,
                level: "info".to_string(),
                stage: "model".to_string(),
                msg: format!(
                    "跳过模型：算法输出 {} 字已 ≤ 最终目标 {} 字",
                    after_algo, target_chars
                ),
            });
        } else {
            entries.push(LogEntry {
                t: start.elapsed().as_millis() as u64,
                level: "warn".to_string(),
                stage: "model".to_string(),
                msg: format!("跳过模型：文本过短（{} 字 ≤ 30 字）", after_algo),
            });
        }
        (algo_output.clone(), after_algo)
    } else {
        // 选择提示词：preset 优先，None 视为 "standard"
        let preset_id = opts.preset.as_deref().unwrap_or("standard");
        let preset = crate::prompt::PresetPrompt::from_str(preset_id);
        let (system, user) = crate::prompt::build_compress_messages_preset(
            &algo_output,
            target_chars,
            preset,
            opts.custom_system.as_deref(),
            opts.custom_user_template.as_deref(),
        );
        let reasoning = opts.reasoning_effort.as_deref().filter(|s| !s.is_empty());

        entries.push(LogEntry {
            t: start.elapsed().as_millis() as u64,
            level: "info".to_string(),
            stage: "model".to_string(),
            msg: format!(
                "调用模型：provider={}, model={}, reasoning={:?}, 输入 {} 字，preset={}",
                opts.provider, opts.model, reasoning, after_algo, preset_id
            ),
        });

        let endpoint = match opts.provider.as_str() {
            "deepseek" => "https://api.deepseek.com/chat/completions".to_string(),
            "custom" => {
                let base = opts.base_url.as_deref().unwrap_or("");
                if base.contains("/chat/completions") {
                    base.to_string()
                } else {
                    let trimmed = base.trim_end_matches('/');
                    if trimmed.ends_with("/v1") {
                        format!("{}/chat/completions", trimmed)
                    } else {
                        format!("{}/v1/chat/completions", trimmed)
                    }
                }
            }
            _ => "http://127.0.0.1:11434/api/chat".to_string(),
        };

        let _ = sink
            .send(StreamEvent::ModelStart {
                provider: opts.provider.clone(),
                model: opts.model.clone(),
                reasoning_effort: reasoning.map(|s| s.to_string()),
                input_chars: after_algo,
                system_prompt: system.clone(),
                user_prompt: user.clone(),
                t: start.elapsed().as_millis() as u64,
            })
            .await;

        let t1 = Instant::now();

        // 启动心跳 task：在 ModelStart 后到 ModelDone 之间周期性推送 ModelHeartbeat
        // 让前端知道后端还活着、模型还在加载/推理，避免误以为卡死
        let (heartbeat_stop_tx, mut heartbeat_stop_rx) = mpsc::channel::<()>(1);
        let heartbeat_sink = sink.clone();
        let heartbeat_provider = opts.provider.clone();
        let heartbeat_input = after_algo;
        let heartbeat_start = t1;
        let heartbeat_handle = tokio::spawn(async move {
            // 第一个 tick 立即返回，跳过避免立即推送
            let mut interval = tokio::time::interval(Duration::from_secs(HEARTBEAT_INTERVAL_SEC));
            interval.tick().await;
            loop {
                tokio::select! {
                    biased;
                    _ = heartbeat_stop_rx.recv() => break,
                    _ = interval.tick() => {
                        let elapsed = heartbeat_start.elapsed().as_secs();
                        // Ollama 本地模型三阶段提示：加载 → Prompt 评估 → 推理生成
                        let phase = if heartbeat_provider == "ollama" {
                            if elapsed < HB_OLLAMA_LOAD_TO_EVAL_SEC {
                                format!("模型加载中 · 已等 {}s（首次需把模型从磁盘加载到内存）", elapsed)
                            } else if elapsed < HB_OLLAMA_EVAL_TO_GEN_SEC {
                                format!("Prompt 评估中 · 已等 {}s（输入 {} 字逐 token 评估）", elapsed, heartbeat_input)
                            } else {
                                format!("推理生成中 · 已等 {}s（thinking 模式会先思考再生成，请耐心等）", elapsed)
                            }
                        } else {
                            // deepseek / custom 远程 API
                            if elapsed < HB_API_WAIT_SEC {
                                format!("等待 API 响应 · 已等 {}s", elapsed)
                            } else {
                                format!("API 仍无响应 · 已等 {}s（可能网络慢或排队中）", elapsed)
                            }
                        };
                        let _ = heartbeat_sink
                            .send(StreamEvent::ModelHeartbeat {
                                elapsed_ms: heartbeat_start.elapsed().as_millis() as u64,
                                phase,
                                t: 0,
                            })
                            .await;
                    }
                }
            }
        });

        let model_result: anyhow::Result<ModelOutputKind> = match opts.provider.as_str() {
            "deepseek" => {
                let key = opts.api_key.as_deref().unwrap_or("");
                crate::model::openai_compat::OpenAiCompatClient::deepseek(key, &opts.model)
                    .compress_stream(&system, &user, reasoning, &mut sink)
                    .await
                    .map(ModelOutputKind::OpenAI)
            }
            "custom" => {
                let base = opts.base_url.as_deref().unwrap_or("");
                let key = opts.api_key.as_deref().unwrap_or("");
                if base.is_empty() {
                    Err(anyhow::anyhow!("自定义 provider 需提供 base_url"))
                } else {
                    crate::model::openai_compat::OpenAiCompatClient::from_base_url(base, key, &opts.model)
                        .compress_stream(&system, &user, reasoning, &mut sink)
                        .await
                        .map(ModelOutputKind::OpenAI)
                }
            }
            "ollama" | _ => {
                crate::model::ollama::OllamaClient::new(&opts.model)
                    .compress_stream(&system, &user, &mut sink)
                    .await
                    .map(ModelOutputKind::Ollama)
            }
        };

        // 停止心跳 task
        let _ = heartbeat_stop_tx.send(()).await;
        let _ = heartbeat_handle.await;

        let model_ms = t1.elapsed().as_millis() as u64;

        match model_result {
            Ok(out) => {
                let (content_str, prompt_tokens, completion_tokens, reasoning_tokens, raw_response, reasoning_text) = match &out {
                    ModelOutputKind::OpenAI(o) => (
                        o.content.clone(),
                        o.usage.prompt_tokens,
                        o.usage.completion_tokens,
                        o.usage.reasoning_tokens,
                        o.raw_response.clone(),
                        o.reasoning_text.clone(),
                    ),
                    ModelOutputKind::Ollama(o) => (
                        o.content.clone(),
                        o.prompt_eval_count,
                        o.eval_count,
                        0u32,
                        o.raw_response.clone(),
                        None,
                    ),
                };

                let after_model = content_str.chars().count();
                let improvement = 1.0 - after_model as f32 / after_algo as f32;

                model_call = Some(ModelCallDetail {
                    provider: opts.provider.clone(),
                    model: opts.model.clone(),
                    endpoint,
                    reasoning_effort: reasoning.map(|s| s.to_string()),
                    system_prompt: system.clone(),
                    user_prompt: user.clone(),
                    input_chars: after_algo,
                    output_chars: after_model,
                    elapsed_ms: model_ms,
                    prompt_tokens,
                    completion_tokens,
                    reasoning_tokens,
                    raw_response,
                    reasoning_text,
                    success: true,
                    error: None,
                });

                let _ = sink
                    .send(StreamEvent::ModelDone {
                        output_chars: after_model,
                        elapsed_ms: model_ms,
                        prompt_tokens,
                        completion_tokens,
                        reasoning_tokens,
                        success: true,
                        error: None,
                        t: start.elapsed().as_millis() as u64,
                    })
                    .await;

                if improvement < 0.05 {
                    // 模型未显著缩短 → 回退二次算法
                    fallback_triggered = true;
                    let reason = format!(
                        "模型输出仅缩短 {:.1}%（{} → {} 字），小于 5% 阈值，回退二次算法",
                        improvement * 100.0, after_algo, after_model
                    );
                    tracing::warn!(
                        error_code = ?crate::errors::AppErrorCode::EPipelineImprovementTooLow.code(),
                        after_algo = after_algo,
                        after_model = after_model,
                        improvement = improvement,
                        "模型输出缩短幅度不足 5%，触发 fallback"
                    );
                    fallback_reason = Some(reason.clone());
                    entries.push(LogEntry {
                        t: start.elapsed().as_millis() as u64,
                        level: "warn".to_string(),
                        stage: "fallback".to_string(),
                        msg: reason.clone(),
                    });
                    let _ = sink
                        .send(StreamEvent::Fallback {
                            reason,
                            t: start.elapsed().as_millis() as u64,
                        })
                        .await;
                    let final_text = final_algo_pass(&algo_output, target_chars);
                    let final_len = final_text.chars().count();
                    entries.push(LogEntry {
                        t: start.elapsed().as_millis() as u64,
                        level: "info".to_string(),
                        stage: "fallback".to_string(),
                        msg: format!("回退二次算法压缩：{} → {} 字", after_algo, final_len),
                    });
                    (final_text, final_len)
                } else {
                    entries.push(LogEntry {
                        t: start.elapsed().as_millis() as u64,
                        level: "info".to_string(),
                        stage: "model".to_string(),
                        msg: format!(
                            "模型完成：{} → {} 字（耗时 {}ms，缩短 {:.1}%，prompt_tokens={}, completion_tokens={}）",
                            after_algo, after_model, model_ms, improvement * 100.0,
                            prompt_tokens, completion_tokens
                        ),
                    });
                    (content_str, after_model)
                }
            }
            Err(e) => {
                let err_msg = e.to_string();
                fallback_triggered = true;
                let reason = format!("模型调用失败：{}", err_msg);
                tracing::error!(
                    error_code = ?crate::errors::AppErrorCode::EPipelineFallbackTriggered.code(),
                    error = %err_msg,
                    provider = %opts.provider,
                    model = %opts.model,
                    "模型调用失败，触发 fallback"
                );
                fallback_reason = Some(reason.clone());
                entries.push(LogEntry {
                    t: start.elapsed().as_millis() as u64,
                    level: "error".to_string(),
                    stage: "fallback".to_string(),
                    msg: format!("模型调用失败，回退算法：{}", err_msg),
                });
                let _ = sink
                    .send(StreamEvent::ModelDone {
                        output_chars: 0,
                        elapsed_ms: model_ms,
                        prompt_tokens: 0,
                        completion_tokens: 0,
                        reasoning_tokens: 0,
                        success: false,
                        error: Some(err_msg.clone()),
                        t: start.elapsed().as_millis() as u64,
                    })
                    .await;
                let _ = sink
                    .send(StreamEvent::Fallback {
                        reason,
                        t: start.elapsed().as_millis() as u64,
                    })
                    .await;
                model_call = Some(ModelCallDetail {
                    provider: opts.provider.clone(),
                    model: opts.model.clone(),
                    endpoint,
                    reasoning_effort: reasoning.map(|s| s.to_string()),
                    system_prompt: system,
                    user_prompt: user,
                    input_chars: after_algo,
                    output_chars: 0,
                    elapsed_ms: model_ms,
                    prompt_tokens: 0,
                    completion_tokens: 0,
                    reasoning_tokens: 0,
                    raw_response: String::new(),
                    reasoning_text: None,
                    success: false,
                    error: Some(err_msg),
                });
                let final_text = final_algo_pass(&algo_output, target_chars);
                let final_len = final_text.chars().count();
                entries.push(LogEntry {
                    t: start.elapsed().as_millis() as u64,
                    level: "info".to_string(),
                    stage: "fallback".to_string(),
                    msg: format!("回退二次算法压缩：{} → {} 字", after_algo, final_len),
                });
                (final_text, final_len)
            }
        }
    };

    let total_ms = start.elapsed().as_millis() as u64;
    entries.push(LogEntry {
        t: total_ms,
        level: "info".to_string(),
        stage: "done".to_string(),
        msg: format!(
            "完成：{} → {} 字（{:.1}% of original，耗时 {}ms）",
            original_chars,
            after_model,
            after_model as f32 / original_chars as f32 * 100.0,
            total_ms
        ),
    });

    let _ = sink
        .send(StreamEvent::Done {
            final_text: final_text.clone(),
            final_chars: after_model,
            total_ms,
            t: total_ms,
        })
        .await;

    Ok(CompressResult {
        original: original_chars,
        compressed: after_model,
        ratio: after_model as f32 / original_chars as f32,
        text: final_text,
        text_algo: algo_output,
        stages: Stages {
            after_algo,
            after_model,
        },
        log: RunLog {
            started_at,
            total_ms,
            original_chars,
            final_chars: after_model,
            target_chars,
            algo_target,
            ratio: opts.ratio,
            provider: opts.provider.clone(),
            model: opts.model.clone(),
            no_model: opts.no_model,
            entries,
            model_call,
            fallback_triggered,
            fallback_reason,
        },
    })
}
