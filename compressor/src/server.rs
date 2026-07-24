//! HTTP 服务模式：axum + CORS，监听 127.0.0.1:8787

use axum::{
    extract::State,
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};

use crate::model::ollama::OllamaClient;
use crate::pipeline::{compress, CompressOptions};

#[derive(Deserialize)]
struct CompressRequest {
    text: String,
    #[serde(default = "default_ratio")]
    ratio: f32,
    #[serde(default)]
    no_model: bool,
    #[serde(default = "default_model")]
    model: String,
}

fn default_ratio() -> f32 {
    0.5
}
fn default_model() -> String {
    "qwen2.5:1.5b".to_string()
}

#[derive(Serialize)]
struct HealthResponse {
    ollama: bool,
    model: String,
}

#[derive(Clone)]
struct AppState {
    args: crate::Cli,
}

pub async fn run(args: crate::Cli) -> anyhow::Result<()> {
    let state = Arc::new(AppState { args: args.clone() });

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let app = Router::new()
        .route("/compress", post(compress_handler))
        .route("/health", get(health_handler))
        .layer(cors)
        .with_state(state);

    let addr = format!("127.0.0.1:{}", args.port);
    eprintln!("[server] listening on http://{}", addr);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn compress_handler(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CompressRequest>,
) -> Result<Json<crate::pipeline::CompressResult>, (StatusCode, String)> {
    let opts = CompressOptions {
        ratio: req.ratio,
        no_model: req.no_model,
        model: req.model.clone(),
        verbose: state.args.verbose,
    };
    match compress(&req.text, &opts).await {
        Ok(result) => Ok(Json(result)),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

async fn health_handler(State(state): State<Arc<AppState>>) -> Json<HealthResponse> {
    let client = OllamaClient::new(&state.args.model);
    let ok = client.health().await;
    Json(HealthResponse {
        ollama: ok,
        model: state.args.model.clone(),
    })
}
