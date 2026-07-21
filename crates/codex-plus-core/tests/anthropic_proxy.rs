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

use codex_plus_core::anthropic_proxy::responses_to_anthropic_messages;
use serde_json::{json, Value};

#[test]
fn converts_instructions_to_system_with_cache_control() {
    let body = json!({
        "model": "k3",
        "instructions": "You are Codex.",
        "input": [{"type":"message","role":"user","content":[{"type":"input_text","text":"hi"}]}]
    });
    let out = responses_to_anthropic_messages(&body).unwrap();
    let system = out["system"].as_array().unwrap();
    assert_eq!(system[0]["text"], "You are Codex.");
    assert_eq!(
        system.last().unwrap()["cache_control"],
        json!({"type":"ephemeral"})
    );
    assert_eq!(out["max_tokens"], 32000);
    assert_eq!(out["model"], "k3");
}

#[test]
fn converts_tool_calls_and_results_preserving_arguments() {
    let args = "{\"command\":[\"bash\",\"-lc\",\"ls\"],\"workdir\":\"/tmp\"}";
    let body = json!({
        "model": "k3",
        "input": [
            {"type":"function_call","call_id":"c1","name":"shell","arguments":args},
            {"type":"function_call_output","call_id":"c1","output":"file1\nfile2"}
        ]
    });
    let out = responses_to_anthropic_messages(&body).unwrap();
    let messages = out["messages"].as_array().unwrap();
    assert_eq!(messages[0]["role"], "assistant");
    assert_eq!(messages[0]["content"][0]["type"], "tool_use");
    assert_eq!(messages[0]["content"][0]["id"], "c1");
    assert_eq!(
        messages[0]["content"][0]["input"],
        json!({"command":["bash","-lc","ls"],"workdir":"/tmp"})
    );
    assert_eq!(messages[1]["role"], "user");
    assert_eq!(messages[1]["content"][0]["type"], "tool_result");
    assert_eq!(messages[1]["content"][0]["tool_use_id"], "c1");
    assert_eq!(messages[1]["content"][0]["content"], "file1\nfile2");
}

#[test]
fn tools_schema_cloned_verbatim() {
    let params = json!({
        "type":"object",
        "properties":{"cmd":{"type":"array","items":{"type":"string"}}},
        "required":["cmd"],
        "additionalProperties":false
    });
    let body = json!({
        "model": "k3",
        "input": [],
        "tools": [{"type":"function","name":"shell","description":"run cmd","parameters":params.clone()}]
    });
    let out = responses_to_anthropic_messages(&body).unwrap();
    let tools = out["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0]["name"], "shell");
    assert_eq!(tools[0]["description"], "run cmd");
    // 语义逐字节一致：Value 直接相等，未解构重建
    assert_eq!(tools[0]["input_schema"], params);
    assert_eq!(tools.last().unwrap()["cache_control"], json!({"type":"ephemeral"}));
}

#[test]
fn converts_input_image_data_url() {
    let body = json!({
        "model": "k3",
        "input": [{"type":"message","role":"user","content":[
            {"type":"input_text","text":"看图"},
            {"type":"input_image","image_url":"data:image/png;base64,iVBORw0KGgo="}
        ]}]
    });
    let out = responses_to_anthropic_messages(&body).unwrap();
    let content = out["messages"][0]["content"].as_array().unwrap();
    assert_eq!(content[1]["type"], "image");
    assert_eq!(content[1]["source"]["type"], "base64");
    assert_eq!(content[1]["source"]["media_type"], "image/png");
    assert_eq!(content[1]["source"]["data"], "iVBORw0KGgo=");
}

#[test]
fn maps_reasoning_effort_to_thinking_budget() {
    let body = json!({"model":"k3","input":[],"reasoning":{"effort":"high"}});
    let out = responses_to_anthropic_messages(&body).unwrap();
    assert_eq!(out["thinking"], json!({"type":"enabled","budget_tokens":16384}));

    let body = json!({"model":"k3","input":[],"reasoning":{"effort":"none"}});
    let out = responses_to_anthropic_messages(&body).unwrap();
    assert!(out.get("thinking").is_none());
}

use codex_plus_core::anthropic_proxy::anthropic_message_to_response;

#[test]
fn converts_message_with_text_tool_use_and_thinking() {
    let body = json!({
        "id": "msg_01",
        "model": "k3",
        "stop_reason": "tool_use",
        "usage": {"input_tokens": 100, "output_tokens": 50,
                  "cache_read_input_tokens": 80, "cache_creation_input_tokens": 20},
        "content": [
            {"type":"thinking","thinking":"先看下目录","signature":"sig-abc"},
            {"type":"text","text":"我来列一下文件"},
            {"type":"tool_use","id":"toolu_1","name":"shell","input":{"command":["ls"]}}
        ]
    });
    let out = anthropic_message_to_response(&body, None).unwrap();
    assert_eq!(out["object"], "response");
    assert_eq!(out["status"], "completed");
    let output = out["output"].as_array().unwrap();
    assert_eq!(output[0]["type"], "reasoning");
    assert_eq!(output[0]["summary"][0]["text"], "先看下目录");
    assert_eq!(output[0]["signature"], "sig-abc");
    assert_eq!(output[1]["type"], "message");
    assert_eq!(output[1]["content"][0]["type"], "output_text");
    assert_eq!(output[1]["content"][0]["text"], "我来列一下文件");
    assert_eq!(output[2]["type"], "function_call");
    assert_eq!(output[2]["call_id"], "toolu_1");
    assert_eq!(output[2]["name"], "shell");
    assert_eq!(output[2]["arguments"], "{\"command\":[\"ls\"]}");
    // usage 透传（含缓存字段）
    assert_eq!(out["usage"]["input_tokens"], 100);
    assert_eq!(out["usage"]["output_tokens"], 50);
    assert_eq!(out["usage"]["input_tokens_details"]["cached_tokens"], 80);
}

#[test]
fn maps_max_tokens_stop_to_incomplete() {
    let body = json!({
        "id": "msg_02", "model": "k3", "stop_reason": "max_tokens",
        "usage": {"input_tokens": 1, "output_tokens": 32000},
        "content": [{"type":"text","text":"..."}]
    });
    let out = anthropic_message_to_response(&body, None).unwrap();
    assert_eq!(out["status"], "incomplete");
    assert_eq!(out["incomplete_details"]["reason"], "max_output_tokens");
}
