//! TDD Step 2: 测试 logger 初始化
//! 仅验证 init_logger_with_dir 返回的 guard 不为空（实际类型是 WorkerGuard）
//! 并验证日志目录被创建

use aitxt_compressor::logger;
use std::path::Path;

#[test]
fn test_init_logger_returns_guard_and_creates_dir() {
    let test_log_dir = "test_logs_init";
    // 清理上次的残留
    let _ = std::fs::remove_dir_all(test_log_dir);

    let guard = logger::init_logger_with_dir(test_log_dir);
    // guard 不为空（不能直接比较，但能调用 drop）
    tracing::info!(target: "test_init", "测试日志条目");

    // 必须保活到日志写入完成，否则 non_blocking 会丢
    drop(guard);

    // 验证目录被创建
    assert!(Path::new(test_log_dir).exists(), "日志目录应被创建");

    // 清理
    let _ = std::fs::remove_dir_all(test_log_dir);
}
