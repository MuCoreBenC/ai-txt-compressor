//! TDD Step 4 RED: 测试 ThinkingParser 状态机
//! 验证：
//! - thinking 标签外内容直接进入 output
//! - thinking 标签内内容只进入 reasoning，不出 output
//! - 跨 chunk 标签（如 "<thi" + "nk>"）能正确解析
//! - flush 时残余内容按当前状态输出

use aitxt_compressor::model::ollama::ThinkingParser;

#[test]
fn test_output_before_thinking_tag() {
    let mut p = ThinkingParser::new();
    p.feed("正在思考");
    assert_eq!(p.take_output(), "正在思考");
    assert_eq!(p.take_reasoning(), "");
}

#[test]
fn test_thinking_content_goes_to_reasoning_only() {
    let mut p = ThinkingParser::new();
    p.feed("\u{3c}think\u{3e}这是思考内容");
    assert_eq!(p.take_output(), "");
    assert_eq!(p.take_reasoning(), "这是思考内容");
}

#[test]
fn test_output_resumes_after_closing_tag() {
    let mut p = ThinkingParser::new();
    p.feed("\u{3c}think\u{3e}思考\u{3c}/think\u{3e}实际输出");
    assert_eq!(p.take_reasoning(), "思考");
    assert_eq!(p.take_output(), "实际输出");
}

#[test]
fn test_tag_split_across_chunks() {
    let mut p = ThinkingParser::new();
    p.feed("正在\u{3c}thi");
    p.feed("nk\u{3e}思考内容\u{3c}/thi");
    p.feed("nk\u{3e}实际");
    assert_eq!(p.take_reasoning(), "思考内容");
    assert_eq!(p.take_output(), "正在实际");
}

#[test]
fn test_flush_emits_remaining_as_current_state() {
    // 流结束时还在 thinking 内：残余作为 reasoning
    let mut p = ThinkingParser::new();
    p.feed("\u{3c}think\u{3e}未闭合的思考");
    p.flush();
    assert_eq!(p.take_reasoning(), "未闭合的思考");
    assert_eq!(p.take_output(), "");
}

#[test]
fn test_flush_emits_output_when_not_in_thinking() {
    // 流结束时不在 thinking：残余作为 output
    let mut p = ThinkingParser::new();
    p.feed("实际输出未结束");
    p.flush();
    assert_eq!(p.take_output(), "实际输出未结束");
    assert_eq!(p.take_reasoning(), "");
}

#[test]
fn test_multiple_think_blocks() {
    let mut p = ThinkingParser::new();
    p.feed("开头\u{3c}think\u{3e}第一段思考\u{3c}/think\u{3e}中间");
    assert_eq!(p.take_output(), "开头中间");
    assert_eq!(p.take_reasoning(), "第一段思考");
    p.feed("\u{3c}think\u{3e}第二段思考\u{3c}/think\u{3e}结尾");
    assert_eq!(p.take_output(), "结尾");
    assert_eq!(p.take_reasoning(), "第二段思考");
}
