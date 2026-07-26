//! 语义级文本压缩器 — Rust + Ollama 混合管线
//!
//! 流程：原文 → 算法压缩（jieba + TextRank + 规则） → 模型压缩（Ollama） → 输出

use aitxt_compressor::cli;
use aitxt_compressor::server;
use aitxt_compressor::Cli;
use clap::Parser;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _guard = aitxt_compressor::logger::init_logger();
    tracing::info!("AI.TXT compressor 启动");
    let args = Cli::parse();
    if args.serve {
        server::run(args).await
    } else {
        cli::run(args).await
    }
}
