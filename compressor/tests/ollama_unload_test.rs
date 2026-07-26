//! TDD Step 3 RED: 测试 OllamaClient::unload_model 方法
//! 验证：
//! - 调用 Ollama /api/generate 端点
//! - 请求体含 { "model": "<model>", "keep_alive": 0 }
//! - 成功时返回 Ok
//! - HTTP 失败时返回 Err 并附带错误码 E_OLLAMA_UNLOAD_FAILED

use aitxt_compressor::model::ollama::OllamaClient;
use mockito::Matcher;

#[tokio::test]
async fn test_unload_model_calls_generate_with_keep_alive_zero() {
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("POST", "/api/generate")
        .match_body(Matcher::JsonString(
            serde_json::json!({"model": "qwen3:1.7b", "keep_alive": 0}).to_string(),
        ))
        .with_status(200)
        .with_body(r#"{"model":"qwen3:1.7b","done":true}"#)
        .create_async()
        .await;

    let client = OllamaClient::with_base_url(&server.url(), "qwen3:1.7b");
    let result = client.unload_model().await;
    assert!(result.is_ok(), "unload_model 应成功，实际: {:?}", result.err());

    mock.assert_async().await;
}

#[tokio::test]
async fn test_unload_model_returns_err_on_ollama_500() {
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("POST", "/api/generate")
        .with_status(500)
        .with_body(r#"{"error":"internal server error"}"#)
        .create_async()
        .await;

    let client = OllamaClient::with_base_url(&server.url(), "qwen3:1.7b");
    let result = client.unload_model().await;
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("卸载模型失败") || err_msg.contains("E1007"),
        "错误信息应含 '卸载模型失败' 或 'E1007'，实际: {}",
        err_msg
    );
}

#[tokio::test]
async fn test_unload_model_returns_err_on_connection_refused() {
    // 使用一个不存在的端口
    let client = OllamaClient::with_base_url("http://127.0.0.1:1", "qwen3:1.7b");
    let result = client.unload_model().await;
    assert!(result.is_err());
}
