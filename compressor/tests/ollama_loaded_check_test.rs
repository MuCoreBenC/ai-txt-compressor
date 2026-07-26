//! TDD Step 6 RED: 测试 OllamaClient::is_model_loaded
//! 验证：
//! - /api/ps 返回的 models 数组中包含当前 model 时返回 true
//! - 不包含时返回 false
//! - /api/ps 返回 5xx 时不报错，返回 false（默认未加载）
//! - 网络不可达时返回 Err

use aitxt_compressor::model::ollama::OllamaClient;
use mockito::Matcher;

#[tokio::test]
async fn test_is_model_loaded_returns_true_when_model_in_ps() {
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("GET", "/api/ps")
        .with_status(200)
        .with_body(r#"{"models":[{"name":"qwen3:1.7b","expires_at":"2025-01-15T10:30:00Z"}]}"#)
        .create_async()
        .await;

    let client = OllamaClient::with_base_url(&server.url(), "qwen3:1.7b");
    let loaded = client.is_model_loaded().await.unwrap();
    assert!(loaded, "模型在 /api/ps 列表中，应返回 true");

    mock.assert_async().await;
}

#[tokio::test]
async fn test_is_model_loaded_returns_true_when_match_by_model_field() {
    // 不同版本 Ollama 可能用 "model" 字段而非 "name"
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("GET", "/api/ps")
        .with_status(200)
        .with_body(r#"{"models":[{"model":"qwen3:1.7b","expires_at":"2025-01-15T10:30:00Z"}]}"#)
        .create_async()
        .await;

    let client = OllamaClient::with_base_url(&server.url(), "qwen3:1.7b");
    let loaded = client.is_model_loaded().await.unwrap();
    assert!(loaded, "通过 model 字段匹配也应返回 true");

    mock.assert_async().await;
}

#[tokio::test]
async fn test_is_model_loaded_returns_false_when_model_not_in_ps() {
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("GET", "/api/ps")
        .with_status(200)
        .with_body(r#"{"models":[]}"#)
        .create_async()
        .await;

    let client = OllamaClient::with_base_url(&server.url(), "qwen3:1.7b");
    let loaded = client.is_model_loaded().await.unwrap();
    assert!(!loaded, "models 数组为空，应返回 false");

    mock.assert_async().await;
}

#[tokio::test]
async fn test_is_model_loaded_returns_false_when_other_model_loaded() {
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("GET", "/api/ps")
        .with_status(200)
        .with_body(r#"{"models":[{"name":"llama3.2:3b"}]}"#)
        .create_async()
        .await;

    let client = OllamaClient::with_base_url(&server.url(), "qwen3:1.7b");
    let loaded = client.is_model_loaded().await.unwrap();
    assert!(!loaded, "加载的是其他模型，应返回 false");

    mock.assert_async().await;
}

#[tokio::test]
async fn test_is_model_loaded_returns_false_on_5xx() {
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("GET", "/api/ps")
        .with_status(500)
        .with_body(r#"{"error":"internal"}"#)
        .create_async()
        .await;

    let client = OllamaClient::with_base_url(&server.url(), "qwen3:1.7b");
    let loaded = client.is_model_loaded().await.unwrap();
    assert!(!loaded, "/api/ps 5xx 时不报错，默认未加载");
}

#[tokio::test]
async fn test_is_model_loaded_returns_false_on_empty_models_array() {
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("GET", "/api/ps")
        .with_status(200)
        .with_body(r#"{"models":[]}"#)
        .match_body(Matcher::Any)
        .create_async()
        .await;

    let client = OllamaClient::with_base_url(&server.url(), "qwen3:1.7b");
    let loaded = client.is_model_loaded().await;
    assert!(loaded.is_ok(), "空 models 数组不应报错");
    assert!(!loaded.unwrap(), "应返回 false");

    mock.assert_async().await;
}
