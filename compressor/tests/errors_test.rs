//! TDD Step 2 RED: 测试 AppErrorCode 错误码体系
//! 验证：
//! - code() 返回值唯一
//! - description() 非空
//! - from_http_status() 正确映射 HTTP 状态码到错误码

use aitxt_compressor::errors::{AppErrorCode, OllamaHttpStatus, OpenAiHttpStatus};

#[test]
fn test_error_codes_are_unique() {
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
        let n = code.code();
        assert!(!seen.contains(&n), "错误码 {} 重复", n);
        seen.insert(n);
    }
}

#[test]
fn test_description_is_non_empty() {
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
    for code in &codes {
        assert!(!code.description().is_empty(), "错误码 {:?} 描述为空", code);
    }
}

#[test]
fn test_ollama_http_status_mapping() {
    // 404 → ModelNotFound
    assert_eq!(
        OllamaHttpStatus::from_status(404),
        AppErrorCode::EOllamaModelNotFound
    );
    // 400 → BadRequest
    assert_eq!(
        OllamaHttpStatus::from_status(400),
        AppErrorCode::EOllamaBadRequest
    );
    // 500 → ServerError
    assert_eq!(
        OllamaHttpStatus::from_status(500),
        AppErrorCode::EOllamaServerError
    );
    // 200 → 不应映射到错误码（应 panic 或返回特殊值）
    // 这里我们要求 200 时返回 EUnknown 表示"不应调用此映射"
    assert_eq!(
        OllamaHttpStatus::from_status(200),
        AppErrorCode::EUnknown
    );
}

#[test]
fn test_openai_http_status_mapping() {
    // 401 → Unauthorized
    assert_eq!(
        OpenAiHttpStatus::from_status(401),
        AppErrorCode::EOpenAiUnauthorized
    );
    // 429 → RateLimit
    assert_eq!(
        OpenAiHttpStatus::from_status(429),
        AppErrorCode::EOpenAiRateLimit
    );
    // 400 → BadRequest
    assert_eq!(
        OpenAiHttpStatus::from_status(400),
        AppErrorCode::EOpenAiBadRequest
    );
    // 500 → ServerError
    assert_eq!(
        OpenAiHttpStatus::from_status(500),
        AppErrorCode::EOpenAiServerError
    );
}

#[test]
fn test_error_code_serializes_to_screaming_snake_case() {
    let code = AppErrorCode::EOllamaStreamBreak;
    let json = serde_json::to_string(&code).unwrap();
    assert_eq!(json, "\"E_OLLAMA_STREAM_BREAK\"");
}
