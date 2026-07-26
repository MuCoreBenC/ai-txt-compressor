//! HTTP 服务模式：axum + CORS，监听 127.0.0.1:8787
//!
//! 单文件模式：前端 compress.html 通过 include_str! 嵌入二进制，
//! 同一端口同时提供页面与 API，无需额外静态服务器。

use axum::{
    extract::State,
    http::{header, StatusCode},
    response::{
        sse::{Event, Sse},
        IntoResponse,
    },
    routing::{get, post},
    Json, Router,
};
use futures::stream::StreamExt;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tower_http::cors::{Any, CorsLayer};

use crate::model::ollama::OllamaClient;
use crate::pipeline::{compress, compress_stream, CompressOptions, StreamEvent};
use crate::prompt::PresetPrompt;

/// 编译时嵌入前端页面，生成真正的单文件二进制
static INDEX_HTML: &str = include_str!("../../compress.html");

#[derive(Deserialize)]
struct CompressRequest {
    text: String,
    #[serde(default = "default_ratio")]
    ratio: f32,
    #[serde(default)]
    no_model: bool,
    #[serde(default = "default_provider")]
    provider: String,
    #[serde(default = "default_model")]
    model: String,
    /// 自定义厂商时由前端提供（前端存 localStorage，每次请求携带）
    #[serde(default)]
    base_url: Option<String>,
    /// 前端提供的 API key 覆盖后端环境变量
    #[serde(default)]
    api_key: Option<String>,
    /// 推理强度：low/medium/high（仅部分厂商支持）
    #[serde(default)]
    reasoning_effort: Option<String>,
    /// 自定义 system 提示词（None 用默认）
    #[serde(default)]
    custom_system: Option<String>,
    /// 自定义 user 模板（None 用默认）
    /// 支持占位符：{text} / {target} / {orig} / {cut}
    #[serde(default)]
    custom_user_template: Option<String>,
    /// 预设提示词 ID（"minimal"/"standard"/"strict_chars"）
    #[serde(default)]
    preset: Option<String>,
    /// 若提供则跳过算法阶段直接用此文本调模型（用于重试）
    #[serde(default)]
    text_algo: Option<String>,
    /// 显式覆盖目标字数（用于重试时保持原目标）
    #[serde(default)]
    target_chars_override: Option<usize>,
    /// 显式覆盖目标字数（用于用户直接指定"压到 1000 字"）
    #[serde(default)]
    target_chars: Option<usize>,
}

fn default_ratio() -> f32 {
    0.5
}
fn default_provider() -> String {
    "ollama".to_string()
}
fn default_model() -> String {
    "qwen2.5:1.5b".to_string()
}

#[derive(Serialize)]
struct HealthResponse {
    ollama: bool,
    deepseek: bool,
    model: String,
}

#[derive(Clone)]
struct AppState {
    args: crate::Cli,
    deepseek_key: Option<String>,
}

pub async fn run(args: crate::Cli) -> anyhow::Result<()> {
    // DeepSeek API key：优先 --api-key 参数，其次 DEEPSEEK_API_KEY 环境变量
    let deepseek_key: Option<String> = if !args.api_key.is_empty() {
        Some(args.api_key.clone())
    } else {
        std::env::var("DEEPSEEK_API_KEY").ok().filter(|s| !s.is_empty())
    };
    if deepseek_key.is_some() {
        tracing::info!("[server] DeepSeek API key 已配置（来自 {}）",
            if !args.api_key.is_empty() { "--api-key 参数" } else { "DEEPSEEK_API_KEY 环境变量" });
    } else {
        tracing::warn!("[server] DeepSeek API key 未配置（设置环境变量 DEEPSEEK_API_KEY 或启动时传 --api-key）");
    }
    let state = Arc::new(AppState { args: args.clone(), deepseek_key });

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let app = Router::new()
        .route("/", get(index_handler))
        .route("/compress.html", get(index_handler))
        .route("/compress", post(compress_handler))
        .route("/compress_stream", post(compress_stream_handler))
        .route("/health", get(health_handler))
        .route("/default-prompt", get(default_prompt_handler))
        .route("/preset-prompts", get(preset_prompts_handler))
        .route("/ollama/pull", post(ollama_pull_handler))
        .route("/ollama/tags", get(ollama_tags_handler))
        .route("/ollama/unload", post(ollama_unload_handler))
        .route("/shutdown", post(shutdown_handler))
        .layer(cors)
        .with_state(state);

    let addr = format!("127.0.0.1:{}", args.port);
    tracing::info!("[server] listening on http://{}", addr);
    tracing::info!("[server] 单文件模式：页面与 API 同源，直接打开上面的地址即可");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

/// 返回嵌入的前端页面，带正确的 Content-Type 与禁止缓存头
async fn index_handler() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        [(header::CACHE_CONTROL, "no-cache, no-store, must-revalidate")],
        INDEX_HTML,
    )
}

/// 停止服务：返回 200 后延迟退出进程（让前端先收到响应）
async fn shutdown_handler() -> &'static str {
    tracing::info!("[server] 收到 shutdown 请求，200ms 后退出进程");
    tokio::spawn(async {
        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
        std::process::exit(0);
    });
    "服务已停止"
}

async fn compress_handler(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CompressRequest>,
) -> Result<Json<crate::pipeline::CompressResult>, (StatusCode, String)> {
    // API key 优先级：前端请求 > 后端环境变量
    // 前端存 localStorage，每次请求携带，覆盖后端 DEEPSEEK_API_KEY
    let api_key = req.api_key.clone().or_else(|| state.deepseek_key.clone());

    // DeepSeek/custom 校验：必须有 api_key
    if (req.provider == "deepseek" || req.provider == "custom") && api_key.is_none() {
        return Err((
            StatusCode::BAD_REQUEST,
            format!(
                "{} 需要 API key（在页面下方填写，或启动服务时设环境变量 DEEPSEEK_API_KEY）",
                req.provider
            ),
        ));
    }
    // custom 校验：必须有 base_url
    if req.provider == "custom" && req.base_url.as_deref().unwrap_or("").is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "自定义 provider 需在页面下方填写 Base URL".to_string(),
        ));
    }

    let opts = CompressOptions {
        ratio: req.ratio,
        no_model: req.no_model,
        provider: req.provider.clone(),
        model: req.model.clone(),
        api_key,
        base_url: req.base_url.clone(),
        reasoning_effort: req.reasoning_effort.clone(),
        custom_system: req.custom_system.clone(),
        custom_user_template: req.custom_user_template.clone(),
        verbose: state.args.verbose,
        preset: req.preset.clone(),
        text_algo: req.text_algo.clone(),
        target_chars_override: req.target_chars_override,
        target_chars: req.target_chars,
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
        deepseek: state.deepseek_key.is_some(),
        model: state.args.model.clone(),
    })
}

#[derive(Serialize)]
struct DefaultPromptResponse {
    system: &'static str,
    user_template: &'static str,
}

/// 返回默认提示词，供前端展示和编辑
async fn default_prompt_handler() -> Json<DefaultPromptResponse> {
    Json(DefaultPromptResponse {
        system: crate::prompt::default_system(),
        user_template: crate::prompt::default_user_template(),
    })
}

// ==================== SSE 流式压缩 ====================

/// SSE 流式压缩 handler
/// 创建 mpsc channel，spawn 后台任务调用 compress_stream，
/// 返回 Sse<ReceiverStream> 把每个 StreamEvent 序列化为 JSON 推给前端
async fn compress_stream_handler(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CompressRequest>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    // API key 优先级：前端请求 > 后端环境变量
    let api_key = req.api_key.clone().or_else(|| state.deepseek_key.clone());

    // DeepSeek/custom 校验：必须有 api_key
    if (req.provider == "deepseek" || req.provider == "custom") && api_key.is_none() {
        return Err((
            StatusCode::BAD_REQUEST,
            format!(
                "{} 需要 API key（在页面下方填写，或启动服务时设环境变量 DEEPSEEK_API_KEY）",
                req.provider
            ),
        ));
    }
    // custom 校验：必须有 base_url
    if req.provider == "custom" && req.base_url.as_deref().unwrap_or("").is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "自定义 provider 需在页面下方填写 Base URL".to_string(),
        ));
    }

    let opts = CompressOptions {
        ratio: req.ratio,
        no_model: req.no_model,
        provider: req.provider.clone(),
        model: req.model.clone(),
        api_key,
        base_url: req.base_url.clone(),
        reasoning_effort: req.reasoning_effort.clone(),
        custom_system: req.custom_system.clone(),
        custom_user_template: req.custom_user_template.clone(),
        verbose: state.args.verbose,
        preset: req.preset.clone(),
        text_algo: req.text_algo.clone(),
        target_chars_override: req.target_chars_override,
        target_chars: req.target_chars,
    };

    let (tx, rx) = mpsc::channel::<StreamEvent>(128);
    // 保留一个 sender 用于 compress_stream 异常时推送 Error 事件
    let tx_for_error = tx.clone();

    let text = req.text.clone();
    tokio::spawn(async move {
        let result = compress_stream(&text, &opts, tx).await;
        if let Err(e) = result {
            let _ = tx_for_error
                .send(StreamEvent::Error {
                    msg: format!("compress_stream 异常：{}", e),
                    t: 0,
                })
                .await;
        }
        // tx 在此 drop，channel 关闭，SSE 流结束
    });

    let stream = ReceiverStream::new(rx).map(|event| {
        let json = serde_json::to_string(&event).unwrap_or_else(|_| "{}".to_string());
        Ok::<Event, std::convert::Infallible>(Event::default().data(json))
    });

    Ok(Sse::new(stream))
}

// ==================== 预设提示词 ====================

#[derive(Serialize)]
struct PresetPromptInfo {
    id: &'static str,
    name: &'static str,
    system: &'static str,
    user_template: &'static str,
}

#[derive(Serialize)]
struct PresetPromptsResponse {
    presets: Vec<PresetPromptInfo>,
}

/// 返回 3 套预设提示词，供前端选择
async fn preset_prompts_handler() -> Json<PresetPromptsResponse> {
    let presets = vec![
        PresetPromptInfo {
            id: PresetPrompt::Minimal.id(),
            name: PresetPrompt::Minimal.name(),
            system: PresetPrompt::Minimal.system(),
            user_template: PresetPrompt::Minimal.user_template(),
        },
        PresetPromptInfo {
            id: PresetPrompt::Standard.id(),
            name: PresetPrompt::Standard.name(),
            system: PresetPrompt::Standard.system(),
            user_template: PresetPrompt::Standard.user_template(),
        },
        PresetPromptInfo {
            id: PresetPrompt::StrictChars.id(),
            name: PresetPrompt::StrictChars.name(),
            system: PresetPrompt::StrictChars.system(),
            user_template: PresetPrompt::StrictChars.user_template(),
        },
    ];
    Json(PresetPromptsResponse { presets })
}

// ==================== Ollama 模型管理 ====================

/// Ollama pull 进度事件（SSE 推送给前端）
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub enum PullEvent {
    PullStart {
        name: String,
        t: u64,
    },
    PullProgress {
        status: String,
        completed: Option<u64>,
        total: Option<u64>,
        digest: Option<String>,
        t: u64,
    },
    PullDone {
        name: String,
        t: u64,
    },
    PullError {
        msg: String,
        t: u64,
    },
}

#[derive(Deserialize)]
struct OllamaPullRequest {
    name: String,
}

/// POST /ollama/pull：调用 Ollama /api/pull 流式拉取模型，每个 NDJSON 行包装为 SSE 事件
async fn ollama_pull_handler(Json(req): Json<OllamaPullRequest>) -> impl IntoResponse {
    use futures::StreamExt;
    use std::time::Instant;

    let (tx, rx) = mpsc::channel::<PullEvent>(128);
    let name = req.name.clone();
    // 预留一份用于 PullDone（name 会在 json! 宏中被 move）
    let name_for_done = name.clone();

    tokio::spawn(async move {
        let start = Instant::now();
        let t_now = || start.elapsed().as_millis() as u64;

        // 推送 PullStart
        let _ = tx
            .send(PullEvent::PullStart {
                name: name.clone(),
                t: t_now(),
            })
            .await;

        // 调用 Ollama POST /api/pull with stream=true
        let client = match reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(600))
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                let _ = tx
                    .send(PullEvent::PullError {
                        msg: format!("构建 HTTP 客户端失败：{}", e),
                        t: t_now(),
                    })
                    .await;
                return;
            }
        };

        let body = serde_json::json!({ "name": name, "stream": true });
        let resp = match client
            .post("http://127.0.0.1:11434/api/pull")
            .json(&body)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                let _ = tx
                    .send(PullEvent::PullError {
                        msg: format!("请求 Ollama /api/pull 失败（确认 ollama serve 在运行）：{}", e),
                        t: t_now(),
                    })
                    .await;
                return;
            }
        };

        let status = resp.status();
        if !status.is_success() {
            let raw = resp.text().await.unwrap_or_default();
            let _ = tx
                .send(PullEvent::PullError {
                    msg: format!("Ollama 返回 {}：{}", status, raw),
                    t: t_now(),
                })
                .await;
            return;
        }

        let mut stream = resp.bytes_stream();
        let mut line_buf = String::new();

        while let Some(chunk_result) = stream.next().await {
            let chunk = match chunk_result {
                Ok(c) => c,
                Err(e) => {
                    let _ = tx
                        .send(PullEvent::PullError {
                            msg: format!("读取 pull 流失败：{}", e),
                            t: t_now(),
                        })
                        .await;
                    return;
                }
            };
            let s = match std::str::from_utf8(&chunk) {
                Ok(s) => s,
                Err(e) => {
                    let _ = tx
                        .send(PullEvent::PullError {
                            msg: format!("pull 流响应非 UTF-8：{}", e),
                            t: t_now(),
                        })
                        .await;
                    return;
                }
            };
            line_buf.push_str(s);

            // NDJSON：每行一个 JSON 对象
            while let Some(pos) = line_buf.find('\n') {
                let line: String = line_buf.drain(..=pos).collect();
                let line = line.trim_end_matches(['\n', '\r']);
                if line.is_empty() {
                    continue;
                }

                let value: serde_json::Value = match serde_json::from_str(line) {
                    Ok(v) => v,
                    Err(_) => continue,
                };

                let status_str = value["status"].as_str().unwrap_or("").to_string();

                // 检查 error 字段（Ollama 错误响应）
                if let Some(err_msg) = value["error"].as_str() {
                    let _ = tx
                        .send(PullEvent::PullError {
                            msg: err_msg.to_string(),
                            t: t_now(),
                        })
                        .await;
                    return;
                }

                // success 状态 → PullDone
                if status_str == "success" {
                    let _ = tx
                        .send(PullEvent::PullDone {
                            name: name_for_done.clone(),
                            t: t_now(),
                        })
                        .await;
                    return;
                }

                // 其他状态 → PullProgress
                let completed = value["completed"].as_u64();
                let total = value["total"].as_u64();
                let digest = value["digest"].as_str().map(|s| s.to_string());

                let _ = tx
                    .send(PullEvent::PullProgress {
                        status: status_str,
                        completed,
                        total,
                        digest,
                        t: t_now(),
                    })
                    .await;
            }
        }

        // 流自然结束但没收到 success：也推送 Done（避免前端卡住）
        let _ = tx
            .send(PullEvent::PullDone {
                name: name_for_done,
                t: t_now(),
            })
            .await;
    });

    let stream = ReceiverStream::new(rx).map(|event| {
        let json = serde_json::to_string(&event).unwrap_or_else(|_| "{}".to_string());
        Ok::<Event, std::convert::Infallible>(Event::default().data(json))
    });

    Sse::new(stream)
}

/// GET /ollama/tags：调用 Ollama /api/tags 获取本地已安装模型列表
/// 失败时返回空数组
async fn ollama_tags_handler() -> impl IntoResponse {
    let empty = serde_json::json!({ "models": [] });
    match reqwest::get("http://127.0.0.1:11434/api/tags").await {
        Ok(resp) if resp.status().is_success() => {
            match resp.json::<serde_json::Value>().await {
                Ok(v) => Json(v),
                Err(_) => Json(empty),
            }
        }
        _ => Json(empty),
    }
}

#[derive(Deserialize)]
struct OllamaUnloadRequest {
    model: String,
}

/// POST /ollama/unload：调用 Ollama /api/generate 传 keep_alive=0 卸载模型
/// 错误码：E_OLLAMA_UNLOAD_FAILED (1007)
async fn ollama_unload_handler(
    Json(req): Json<OllamaUnloadRequest>,
) -> impl IntoResponse {
    use crate::errors::AppErrorCode;

    if req.model.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "ok": false,
                "error": "model 字段不能为空",
                "error_code": "E_OLLAMA_UNLOAD_FAILED",
                "error_code_num": AppErrorCode::EOllamaUnloadFailed.code(),
            })),
        )
            .into_response();
    }

    let client = OllamaClient::new(&req.model);
    match client.unload_model().await {
        Ok(_) => {
            tracing::info!(model = %req.model, "前端请求卸载模型成功");
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "ok": true,
                    "model": req.model,
                    "message": "模型已从内存卸载"
                })),
            )
                .into_response()
        }
        Err(e) => {
            tracing::error!(
                error_code = ?AppErrorCode::EOllamaUnloadFailed.code(),
                error = %e,
                model = %req.model,
                "前端请求卸载模型失败"
            );
            (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({
                    "ok": false,
                    "error": e.to_string(),
                    "error_code": "E_OLLAMA_UNLOAD_FAILED",
                    "error_code_num": AppErrorCode::EOllamaUnloadFailed.code(),
                })),
            )
                .into_response()
        }
    }
}
