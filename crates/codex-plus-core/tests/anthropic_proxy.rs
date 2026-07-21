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
use serde_json::json;

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
    // usage 转换：缓存读/写计入总输入（100+80+20=200），cached_tokens 仅取 cache_read
    assert_eq!(out["usage"]["input_tokens"], 200);
    assert_eq!(out["usage"]["output_tokens"], 50);
    assert_eq!(out["usage"]["total_tokens"], 250);
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

use codex_plus_core::anthropic_proxy::AnthropicSseToResponsesConverter;

/// 把转换器输出按 SSE block 拆成 (event, data_json) 列表方便断言。
fn collect_events(bytes: &[u8]) -> Vec<(String, serde_json::Value)> {
    let text = String::from_utf8_lossy(bytes);
    text.split("\n\n")
        .filter(|block| !block.trim().is_empty())
        .map(|block| {
            let mut event = String::new();
            let mut data = String::new();
            for line in block.lines() {
                if let Some(value) = line.strip_prefix("event: ") {
                    event = value.to_string();
                }
                if let Some(value) = line.strip_prefix("data: ") {
                    data.push_str(value);
                }
            }
            (event, serde_json::from_str(&data).unwrap())
        })
        .collect()
}

#[test]
fn streams_text_tool_call_and_usage() {
    let mut converter = AnthropicSseToResponsesConverter::with_request(&serde_json::json!({}));
    let mut out = Vec::new();
    let stream = concat!(
        "event: message_start\n",
        "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"model\":\"k3\",\"usage\":{\"input_tokens\":10}}}\n\n",
        "event: content_block_start\n",
        "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
        "event: content_block_delta\n",
        "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"你好\"}}\n\n",
        "event: content_block_stop\n",
        "data: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
        "event: content_block_start\n",
        "data: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_1\",\"name\":\"shell\",\"input\":{}}}\n\n",
        "event: content_block_delta\n",
        "data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"command\\\":\"}}\n\n",
        "event: content_block_delta\n",
        "data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"[\\\"ls\\\"]}\"}}\n\n",
        "event: content_block_stop\n",
        "data: {\"type\":\"content_block_stop\",\"index\":1}\n\n",
        "event: message_delta\n",
        "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"},\"usage\":{\"output_tokens\":20}}\n\n",
        "event: message_stop\n",
        "data: {\"type\":\"message_stop\"}\n\n",
    );
    out.extend(converter.push_bytes(stream.as_bytes()));
    out.extend(converter.finish());

    let events = collect_events(&out);
    let names: Vec<&str> = events.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(names.first().copied(), Some("response.created"));
    assert!(names.contains(&"response.output_text.delta"));
    assert!(names.contains(&"response.function_call_arguments.delta"));
    assert_eq!(names.last().copied(), Some("response.completed"));

    // 文本 delta 内容
    let text_delta = events
        .iter()
        .find(|(name, _)| name == "response.output_text.delta")
        .map(|(_, data)| data["delta"].as_str().unwrap().to_string())
        .unwrap();
    assert_eq!(text_delta, "你好");

    // arguments 两个 delta 原样拼接后等于完整 JSON
    let args: String = events
        .iter()
        .filter(|(name, _)| name == "response.function_call_arguments.delta")
        .map(|(_, data)| data["delta"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(args, "{\"command\":[\"ls\"]}");

    // completed 携带 usage
    let completed = events
        .iter()
        .find(|(name, _)| name == "response.completed")
        .map(|(_, data)| data.clone())
        .unwrap();
    assert_eq!(completed["response"]["status"], "completed");
    assert_eq!(completed["response"]["usage"]["output_tokens"], 20);
}

#[test]
fn error_event_yields_response_failed() {
    let mut converter = AnthropicSseToResponsesConverter::with_request(&serde_json::json!({}));
    let out = converter.push_bytes(
        b"event: error\ndata: {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\",\"message\":\"Overloaded\"}}\n\n",
    );
    let events = collect_events(&out);
    assert_eq!(events[0].0, "response.failed");
    assert!(events[0].1["response"]["error"]["message"]
        .as_str()
        .unwrap()
        .contains("Overloaded"));
}

#[test]
fn split_utf8_across_chunks() {
    let mut converter = AnthropicSseToResponsesConverter::with_request(&serde_json::json!({}));
    let full = "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"中文\"}}\n\n";
    let bytes = full.as_bytes();
    // 在多字节字符中间切开喂入，不应产生乱码或 panic
    let mut out = converter.push_bytes(&bytes[..bytes.len() / 2]);
    out.extend(converter.push_bytes(&bytes[bytes.len() / 2..]));
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("中文") || converter.finish().is_empty());
}

#[test]
fn finish_processes_trailing_block_without_blank_line() {
    let mut converter = AnthropicSseToResponsesConverter::with_request(&serde_json::json!({}));
    let mut out = Vec::new();
    // message_start 正常结尾，末尾 message_delta（携带 usage）残缺（无尾部空行）
    let stream = concat!(
        "event: message_start\n",
        "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_2\",\"model\":\"k3\",\"usage\":{\"input_tokens\":7}}}\n\n",
        "event: message_delta\n",
        "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":42}}",
    );
    out.extend(converter.push_bytes(stream.as_bytes()));
    // finish() 应解析 buffer 中未以 \n\n 结尾的 message_delta 残缺块，
    // 否则其 usage 会丢失（旧行为同样会补 response.completed，无法区分）
    out.extend(converter.finish());

    let events = collect_events(&out);
    let names: Vec<&str> = events.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(names.last().copied(), Some("response.completed"));
    let completed = events
        .iter()
        .find(|(name, _)| name == "response.completed")
        .map(|(_, data)| data.clone())
        .unwrap();
    assert_eq!(completed["response"]["status"], "completed");
    // 残缺 message_delta 的 output_tokens 必须并入 completed 的 usage
    assert_eq!(completed["response"]["usage"]["output_tokens"], 42);
    assert_eq!(completed["response"]["usage"]["input_tokens"], 7);
}

#[test]
fn no_completed_after_error_event() {
    let mut converter = AnthropicSseToResponsesConverter::with_request(&serde_json::json!({}));
    let mut out = Vec::new();
    // 先收到 error 事件（流已 failed）
    out.extend(converter.push_bytes(
        b"event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_3\",\"model\":\"k3\",\"usage\":{\"input_tokens\":1}}}\n\n",
    ));
    out.extend(converter.push_bytes(
        b"event: error\ndata: {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\",\"message\":\"Overloaded\"}}\n\n",
    ));
    // 之后再收到 message_stop：只能有 response.failed，不能补 response.completed
    out.extend(converter.push_bytes(
        b"event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
    ));
    out.extend(converter.finish());

    let events = collect_events(&out);
    let names: Vec<&str> = events.iter().map(|(name, _)| name.as_str()).collect();
    assert!(names.contains(&"response.failed"));
    assert!(!names.contains(&"response.completed"));
}
