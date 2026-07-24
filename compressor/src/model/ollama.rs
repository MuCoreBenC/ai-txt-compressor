//! Ollama HTTP 客户端：调用本地小模型做抽象式压缩

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Serialize)]
struct GenerateRequest<'a> {
    model: &'a str,
    prompt: &'a str,
    stream: bool,
    options: Options,
}

#[derive(Serialize)]
struct Options {
    temperature: f32,
    top_p: f32,
}

#[derive(Deserialize)]
struct GenerateResponse {
    response: String,
}

pub struct OllamaClient {
    base_url: String,
    model: String,
    client: reqwest::Client,
}

impl OllamaClient {
    pub fn new(model: &str) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .expect("failed to build reqwest client");
        Self {
            base_url: "http://127.0.0.1:11434".to_string(),
            model: model.to_string(),
            client,
        }
    }

    /// 调用 Ollama 把 text 压缩到 target_chars 字以内
    pub async fn compress(&self, text: &str, target_chars: usize) -> Result<String> {
        let prompt = crate::prompt::build_compress_prompt(text, target_chars);
        let req = GenerateRequest {
            model: &self.model,
            prompt: &prompt,
            stream: false,
            options: Options {
                temperature: 0.3,
                top_p: 0.9,
            },
        };
        let resp = self
            .client
            .post(format!("{}/api/generate", self.base_url))
            .json(&req)
            .send()
            .await
            .context("Ollama request failed (is `ollama serve` running on 11434?)")?;
        let body: GenerateResponse = resp
            .json()
            .await
            .context("Failed to parse Ollama response")?;
        Ok(body.response.trim().to_string())
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
