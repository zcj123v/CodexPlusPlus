use codex_plus_core::protocol_proxy::anthropic_messages_url;
use codex_plus_core::settings::{BackendSettings, RelayProfile, RelayProtocol};

#[test]
fn anthropic_protocol_serde_roundtrip() {
    let json = serde_json::to_string(&RelayProtocol::Anthropic).unwrap();
    assert_eq!(json, "\"anthropic\"");
    let parsed: RelayProtocol = serde_json::from_str("\"anthropic\"").unwrap();
    assert_eq!(parsed, RelayProtocol::Anthropic);
}

#[test]
fn anthropic_profile_uses_protocol_proxy() {
    let mut settings = BackendSettings::default();
    let mut profile = RelayProfile::default();
    profile.id = "p1".to_string();
    profile.protocol = RelayProtocol::Anthropic;
    settings.relay_profiles = vec![profile];
    settings.active_relay_id = "p1".to_string();
    assert!(settings.active_relay_uses_protocol_proxy());
}

#[test]
fn anthropic_messages_url_rules() {
    assert_eq!(
        anthropic_messages_url("https://api.kimi.com/coding"),
        "https://api.kimi.com/coding/v1/messages"
    );
    assert_eq!(
        anthropic_messages_url("https://api.kimi.com/coding/"),
        "https://api.kimi.com/coding/v1/messages"
    );
    // `#` 后缀跳过版本前缀
    assert_eq!(
        anthropic_messages_url("https://proxy.example.com/anthropic#"),
        "https://proxy.example.com/anthropic/messages"
    );
    // 已是完整端点则不拼接
    assert_eq!(
        anthropic_messages_url("https://api.example.com/v1/messages"),
        "https://api.example.com/v1/messages"
    );
}
