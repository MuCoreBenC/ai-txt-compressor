//! Ollama HTTP 客户端：调用本地小模型做抽象式压缩
//!
//! 改用 /api/chat 端点（chat messages 格式），
//! 利用 system role 隔离指令，减少 prompt 泄露。

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Qwen3 thinking 标签解析状态机
/// 处理流式到达的 content，分离开标签和闭标签之间的思考链
/// 支持跨 chunk 边界的标签（如 `<th` + `ink>`）
pub struct ThinkingParser {
    /// 当前是否在思考链内
    in_thinking: bool,
    /// 缓冲区：用于处理跨 chunk 的标签
    buf: String,
    /// 累积的实际输出增量（待 take_output 取走）
    output_buf: String,
    /// 累积的思考链增量（待 take_reasoning 取走）
    reasoning_buf: String,
    /// 累积的实际输出全文（用于最终返回值）
    output_full: String,
}

impl ThinkingParser {
    pub fn new() -> Self {
        Self {
            in_thinking: false,
            buf: String::new(),
            output_buf: String::new(),
            reasoning_buf: String::new(),
            output_full: String::new(),
        }
    }

    /// 喂入一段 content，更新内部状态
    /// 算法：维护一个 buf，扫描开标签和闭标签
    /// 标签之前/之外的内容加入 output_buf 或 reasoning_buf（视当前状态）
    /// 标签本身不输出，只切换状态
    /// 当标签跨 chunk 时，保留 buf 中可能是标签前缀的尾部，其余全部输出
    pub fn feed(&mut self, content: &str) {
        self.buf.push_str(content);
        loop {
            if self.in_thinking {
                // 寻找闭标签（用 Unicode 转义避免被渲染）
                let close_tag = "\u{3c}/think\u{3e}";
                if let Some(pos) = self.buf.find(close_tag) {
                    // 闭标签之前的内容是思考链
                    self.reasoning_buf.push_str(&self.buf[..pos]);
                    // 跳过闭标签
                    self.buf.drain(..pos + close_tag.len());
                    self.in_thinking = false;
                    // 继续循环处理剩余
                } else {
                    // 没找到闭标签，可能标签跨 chunk
                    // 保留 buf 末尾"可能是标签前缀"的部分，其余全部加入 reasoning_buf
                    let safe_end = safe_emit_len(&self.buf, close_tag);
                    if safe_end > 0 {
                        self.reasoning_buf.push_str(&self.buf[..safe_end]);
                        self.buf.drain(..safe_end);
                    }
                    break;
                }
            } else {
                // 寻找开标签（用 Unicode 转义避免被渲染）
                let open_tag = "\u{3c}think\u{3e}";
                if let Some(pos) = self.buf.find(open_tag) {
                    // 开标签之前的内容是实际输出
                    self.output_buf.push_str(&self.buf[..pos]);
                    self.output_full.push_str(&self.buf[..pos]);
                    // 跳过开标签
                    self.buf.drain(..pos + open_tag.len());
                    self.in_thinking = true;
                    // 继续循环处理剩余
                } else {
                    // 没找到开标签，可能标签跨 chunk
                    // 保留 buf 末尾"可能是标签前缀"的部分，其余全部加入 output_buf
                    let safe_end = safe_emit_len(&self.buf, open_tag);
                    if safe_end > 0 {
                        self.output_buf.push_str(&self.buf[..safe_end]);
                        self.output_full.push_str(&self.buf[..safe_end]);
                        self.buf.drain(..safe_end);
                    }
                    break;
                }
            }
        }
    }

    /// 取出累积的实际输出增量
    pub fn take_output(&mut self) -> String {
        std::mem::take(&mut self.output_buf)
    }

    /// 取出累积的思考链增量
    pub fn take_reasoning(&mut self) -> String {
        std::mem::take(&mut self.reasoning_buf)
    }

    /// 流结束时把 buf 残余内容按当前状态刷出
    pub fn flush(&mut self) {
        if self.in_thinking {
            // thinking 未闭合，把残余当思考链
            self.reasoning_buf.push_str(&self.buf);
        } else {
            self.output_buf.push_str(&self.buf);
            self.output_full.push_str(&self.buf);
        }
        self.buf.clear();
    }
}

/// 计算 buf 中可以安全输出的最大前缀长度（char boundary 对齐）
/// 算法：找到最小的 split_point（必须是 char boundary），使得 buf[split_point..]
///      是 tag 的非空前缀；若不存在这样的 split_point，返回 buf.len()（全部输出）
/// 这样保证保留最长的"可能是标签开头"的后缀，其余立即输出，避免视觉卡顿
fn safe_emit_len(buf: &str, tag: &str) -> usize {
    for (idx, _) in buf.char_indices() {
        let suffix = &buf[idx..];
        if !suffix.is_empty() && tag.starts_with(suffix) {
            return idx;
        }
    }
    buf.len()
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: Vec<Message<'a>>,
    stream: bool,
    options: Options,
    /// Ollama keep_alive 参数：模型在内存中保持的时间，避免每次重新加载
    /// 默认 5m，这里设为 30m 让模型常驻内存，第二次调用起跳过加载阶段
    #[serde(rename = "keep_alive")]
    keep_alive: &'a str,
    /// Ollama think 字段（0.17.6+）：可选 bool 或 level 字符串（"low"/"medium"/"high"/"max"）
    /// - Some(Bool(false)) → 关闭思考（对应 /set nothink）
    /// - Some(Bool(true)) → 开启思考（默认强度）
    /// - Some(Level("high"/"max")) → 指定思考强度
    /// - None → 不传，用模型默认
    #[serde(skip_serializing_if = "Option::is_none")]
    think: Option<ThinkParam>,
}

/// think 参数：支持 bool 或 level 字符串（untagged 序列化为 JSON 原始值）
#[derive(Serialize)]
#[serde(untagged)]
enum ThinkParam {
    Bool(bool),
    Level(String),
}

/// 把 reasoning 字符串映射到 Ollama think 参数
/// - "none" → Bool(false) 关闭思考
/// - "high"/"max"/"medium"/"low" → Level(原值) 指定思考强度
/// - 其他自定义值（如 "xhigh"）→ Level(原值)
/// - None → None 不传 think，用模型默认
fn reasoning_to_think(reasoning: Option<&str>) -> Option<ThinkParam> {
    match reasoning {
        Some("none") => Some(ThinkParam::Bool(false)),
        Some(level) => Some(ThinkParam::Level(level.to_string())),
        None => None,
    }
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
    /// 重复惩罚因子（Ollama repeat_penalty）：>1 降低已出现 token 的概率
    /// 默认 1.0 不惩罚，1.1-1.2 抑制重复，>1.2 可能导致语法崩溃
    /// Qwen3 thinking 模式特别容易陷入循环，设为 1.15 从源头减少重复
    repeat_penalty: f32,
    /// 重复惩罚回溯窗口（Ollama repeat_last_n）：模型回头看多少个 token 判断重复
    /// 默认 64 太短，设为 512 让惩罚覆盖更长的思考链
    repeat_last_n: i32,
}

#[derive(Deserialize)]
struct ChatResponse {
    message: ResponseMessage,
    #[serde(default)]
    prompt_eval_count: Option<u32>,
    #[serde(default)]
    eval_count: Option<u32>,
    /// Ollama 统计字段（非流式响应末尾返回，单位纳秒）
    #[serde(default)]
    total_duration: Option<u64>,
    #[serde(default)]
    load_duration: Option<u64>,
    #[serde(default)]
    prompt_eval_duration: Option<u64>,
    #[serde(default)]
    eval_duration: Option<u64>,
}

#[derive(Deserialize)]
struct ResponseMessage {
    /// 实际输出内容（think 模式下为纯输出，无 <think> 标签）
    #[serde(default)]
    content: String,
    /// 思考内容（Ollama 0.17+ think 模式下与 content 同级返回，流式增量）
    /// 老版本或 think:false 时为 None
    #[serde(default)]
    thinking: Option<String>,
}

/// Ollama 性能统计（纳秒，前端转换为可读单位）
#[derive(Debug, Clone, Serialize, Default)]
pub struct OllamaTiming {
    /// 总耗时（纳秒）
    pub total_duration_ns: u64,
    /// 模型加载耗时（纳秒）
    pub load_duration_ns: u64,
    /// prompt 评估耗时（纳秒）
    pub prompt_eval_duration_ns: u64,
    /// 生成耗时（纳秒）
    pub eval_duration_ns: u64,
}

impl OllamaTiming {
    /// 计算生成速率（tokens/s），eval_duration 为纳秒
    pub fn eval_rate(&self, eval_count: u32) -> Option<f64> {
        if self.eval_duration_ns == 0 { return None; }
        let secs = self.eval_duration_ns as f64 / 1_000_000_000.0;
        if secs <= 0.0 { return None; }
        Some(eval_count as f64 / secs)
    }
    /// 计算 prompt 评估速率（tokens/s）
    pub fn prompt_eval_rate(&self, prompt_eval_count: u32) -> Option<f64> {
        if self.prompt_eval_duration_ns == 0 { return None; }
        let secs = self.prompt_eval_duration_ns as f64 / 1_000_000_000.0;
        if secs <= 0.0 { return None; }
        Some(prompt_eval_count as f64 / secs)
    }
}

/// Ollama 调用结果（与 openai_compat::ModelOutput 结构对齐）
#[derive(Clone, Serialize)]
pub struct OllamaOutput {
    pub content: String,
    pub prompt_eval_count: u32,
    pub eval_count: u32,
    /// 原始 JSON 响应（截断到 4000 字符）
    pub raw_response: String,
    /// 性能统计（流式从最后一个 chunk 提取，非流式从响应体提取）
    #[serde(default)]
    pub timing: OllamaTiming,
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

    /// 测试用构造函数：注入自定义 base_url（用于 mockito 集成测试）
    pub fn with_base_url(base_url: &str, model: &str) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("failed to build reqwest client");
        Self {
            base_url: base_url.to_string(),
            model: model.to_string(),
            client,
        }
    }

    /// 卸载模型：调用 Ollama /api/generate 传 keep_alive=0 让模型从内存释放
    /// 错误码：E_OLLAMA_UNLOAD_FAILED (1007)
    pub async fn unload_model(&self) -> Result<()> {
        let body = serde_json::json!({
            "model": self.model,
            "keep_alive": 0
        });
        let resp = self
            .client
            .post(format!("{}/api/generate", self.base_url))
            .json(&body)
            .send()
            .await
            .context(format!(
                "卸载模型失败 (E1007): 无法连接 Ollama ({})",
                self.base_url
            ))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            tracing::error!(
                error_code = ?crate::errors::AppErrorCode::EOllamaUnloadFailed.code(),
                status = status.as_u16(),
                model = %self.model,
                body = %body,
                "Ollama 模型卸载失败"
            );
            return Err(anyhow::anyhow!(
                "卸载模型失败 (E1007): Ollama 返回 {} - {}",
                status,
                body
            ));
        }
        tracing::info!(model = %self.model, "Ollama 模型已卸载");
        Ok(())
    }

    /// 查询模型是否已加载到内存（Ollama /api/ps 接口）
    /// 用于避免第二次调用时仍显示"模型加载中"误导用户
    /// 返回：
    /// - Ok(true)  : 模型在 /api/ps 的 models 数组中
    /// - Ok(false) : 模型未加载，或 /api/ps 返回非 200
    /// - Err(_)    : 网络错误或响应解析失败
    pub async fn is_model_loaded(&self) -> Result<bool> {
        let resp = self
            .client
            .get(format!("{}/api/ps", self.base_url))
            .send()
            .await
            .context(format!(
                "查询 Ollama /api/ps 失败: 无法连接 ({})",
                self.base_url
            ))?;
        if !resp.status().is_success() {
            // /api/ps 返回 5xx 时不报错，默认未加载
            tracing::warn!(
                status = resp.status().as_u16(),
                "Ollama /api/ps 返回非 200，默认未加载"
            );
            return Ok(false);
        }
        let v: serde_json::Value = resp
            .json()
            .await
            .context("解析 Ollama /api/ps 响应失败")?;
        // 兼容 name 字段（旧版 Ollama）和 model 字段（新版 Ollama）
        let loaded = v["models"]
            .as_array()
            .map(|arr| {
                arr.iter().any(|m| {
                    m["name"].as_str() == Some(self.model.as_str())
                        || m["model"].as_str() == Some(self.model.as_str())
                })
            })
            .unwrap_or(false);
        Ok(loaded)
    }

    /// 调用 Ollama chat 接口，system 隔离指令、user 给原文
    /// 返回完整输出（含 token 用量和原始响应）
    /// reasoning 映射到 Ollama think 字段：
    /// - Some("none") → think:false（关闭思考，对应 /set nothink）
    /// - Some("high"/"max"/...) → think:"high"/"max"/...（指定思考强度）
    /// - None → 不传 think，用模型默认
    pub async fn compress_full(&self, system: &str, user: &str, reasoning: Option<&str>) -> Result<OllamaOutput> {
        let think = reasoning_to_think(reasoning);
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
                repeat_penalty: 1.15,
                repeat_last_n: 512,
            },
            keep_alive: "30m",
            think,
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
        // think 模式下 content 是纯输出（无 <think> 标签），thinking 字段是思考内容
        // 兼容老版本：若 thinking 字段为空，content 可能含 <think> 标签，仍用 ThinkingParser 清洗
        let clean_content = if body.message.thinking.is_some() {
            // 新版 ollama：thinking 在独立字段，content 是纯输出
            body.message.content.trim().to_string()
        } else {
            // 老版本或 think:false：content 可能含 <think> 标签，用 ThinkingParser 清洗
            let mut parser = ThinkingParser::new();
            parser.feed(&body.message.content);
            parser.flush();
            parser.output_full.trim().to_string()
        };
        Ok(OllamaOutput {
            content: clean_content,
            prompt_eval_count: body.prompt_eval_count.unwrap_or(0),
            eval_count: body.eval_count.unwrap_or(0),
            raw_response: raw_truncated,
            timing: OllamaTiming {
                total_duration_ns: body.total_duration.unwrap_or(0),
                load_duration_ns: body.load_duration.unwrap_or(0),
                prompt_eval_duration_ns: body.prompt_eval_duration.unwrap_or(0),
                eval_duration_ns: body.eval_duration.unwrap_or(0),
            },
        })
    }

    /// 流式调用 Ollama chat 接口，通过 sink 推送 ModelDelta 事件
    /// 流结束后聚合 content、prompt_eval_count、eval_count、raw_response 返回 OllamaOutput
    /// reasoning 映射到 Ollama think 字段：
    /// - Some("none") → think:false（关闭思考）
    /// - Some("high"/"max"/...) → think:"high"/"max"/...（开启思考，思考内容在 message.thinking 字段）
    /// - None → 不传 think，用模型默认
    pub async fn compress_stream(
        &self,
        system: &str,
        user: &str,
        reasoning: Option<&str>,
        sink: &mut tokio::sync::mpsc::Sender<crate::pipeline::StreamEvent>,
        cancel: tokio_util::sync::CancellationToken,
    ) -> Result<OllamaOutput> {
        use futures::StreamExt;

        let think = reasoning_to_think(reasoning);
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
                repeat_penalty: 1.15,
                repeat_last_n: 512,
            },
            keep_alive: "30m",
            think,
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
        // Ollama 统计字段（最后一个 chunk 才返回，单位纳秒）
        let mut total_duration_ns: u64 = 0;
        let mut load_duration_ns: u64 = 0;
        let mut prompt_eval_duration_ns: u64 = 0;
        let mut eval_duration_ns: u64 = 0;
        // thinking 标签解析状态机
        let mut thinking_parser = ThinkingParser::new();
        // 循环检测器：
        // - content 用 N-gram 检测（短片段重复）
        // - thinking 用长段落哈希检测（整段重复，避免误判正常推导）
        let mut content_loop_detector = crate::model::loop_detector::LoopDetector::new();
        let mut thinking_loop_detector = crate::model::loop_detector::ThinkingLoopDetector::new();
        let mut loop_detected = false;
        // 流式读取容错：单个 chunk 失败不直接返回 Err，记录 warn 后 break 循环
        // 保留已累积的 content_buf，流结束后判断是否完全失败
        let mut chunk_success: bool = false;
        let mut stream_error: Option<String> = None;

        loop {
            tokio::select! {
                biased;
                _ = cancel.cancelled() => {
                    // 用户取消：break 循环，drop stream 关闭 HTTP 连接（Ollama 会感知客户端断开停止生成）
                    tracing::info!(model = %self.model, "Ollama 流式调用被用户取消");
                    break;
                }
                chunk_result = stream.next() => {
                    match chunk_result {
                        None => break,
                        Some(Err(e)) => {
                            stream_error = Some(format!("读取 Ollama 流失败：{}", e));
                            break;
                        }
                        Some(Ok(c)) => { let chunk = c;
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

                // 提取 message.thinking（思考链增量，Ollama 0.17+ think 模式下与 content 同级）
                if let Some(thinking) = value["message"]["thinking"].as_str() {
                    if !thinking.is_empty() {
                        // 思考链用长段落哈希检测：整段 200 字符完全相同才计数
                        // 正常推导不会整段雷同，只有 Qwen3 那种几百字反复输出的循环才会触发
                        if thinking_loop_detector.feed(thinking) {
                            tracing::warn!(model = %self.model, "检测到 thinking 整段重复循环，主动中断");
                            let _ = sink
                                .send(crate::pipeline::StreamEvent::ModelLoopDetected { t: 0 })
                                .await;
                            loop_detected = true;
                            break;
                        }
                        let _ = sink
                            .send(crate::pipeline::StreamEvent::ModelReasoning {
                                delta: thinking.to_string(),
                                t: 0,
                            })
                            .await;
                    }
                }

                // 提取 message.content（实际输出增量）
                if let Some(content) = value["message"]["content"].as_str() {
                    if !content.is_empty() {
                        // 循环检测：content 重复输出同一段内容时主动中断
                        if content_loop_detector.feed(content) {
                            tracing::warn!(model = %self.model, "检测到 content 输出循环，主动中断");
                            let _ = sink
                                .send(crate::pipeline::StreamEvent::ModelLoopDetected { t: 0 })
                                .await;
                            loop_detected = true;
                            break;
                        }
                        content_buf.push_str(content);
                        thinking_parser.feed(content);
                        let output_delta = thinking_parser.take_output();
                        if !output_delta.is_empty() {
                            let _ = sink
                                .send(crate::pipeline::StreamEvent::ModelDelta {
                                    delta: output_delta,
                                    t: 0,
                                })
                                .await;
                        }
                        let reasoning_delta = thinking_parser.take_reasoning();
                        if !reasoning_delta.is_empty() {
                            let _ = sink
                                .send(crate::pipeline::StreamEvent::ModelReasoning {
                                    delta: reasoning_delta,
                                    t: 0,
                                })
                                .await;
                        }
                    }
                }

                if let Some(pe) = value["prompt_eval_count"].as_u64() {
                    prompt_eval_count = pe as u32;
                }
                if let Some(ec) = value["eval_count"].as_u64() {
                    eval_count = ec as u32;
                }
                if let Some(v) = value["total_duration"].as_u64() { total_duration_ns = v; }
                if let Some(v) = value["load_duration"].as_u64() { load_duration_ns = v; }
                if let Some(v) = value["prompt_eval_duration"].as_u64() { prompt_eval_duration_ns = v; }
                if let Some(v) = value["eval_duration"].as_u64() { eval_duration_ns = v; }
            }
            if loop_detected {
                break;
            }
                        }
                    }
                }
            }
        }

        if loop_detected {
            tracing::info!(model = %self.model, "流式调用因循环检测提前终止，已接收 content {} 字符",
                content_buf.chars().count());
        }

        // 流结束：刷出 thinking_parser 残余内容
        thinking_parser.flush();
        // 把残余实际输出推送给前端
        let tail_output = thinking_parser.take_output();
        if !tail_output.is_empty() {
            let _ = sink
                .send(crate::pipeline::StreamEvent::ModelDelta {
                    delta: tail_output,
                    t: 0,
                })
                .await;
        }
        // 把残余思考链推送给前端
        let tail_reasoning = thinking_parser.take_reasoning();
        if !tail_reasoning.is_empty() {
            let _ = sink
                .send(crate::pipeline::StreamEvent::ModelReasoning {
                    delta: tail_reasoning,
                    t: 0,
                })
                .await;
        }

        // 流结束后判断：
        // - content_buf 为空且没有任何 chunk 成功 → 返回 Err
        // - content_buf 非空但中途断流 → 正常返回 OllamaOutput，并通过 sink 推送 warn 事件
        if content_buf.trim().is_empty() && !chunk_success {
            let err = anyhow::anyhow!(
                "Ollama 流式响应为空：{}",
                stream_error.unwrap_or_else(|| "未知错误".to_string())
            );
            tracing::error!(
                error_code = ?crate::errors::AppErrorCode::EOllamaStreamEmpty.code(),
                error = %err,
                model = %self.model,
                "Ollama 流式响应为空"
            );
            return Err(err);
        }

        // 中途断流但已有内容：推送 warn 事件（不影响返回值）
        if let Some(err_msg) = stream_error.as_ref() {
            tracing::warn!(
                error_code = ?crate::errors::AppErrorCode::EOllamaStreamBreak.code(),
                error = %err_msg,
                model = %self.model,
                content_buf_len = content_buf.len(),
                "Ollama 流中途断开，已保留部分结果"
            );
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

        // content 用 thinking_parser 分离后的实际输出（去除思考链）
        let final_content = if thinking_parser.output_full.trim().is_empty() {
            content_buf.trim().to_string()
        } else {
            thinking_parser.output_full.trim().to_string()
        };

        Ok(OllamaOutput {
            content: final_content,
            prompt_eval_count,
            eval_count,
            raw_response: raw_truncated,
            timing: OllamaTiming {
                total_duration_ns,
                load_duration_ns,
                prompt_eval_duration_ns,
                eval_duration_ns,
            },
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
