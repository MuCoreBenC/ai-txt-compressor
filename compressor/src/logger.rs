//! 日志初始化：tracing + tracing-subscriber + tracing-appender
//!
//! 同时输出到控制台（彩色）和文件（按天滚动）。
//! 文件位置：`compressor/logs/aitxt.log.YYYY-MM-DD`
//! 控制级别：环境变量 RUST_LOG，默认 info,aitxt_compressor=debug

use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling;
use tracing_subscriber::{fmt, EnvFilter, prelude::*};

/// 初始化全局日志系统，返回 WorkerGuard（必须保活以维持文件写入）
pub fn init_logger() -> WorkerGuard {
    init_logger_with_dir("logs")
}

/// 指定日志目录的初始化（便于测试）
pub fn init_logger_with_dir(log_dir: &str) -> WorkerGuard {
    // 确保日志目录存在
    let _ = std::fs::create_dir_all(log_dir);

    let file_appender = rolling::daily(log_dir, "aitxt.log");
    let (non_blocking_file, guard) = tracing_appender::non_blocking(file_appender);

    let console_layer = fmt::layer()
        .with_target(true)
        .with_thread_ids(false)
        .with_level(true)
        .with_ansi(true);

    let file_layer = fmt::layer()
        .with_writer(non_blocking_file)
        .with_ansi(false)
        .with_target(true);

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,aitxt_compressor=debug"));

    tracing_subscriber::registry()
        .with(filter)
        .with(console_layer)
        .with(file_layer)
        .init();

    guard
}

/// 仅用于测试的轻量初始化（不写文件，只设控制台 + 测试缓冲）
#[cfg(test)]
pub fn init_test_logger() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::new("debug"))
        .with_test_writer()
        .try_init();
}
