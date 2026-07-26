//! 应用错误码体系
//!
//! 用于日志结构化输出和前端展示。
//! 命名规范：E_<模块>_<具体场景>，数字编号按模块段分配：
//!   - 1xxx: Ollama
//!   - 2xxx: OpenAI 兼容 API
//!   - 3xxx: Pipeline
//!   - 9xxx: 通用/未知

use serde::Serialize;

/// 应用错误码枚举
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AppErrorCode {
    // Ollama 相关（1xxx）
    EOllamaStreamBreak,
    EOllamaStreamEmpty,
    EOllamaRequestFailed,
    EOllamaBadRequest,
    EOllamaServerError,
    EOllamaModelNotFound,
    EOllamaUnloadFailed,

    // OpenAI 兼容相关（2xxx）
    EOpenAiRequestFailed,
    EOpenAiBadRequest,
    EOpenAiUnauthorized,
    EOpenAiRateLimit,
    EOpenAiServerError,

    // Pipeline 相关（3xxx）
    EPipelineFallbackTriggered,
    EPipelineImprovementTooLow,

    // 通用（9xxx）
    EUnknown,
}

impl AppErrorCode {
    /// 数字编号
    pub fn code(&self) -> u16 {
        match self {
            AppErrorCode::EOllamaStreamBreak => 1001,
            AppErrorCode::EOllamaStreamEmpty => 1002,
            AppErrorCode::EOllamaRequestFailed => 1003,
            AppErrorCode::EOllamaBadRequest => 1004,
            AppErrorCode::EOllamaServerError => 1005,
            AppErrorCode::EOllamaModelNotFound => 1006,
            AppErrorCode::EOllamaUnloadFailed => 1007,
            AppErrorCode::EOpenAiRequestFailed => 2001,
            AppErrorCode::EOpenAiBadRequest => 2002,
            AppErrorCode::EOpenAiUnauthorized => 2003,
            AppErrorCode::EOpenAiRateLimit => 2004,
            AppErrorCode::EOpenAiServerError => 2005,
            AppErrorCode::EPipelineFallbackTriggered => 3001,
            AppErrorCode::EPipelineImprovementTooLow => 3002,
            AppErrorCode::EUnknown => 9999,
        }
    }

    /// 人类可读描述
    pub fn description(&self) -> &'static str {
        match self {
            AppErrorCode::EOllamaStreamBreak => "Ollama 流式响应中途断开",
            AppErrorCode::EOllamaStreamEmpty => "Ollama 流式响应为空",
            AppErrorCode::EOllamaRequestFailed => "Ollama HTTP 请求失败",
            AppErrorCode::EOllamaBadRequest => "Ollama 返回 4xx 错误",
            AppErrorCode::EOllamaServerError => "Ollama 返回 5xx 错误",
            AppErrorCode::EOllamaModelNotFound => "Ollama 模型不存在",
            AppErrorCode::EOllamaUnloadFailed => "Ollama 模型卸载失败",
            AppErrorCode::EOpenAiRequestFailed => "OpenAI 兼容 API 请求失败",
            AppErrorCode::EOpenAiBadRequest => "OpenAI 兼容 API 返回 4xx",
            AppErrorCode::EOpenAiUnauthorized => "API key 无效或未授权",
            AppErrorCode::EOpenAiRateLimit => "API 限流",
            AppErrorCode::EOpenAiServerError => "OpenAI 兼容 API 返回 5xx",
            AppErrorCode::EPipelineFallbackTriggered => "Pipeline 触发 fallback",
            AppErrorCode::EPipelineImprovementTooLow => "模型输出缩短幅度不足 5%",
            AppErrorCode::EUnknown => "未知错误",
        }
    }

    /// 用于日志的字符串标识（如 "E_OLLAMA_STREAM_BREAK (1001)"）
    pub fn log_tag(&self) -> String {
        format!("{:?} ({})", self, self.code())
    }
}

/// Ollama HTTP 状态码到 AppErrorCode 的映射
pub struct OllamaHttpStatus;

impl OllamaHttpStatus {
    pub fn from_status(status: u16) -> AppErrorCode {
        match status {
            200..=299 => AppErrorCode::EUnknown, // 成功状态不应映射到错误码
            400 => AppErrorCode::EOllamaBadRequest,
            404 => AppErrorCode::EOllamaModelNotFound,
            500..=599 => AppErrorCode::EOllamaServerError,
            _ => AppErrorCode::EOllamaRequestFailed,
        }
    }
}

/// OpenAI 兼容 API HTTP 状态码到 AppErrorCode 的映射
pub struct OpenAiHttpStatus;

impl OpenAiHttpStatus {
    pub fn from_status(status: u16) -> AppErrorCode {
        match status {
            200..=299 => AppErrorCode::EUnknown,
            400 => AppErrorCode::EOpenAiBadRequest,
            401 => AppErrorCode::EOpenAiUnauthorized,
            429 => AppErrorCode::EOpenAiRateLimit,
            500..=599 => AppErrorCode::EOpenAiServerError,
            _ => AppErrorCode::EOpenAiRequestFailed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_codes_are_unique() {
        let codes = vec![
            AppErrorCode::EOllamaStreamBreak,
            AppErrorCode::EOllamaStreamEmpty,
            AppErrorCode::EOllamaRequestFailed,
            AppErrorCode::EOllamaBadRequest,
            AppErrorCode::EOllamaServerError,
            AppErrorCode::EOllamaModelNotFound,
            AppErrorCode::EOllamaUnloadFailed,
            AppErrorCode::EOpenAiRequestFailed,
            AppErrorCode::EOpenAiBadRequest,
            AppErrorCode::EOpenAiUnauthorized,
            AppErrorCode::EOpenAiRateLimit,
            AppErrorCode::EOpenAiServerError,
            AppErrorCode::EPipelineFallbackTriggered,
            AppErrorCode::EPipelineImprovementTooLow,
            AppErrorCode::EUnknown,
        ];
        let mut seen = std::collections::HashSet::new();
        for code in &codes {
            assert!(seen.insert(code.code()), "错误码 {} 重复", code.code());
        }
    }
}
