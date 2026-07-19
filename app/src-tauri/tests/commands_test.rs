// 集成测试：验证命令级行为（独立于 DB，演示测试目录）。
use echo_memory_lib::commands;

#[test]
fn generate_id_returns_valid_uuid_v4() {
    let id = commands::generate_id();
    assert!(
        uuid::Uuid::parse_str(&id).is_ok(),
        "generate_id 应返回可解析的 UUID：{id}"
    );
}

#[test]
fn greet_includes_name() {
    let msg = commands::greet("测试用户".to_string());
    assert!(msg.contains("测试用户"), "问候应回显名字：{msg}");
}
