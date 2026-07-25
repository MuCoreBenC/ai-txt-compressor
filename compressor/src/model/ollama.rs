//! Ollama HTTP 客户端：调用本地小模型做抽象式压缩
//!
//! 改用 /api/chat 端点（chat messages 格式），
//! 利用 system role 隔离指令，减少 prompt 泄露。

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: Vec<Message<'a>>,
    stream: bool,
    options: Options,
}

#[derive(Serialize)]
struct Message<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Serialize)]
struct Options {
    temperature: f32,
    top_p: f32,
}

#[derive(Deserialize)]
struct ChatResponse {
    message: ResponseMessage,
    #[serde(default)]
    prompt_eval_count: Option<u32>,
    #[serde(default)]
    eval_count: Option<u32>,
}

#[derive(Deserialize)]
struct ResponseMessage {
    content: String,
}

/// Ollama 调用结果（与 openai_compat::ModelOutput 结构对齐）
#[derive(Clone, Serialize)]
pub struct OllamaOutput {
    pub content: String,
    pub prompt_eval_count: u32,
    pub eval_count: u32,
    /// 原始 JSON 响应（截断到 4000 字符）
    pub raw_response: String,
}

pub struct OllamaClient {
    base_url: String,
    model: String,
    client: reqwest::Client,
}

impl OllamaClient {
    pub fn new(model: &str) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(600))
            .build()
            .expect("failed to build reqwest client");
        Self {
            base_url: "http://127.0.0.1:11434".to_string(),
            model: model.to_string(),
            client,
        }
    }

    /// 调用 Ollama chat 接口，system 隔离指令、user 给原文
    /// 返回完整输出（含 token 用量和原始响应）
    pub async fn compress_full(&self, system: &str, user: &str) -> Result<OllamaOutput> {
        let req = ChatRequest {
            model: &self.model,
            messages: vec![
                Message {
                    role: "system",
                    content: system,
                },
                Message {
                    role: "user",
                    content: user,
                },
            ],
            stream: false,
            options: Options {
                temperature: 0.3,
                top_p: 0.9,
            },
        };
        let resp = self
            .client
            .post(format!("{}/api/chat", self.base_url))
            .json(&req)
            .send()
            .await
            .context("Ollama request failed (is `ollama serve` running on 11434?)")?;
        let status = resp.status();
        let raw_text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(anyhow::anyhow!("Ollama 返回 {}：{}", status, raw_text));
        }
        let raw_truncated = if raw_text.len() > 4000 {
            format!("{}…（截断，完整 {} 字符）", &raw_text[..4000], raw_text.len())
        } else {
            raw_text.clone()
        };
        let body: ChatResponse = serde_json::from_str(&raw_text)
            .context(format!("解析 Ollama 响应失败，原始：{}", raw_truncated))?;
        Ok(OllamaOutput {
            content: body.message.content.trim().to_string(),
            prompt_eval_count: body.prompt_eval_count.unwrap_or(0),
            eval_count: body.eval_count.unwrap_or(0),
            raw_response: raw_truncated,
        })
    }

    /// 流式调用 Ollama chat 接口，通过 sink 推送 ModelDelta 事件
    /// 流结束后聚合 content、prompt_eval_count、eval_count、raw_response 返回 OllamaOutput
    pub async fn compress_stream(
        &self,
        system: &str,
        user: &str,
        sink: &mut tokio::sync::mpsc::Sender<crate::pipeline::StreamEvent>,
    ) -> Result<OllamaOutput> {
        use futures::StreamExt;

        let req = ChatRequest {
            model: &self.model,
            messages: vec![
                Message {
                    role: "system",
                    content: system,
                },
                Message {
                    role: "user",
                    content: user,
                },
            ],
            stream: true,
            options: Options {
                temperature: 0.3,
                top_p: 0.9,
            },
        };
        let resp = self
            .client
            .post(format!("{}/api/chat", self.base_url))
            .json(&req)
            .send()
            .await
            .context("Ollama request failed (is `ollama serve` running on 11434?)")?;
        let status = resp.status();
        if !status.is_success() {
            let raw_text = resp.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!("Ollama 返回 {}：{}", status, raw_text));
        }

        let mut stream = resp.bytes_stream();
        let mut content_buf = String::new();
        let mut raw_buf = String::new();
        let mut line_buf = String::new();
        let mut prompt_eval_count: u32 = 0;
        let mut eval_count: u32 = 0;
        // 流式读取容错：单个 chunk 失败不直接返回 Err，记录 warn 后 break 循环
        // 保留已累积的 content_buf，流结束后判断是否完全失败
        let mut chunk_success: bool = false;
        let mut stream_error: Option<String> = None;

        while let Some(chunk_result) = stream.next().await {
            let chunk = match chunk_result {
                Ok(c) => c,
                Err(e) => {
                    // 单个 chunk 读取失败：记录 warn，break 循环保留已累积内容
                    stream_error = Some(format!("读取 Ollama 流失败：{}", e));
                    break;
                }
            };
            let s = match std::str::from_utf8(&chunk) {
                Ok(s) => s,
                Err(e) => {
                    stream_error = Some(format!("Ollama 流响应非 UTF-8：{}", e));
                    break;
                }
            };
            line_buf.push_str(s);
            chunk_success = true;

            // NDJSON：每行一个 JSON 对象
            while let Some(pos) = line_buf.find('\n') {
                let line: String = line_buf.drain(..=pos).collect();
                let line = line.trim_end_matches(['\n', '\r']);
                if line.is_empty() {
                    continue;
                }

                // 累积原始响应（截断 4000 字符）
                if raw_buf.len() < 4000 {
                    let remaining = 4000 - raw_buf.len();
                    if line.len() <= remaining {
                        raw_buf.push_str(line);
                        raw_buf.push('\n');
                    } else {
                        raw_buf.push_str(&line[..remaining]);
                    }
                }

                let value: serde_json::Value = match serde_json::from_str(line) {
                    Ok(v) => v,
                    Err(_) => continue,
                };

                // 提取 message.content（增量）
                if let Some(content) = value["message"]["content"].as_str() {
                    if !content.is_empty() {
                        content_buf.push_str(content);
                        let _ = sink
                            .send(crate::pipeline::StreamEvent::ModelDelta {
                                delta: content.to_string(),
                                t: 0,
                            })
                            .await;
                    }
                }

                // 最后一个 chunk 包含 prompt_eval_count / eval_count
                if let Some(pe) = value["prompt_eval_count"].as_u64() {
                    prompt_eval_count = pe as u32;
                }
                if let Some(ec) = value["eval_count"].as_u64() {
                    eval_count = ec as u32;
                }
            }
        }

        // 流结束后判断：
        // - content_buf 为空且没有任何 chunk 成功 → 返回 Err
        // - content_buf 非空但中途断流 → 正常返回 OllamaOutput，并通过 sink 推送 warn 事件
        if content_buf.trim().is_empty() && !chunk_success {
            return Err(anyhow::anyhow!(
                "Ollama 流式响应为空：{}",
                stream_error.unwrap_or_else(|| "未知错误".to_string())
            ));
        }

        // 中途断流但已有内容：推送 warn 事件（不影响返回值）
        if let Some(err_msg) = stream_error.as_ref() {
            let _ = sink
                .send(crate::pipeline::StreamEvent::Fallback {
                    reason: format!("Ollama 流中途断开，已保留部分结果：{}", err_msg),
                    t: 0,
                })
                .await;
        }

        let raw_truncated = if raw_buf.len() > 4000 {
            format!("{}…（截断，完整 {} 字符）", &raw_buf[..4000], raw_buf.len())
        } else {
            raw_buf
        };

        Ok(OllamaOutput {
            content: content_buf.trim().to_string(),
            prompt_eval_count,
            eval_count,
            raw_response: raw_truncated,
        })
    }

    /// 检查 Ollama 服务可用性
    pub async fn health(&self) -> bool {
        self.client
            .get(format!("{}/api/tags", self.base_url))
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    }
}
