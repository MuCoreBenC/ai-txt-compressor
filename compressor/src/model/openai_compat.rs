//! OpenAI 兼容协议客户端（支持 DeepSeek、OpenAI、Moonshot、SiliconFlow 等）
//!
//! 协议：POST {base_url}/chat/completions
//! 鉴权：Authorization: Bearer <API_KEY>
//!
//! reasoning_effort 支持（部分厂商）：
//! - OpenAI o1/o3: low/medium/high
//! - DeepSeek-reasoner: 不传该字段（用独立模型）
//! - Anthropic Claude thinking: 厂商扩展字段，本实现不涉及

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: Vec<Message<'a>>,
    temperature: f32,
    top_p: f32,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_effort: Option<&'a str>,
    /// DeepSeek V4 thinking mode 参数：{"type": "enabled"} 或 {"type": "disabled"}
    /// 仅当 provider=deepseek 且 model 以 deepseek-v4 开头时附加
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking: Option<ThinkingParam>,
}

/// DeepSeek V4 thinking mode 参数
#[derive(Serialize)]
struct ThinkingParam {
    #[serde(rename = "type")]
    kind: &'static str,
}

#[derive(Serialize)]
struct Message<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
    #[serde(default)]
    usage: Option<Usage>,
}

#[derive(Deserialize)]
struct Choice {
    message: ResponseMessage,
}

#[derive(Deserialize)]
struct ResponseMessage {
    content: String,
}

#[derive(Deserialize, Default, Serialize, Clone)]
pub struct Usage {
    #[serde(default)]
    pub prompt_tokens: u32,
    #[serde(default)]
    pub completion_tokens: u32,
    #[serde(default)]
    pub total_tokens: u32,
    /// 部分厂商（如 DeepSeek-reasoner）返回推理 token 数
    #[serde(default)]
    pub reasoning_tokens: u32,
}

/// 模型调用结果（含原始响应和 token 用量，供日志展示）
#[derive(Clone, Serialize)]
pub struct ModelOutput {
    pub content: String,
    pub usage: Usage,
    /// 原始 JSON 响应（截断到 4000 字符，避免日志过长）
    pub raw_response: String,
    /// DeepSeek V4 thinking mode 的思考链累计文本（仅 thinking enabled 时有值）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_text: Option<String>,
}

pub struct OpenAiCompatClient {
    endpoint: String,
    api_key: String,
    model: String,
    /// 是否为 DeepSeek 官方 provider（用于决定是否附加 thinking 字段）
    is_deepseek: bool,
    client: reqwest::Client,
}

impl OpenAiCompatClient {
    /// 创建通用 OpenAI 兼容客户端
    /// endpoint 应为完整的 chat completions URL，例如：
    /// - DeepSeek: https://api.deepseek.com/chat/completions（不带 /v1）
    /// - OpenAI:   https://api.openai.com/v1/chat/completions
    /// - Moonshot: https://api.moonshot.cn/v1/chat/completions
    /// - 自定义:   用户填写的 base_url + /v1/chat/completions
    pub fn new(endpoint: &str, api_key: &str, model: &str) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(180))
            .build()
            .expect("failed to build reqwest client");
        Self {
            endpoint: endpoint.to_string(),
            api_key: api_key.to_string(),
            model: model.to_string(),
            is_deepseek: false,
            client,
        }
    }

    /// 便捷构造：DeepSeek 官方端点（不带 /v1，V4 模型专用）
    pub fn deepseek(api_key: &str, model: &str) -> Self {
        let mut c = Self::new(
            "https://api.deepseek.com/chat/completions",
            api_key,
            model,
        );
        c.is_deepseek = true;
        c
    }

    /// 从用户填写的 base_url 推导完整 endpoint
    /// 规则：如果已含 /chat/completions 直接用，否则补 /v1/chat/completions
    pub fn from_base_url(base_url: &str, api_key: &str, model: &str) -> Self {
        let endpoint = if base_url.contains("/chat/completions") {
            base_url.to_string()
        } else {
            let trimmed = base_url.trim_end_matches('/');
            if trimmed.ends_with("/v1") {
                format!("{}/chat/completions", trimmed)
            } else {
                format!("{}/v1/chat/completions", trimmed)
            }
        };
        Self::new(&endpoint, api_key, model)
    }

    /// 调用 chat completions，返回完整模型输出（含 token 用量和原始响应）
    /// reasoning_effort: Some("low"/"medium"/"high") 仅对支持的模型生效，其他情况会被忽略
    pub async fn compress_full(
        &self,
        system: &str,
        user: &str,
        reasoning_effort: Option<&str>,
    ) -> Result<ModelOutput> {
        if self.api_key.is_empty() {
            return Err(anyhow!("API key 未配置"));
        }
        // 计算 DeepSeek V4 thinking 参数和 reasoning_effort 映射
        let (thinking, mapped_reasoning) = self.build_thinking_and_effort(reasoning_effort);
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
            temperature: 0.3,
            top_p: 0.9,
            stream: false,
            reasoning_effort: mapped_reasoning,
            thinking,
        };
        let resp = self
            .client
            .post(&self.endpoint)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&req)
            .send()
            .await
            .context(format!(
                "请求 {} 失败（检查网络或 API key）",
                self.endpoint
            ))?;
        let status = resp.status();
        let raw_text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(anyhow!(
                "{} 返回 {}：{}",
                self.endpoint,
                status,
                if raw_text.len() > 400 {
                    format!("{}…", &raw_text[..400])
                } else {
                    raw_text.clone()
                }
            ));
        }
        // 先存原始响应（截断），再解析
        let raw_truncated = if raw_text.len() > 4000 {
            format!("{}…（截断，完整 {} 字符）", &raw_text[..4000], raw_text.len())
        } else {
            raw_text.clone()
        };
        let body: ChatResponse = serde_json::from_str(&raw_text)
            .context(format!("解析响应 JSON 失败，原始：{}", raw_truncated))?;
        let content = body
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("响应无 choices"))?
            .message
            .content
            .trim()
            .to_string();
        let usage = body.usage.unwrap_or_default();
        Ok(ModelOutput {
            content,
            usage,
            raw_response: raw_truncated,
            reasoning_text: None,
        })
    }

    /// 流式调用 chat completions，通过 sink 推送 ModelDelta 事件
    /// 流结束后聚合 content、usage、raw_response 返回 ModelOutput
    pub async fn compress_stream(
        &self,
        system: &str,
        user: &str,
        reasoning_effort: Option<&str>,
        sink: &mut tokio::sync::mpsc::Sender<crate::pipeline::StreamEvent>,
    ) -> Result<ModelOutput> {
        use futures::StreamExt;

        if self.api_key.is_empty() {
            return Err(anyhow!("API key 未配置"));
        }
        // 计算 DeepSeek V4 thinking 参数和 reasoning_effort 映射
        let (thinking, mapped_reasoning) = self.build_thinking_and_effort(reasoning_effort);
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
            temperature: 0.3,
            top_p: 0.9,
            stream: true,
            reasoning_effort: mapped_reasoning,
            thinking,
        };
        let resp = self
            .client
            .post(&self.endpoint)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&req)
            .send()
            .await
            .context(format!(
                "请求 {} 失败（检查网络或 API key）",
                self.endpoint
            ))?;
        let status = resp.status();
        if !status.is_success() {
            let raw_text = resp.text().await.unwrap_or_default();
            return Err(anyhow!(
                "{} 返回 {}：{}",
                self.endpoint,
                status,
                if raw_text.len() > 400 {
                    format!("{}…", &raw_text[..400])
                } else {
                    raw_text.clone()
                }
            ));
        }

        let mut stream = resp.bytes_stream();
        let mut content_buf = String::new();
        let mut reasoning_buf = String::new();
        let mut raw_buf = String::new();
        let mut line_buf = String::new();
        let mut usage = Usage::default();

        while let Some(chunk_result) = stream.next().await {
            let chunk = chunk_result.context("读取流失败")?;
            let s = std::str::from_utf8(&chunk).context("流响应非 UTF-8")?;
            line_buf.push_str(s);

            // 按行处理 SSE
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

                // 解析 SSE data 行
                let data = if let Some(stripped) = line.strip_prefix("data: ") {
                    stripped
                } else if let Some(stripped) = line.strip_prefix("data:") {
                    stripped
                } else {
                    continue;
                };
                let data = data.trim();
                if data.is_empty() || data == "[DONE]" {
                    continue;
                }

                let value: serde_json::Value = match serde_json::from_str(data) {
                    Ok(v) => v,
                    Err(_) => continue,
                };

                // 提取 delta.content（最终压缩结果增量，推送给前端结果文本框）
                if let Some(content) = value["choices"][0]["delta"]["content"].as_str() {
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

                // 提取 delta.reasoning_content（DeepSeek V4 thinking mode 的思考链增量）
                // 与 content 平级，通过 ModelReasoning 事件实时推送给前端思考面板
                if let Some(reasoning) = value["choices"][0]["delta"]["reasoning_content"].as_str() {
                    if !reasoning.is_empty() {
                        reasoning_buf.push_str(reasoning);
                        let _ = sink
                            .send(crate::pipeline::StreamEvent::ModelReasoning {
                                delta: reasoning.to_string(),
                                t: 0,
                            })
                            .await;
                    }
                }

                // 提取 usage（部分厂商在最后一个 chunk 包含）
                if let Some(usage_obj) = value.get("usage") {
                    if let Some(pt) = usage_obj["prompt_tokens"].as_u64() {
                        usage.prompt_tokens = pt as u32;
                    }
                    if let Some(ct) = usage_obj["completion_tokens"].as_u64() {
                        usage.completion_tokens = ct as u32;
                    }
                    if let Some(tt) = usage_obj["total_tokens"].as_u64() {
                        usage.total_tokens = tt as u32;
                    }
                    if let Some(rt) = usage_obj["reasoning_tokens"].as_u64() {
                        usage.reasoning_tokens = rt as u32;
                    }
                }
            }
        }

        let raw_truncated = if raw_buf.len() > 4000 {
            format!("{}…（截断，完整 {} 字符）", &raw_buf[..4000], raw_buf.len())
        } else {
            raw_buf
        };

        let reasoning_text = if reasoning_buf.is_empty() {
            None
        } else {
            Some(reasoning_buf)
        };

        Ok(ModelOutput {
            content: content_buf.trim().to_string(),
            usage,
            raw_response: raw_truncated,
            reasoning_text,
        })
    }

    /// 根据 provider 类型和模型名计算 thinking 参数和映射后的 reasoning_effort
    ///
    /// DeepSeek V4 模型（model 以 `deepseek-v4` 开头）：
    /// - `reasoning_effort` 为 `None`/`""`/`"none"`：thinking 设为 `disabled`，不传 reasoning_effort
    /// - `reasoning_effort` 为 `"low"`/`"medium"`：thinking 设为 `enabled`，reasoning_effort 映射为 `"high"`
    /// - `reasoning_effort` 为 `"xhigh"`：thinking 设为 `enabled`，reasoning_effort 映射为 `"max"`
    /// - `reasoning_effort` 为 `"high"`/`"max"`：thinking 设为 `enabled`，reasoning_effort 保持不变
    /// - 其他值：thinking 设为 `enabled`，reasoning_effort 原样传给厂商
    ///
    /// 非 DeepSeek 或非 V4 模型：不附加 thinking 字段，reasoning_effort 原样返回
    fn build_thinking_and_effort<'a>(
        &self,
        reasoning_effort: Option<&'a str>,
    ) -> (Option<ThinkingParam>, Option<&'a str>) {
        if self.is_deepseek && self.model.starts_with("deepseek-v4") {
            match reasoning_effort {
                None | Some("") | Some("none") => (
                    Some(ThinkingParam { kind: "disabled" }),
                    None,
                ),
                Some("low") | Some("medium") => (
                    Some(ThinkingParam { kind: "enabled" }),
                    Some("high"),
                ),
                Some("xhigh") => (
                    Some(ThinkingParam { kind: "enabled" }),
                    Some("max"),
                ),
                Some(other) => (
                    Some(ThinkingParam { kind: "enabled" }),
                    Some(other),
                ),
            }
        } else {
            (None, reasoning_effort)
        }
    }
}
