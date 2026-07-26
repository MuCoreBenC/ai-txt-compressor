//! 冒烟测试：验证 cargo test 基础设施工作
//! TDD Step 1：先建立测试底座，后续每个 bug 修复都先写 RED 测试

#[test]
fn smoke_test_cargo_test_works() {
    assert_eq!(2 + 2, 4);
}

#[test]
fn smoke_test_serde_json_available() {
    let v: serde_json::Value = serde_json::json!({"ok": true});
    assert_eq!(v["ok"].as_bool(), Some(true));
}
