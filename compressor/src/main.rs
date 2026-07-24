//! 语义级文本压缩器 — Rust + Ollama 混合管线
//!
//! 流程：原文 → 算法压缩（jieba + TextRank + 规则） → 模型压缩（Ollama） → 输出

mod algo;
mod cli;
mod model;
mod pipeline;
mod prompt;
mod server;

use clap::Parser;

/// 语义级文本压缩器
#[derive(Parser, Debug, Clone)]
#[command(name = "compressor", version, about = "语义级文本压缩器 (algo + Ollama)")]
pub struct Cli {
    /// 启动 HTTP 服务模式（默认走 stdin/stdout CLI）
    #[arg(long, default_value_t = false)]
    pub serve: bool,

    /// HTTP 服务监听端口
    #[arg(long, default_value_t = 8787)]
    pub port: u16,

    /// 目标压缩比 (0.0-1.0)，0.5 = 压到原文一半
    #[arg(long, default_value_t = 0.5)]
    pub ratio: f32,

    /// 只跑算法，不调用模型（离线/快速模式）
    #[arg(long, default_value_t = false)]
    pub no_model: bool,

    /// Ollama 模型名（1.5b 质量更好，0.5b 更快）
    #[arg(long, default_value = "qwen2.5:1.5b")]
    pub model: String,

    /// 打印每阶段字数与耗时到 stderr
    #[arg(long, default_value_t = false)]
    pub verbose: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Cli::parse();
    if args.serve {
        server::run(args).await
    } else {
        cli::run(args).await
    }
}
