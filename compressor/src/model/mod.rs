//! 模型层：支持多 provider（Ollama 本地 / DeepSeek API）
//!
//! 实际 provider 调度见 pipeline.rs

pub mod loop_detector;
pub mod ollama;
pub mod openai_compat;
