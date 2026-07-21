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
        "input": "schema test",
        "tools": [{"type":"function","name":"shell","description":"run cmd","parameters":params.clone()}]
    });
    let out = responses_to_anthropic_messages(&body).unwrap();
    let tools = out["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0]["name"], "shell");
    assert_eq!(tools[0]["description"], "run cmd");
    // 语义逐字节一致：Value 直接相等，未解构重建
    assert_eq!(tools[0]["input_schema"], params);
    assert_eq!(
        tools.last().unwrap()["cache_control"],
        json!({"type":"ephemeral"})
    );
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
    let body = json!({"model":"k3","input":"budget","reasoning":{"effort":"high"}});
    let out = responses_to_anthropic_messages(&body).unwrap();
    assert_eq!(
        out["thinking"],
        json!({"type":"enabled","budget_tokens":16384})
    );

    let body = json!({"model":"k3","input":"budget","reasoning":{"effort":"none"}});
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

fn event_names(events: &[(String, serde_json::Value)]) -> Vec<&str> {
    events.iter().map(|(name, _)| name.as_str()).collect()
}

fn message_start(id: &str) -> String {
    format!(
        "event: message_start\ndata: {{\"type\":\"message_start\",\"message\":{{\"id\":\"{id}\",\"model\":\"k3\",\"usage\":{{\"input_tokens\":1}}}}}}\n\n"
    )
}

fn raw_message_stop() -> String {
    sse_event("message_stop", json!({"type":"message_stop"}))
}

fn message_stop() -> String {
    format!(
        "{}{}",
        sse_event(
            "message_delta",
            json!({"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":1}}),
        ),
        raw_message_stop(),
    )
}

fn assert_invalid_sse_event(stream: &str, raw_marker: Option<&str>) {
    let mut converter = AnthropicSseToResponsesConverter::with_request(&json!({}));
    let mut out = converter.push_bytes(stream.as_bytes());
    out.extend(converter.finish());
    let events = collect_events(&out);
    assert!(
        !event_names(&events).contains(&"response.completed"),
        "{stream:?}"
    );
    assert_eq!(
        event_names(&events)
            .iter()
            .filter(|name| **name == "response.failed")
            .count(),
        1,
        "{stream:?}",
    );
    let error = &events
        .iter()
        .find(|(name, _)| name == "response.failed")
        .expect("missing response.failed")
        .1["response"]["error"];
    assert_eq!(error["code"], "invalid_sse_event", "{stream:?}");
    assert_eq!(error["type"], "invalid_sse_event", "{stream:?}");
    if let Some(marker) = raw_marker {
        assert!(
            !error["message"].as_str().unwrap().contains(marker),
            "raw SSE data leaked into error: {error}"
        );
    }
}

fn sse_event(name: &str, data: serde_json::Value) -> String {
    format!("event: {name}\ndata: {data}\n\n")
}

fn completed_response(events: &[(String, serde_json::Value)]) -> &serde_json::Value {
    &events
        .iter()
        .find(|(name, _)| name == "response.completed")
        .expect("missing response.completed")
        .1["response"]
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
    assert!(
        events[0].1["response"]["error"]["message"]
            .as_str()
            .unwrap()
            .contains("Overloaded")
    );
}

#[test]
fn split_utf8_across_chunks() {
    let mut converter = AnthropicSseToResponsesConverter::with_request(&serde_json::json!({}));
    let full = concat!(
        "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_utf8\",\"model\":\"k3\",\"usage\":{\"input_tokens\":1}}}\n\n",
        "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
        "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"中文\"}}\n\n",
        "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
        "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":1}}\n\n",
        "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
    );
    let bytes = full.as_bytes();
    let character = full.find("中").unwrap();
    let mut out = converter.push_bytes(&bytes[..character + 1]);
    out.extend(converter.push_bytes(&bytes[character + 1..]));
    out.extend(converter.finish());
    let events = collect_events(&out);
    assert_eq!(event_names(&events).last(), Some(&"response.completed"));
    assert_eq!(
        events
            .iter()
            .find(|(name, _)| name == "response.output_text.delta")
            .unwrap()
            .1["delta"],
        "中文"
    );
    assert!(!String::from_utf8(out).unwrap().contains('\u{fffd}'));
}

#[test]
fn finish_processes_trailing_block_without_blank_line() {
    let mut converter = AnthropicSseToResponsesConverter::with_request(&serde_json::json!({}));
    let stream = concat!(
        "event: message_start\n",
        "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_2\",\"model\":\"k3\",\"usage\":{\"input_tokens\":7}}}\n\n",
        "event: message_delta\n",
        "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":42}}\n\n",
        "event: message_stop\n",
        "data: {\"type\":\"message_stop\"}",
    );
    let mut out = converter.push_bytes(stream.as_bytes());
    out.extend(converter.finish());

    let events = collect_events(&out);
    assert_eq!(
        event_names(&events)
            .iter()
            .filter(|name| **name == "response.completed")
            .count(),
        1
    );
    let completed = events
        .iter()
        .find(|(name, _)| name == "response.completed")
        .unwrap();
    assert_eq!(completed.1["response"]["status"], "completed");
    assert_eq!(completed.1["response"]["usage"]["output_tokens"], 42);
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
    out.extend(converter.push_bytes(b"event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n"));
    out.extend(converter.finish());

    let events = collect_events(&out);
    let names: Vec<&str> = events.iter().map(|(name, _)| name.as_str()).collect();
    assert!(names.contains(&"response.failed"));
    assert!(!names.contains(&"response.completed"));
}

#[test]
fn anthropic_error_maps_to_responses_error() {
    let body = br#"{"type":"error","error":{"type":"invalid_request_error","message":"max_tokens is required"}}"#;
    let error = codex_plus_core::anthropic_proxy::anthropic_error_to_responses_error(400, body);
    assert_eq!(error["error"]["message"], "max_tokens is required");
    assert_eq!(error["error"]["type"], "invalid_request_error");
    assert_eq!(error["error"]["code"], 400);
}

#[test]
fn anthropic_error_maps_string_error_without_losing_text() {
    let body = br#"{"type":"error","error":"upstream refused the request"}"#;
    let error = codex_plus_core::anthropic_proxy::anthropic_error_to_responses_error(502, body);
    assert_eq!(error["error"]["message"], "upstream refused the request");
    assert_eq!(error["error"]["type"], "upstream_error");
    assert_eq!(error["error"]["code"], 502);
}

#[test]
fn anthropic_error_falls_back_on_non_json() {
    let error =
        codex_plus_core::anthropic_proxy::anthropic_error_to_responses_error(502, b"bad gateway");
    assert_eq!(error["error"]["code"], 502);
    assert!(
        error["error"]["message"]
            .as_str()
            .unwrap()
            .contains("bad gateway")
    );
}

#[test]
fn anthropic_request_builder_sets_auth_and_originator_headers() {
    let client = reqwest::Client::new();
    let builder = codex_plus_core::anthropic_proxy::anthropic_request_builder(
        client,
        "https://example.com/v1/messages",
        "sk-test",
        true,
        &serde_json::json!({"model":"k3"}),
        Some("codex_cli_rs"),
    );
    let request = builder.build().unwrap();
    assert_eq!(request.headers()["x-api-key"], "sk-test");
    assert_eq!(request.headers()["anthropic-version"], "2023-06-01");
    assert_eq!(request.headers()["authorization"], "Bearer sk-test");
    assert_eq!(request.headers()["originator"], "codex_cli_rs");
    assert_eq!(request.headers()["accept"], "text/event-stream");
}

use codex_plus_core::anthropic_proxy::anthropic_models_to_openai_models;
use codex_plus_core::protocol_proxy::anthropic_models_url;

#[test]
fn converts_anthropic_models_payload() {
    let body = json!({
        "data": [
            {"type":"model","id":"k3","display_name":"K3","created_at":"2026-07-15T00:00:00Z"},
            {"type":"model","id":"kimi-for-coding","display_name":"K2.7 Code","created_at":"2026-04-01T00:00:00Z"}
        ],
        "has_more": false
    });
    let out = anthropic_models_to_openai_models(&body);
    assert_eq!(out["object"], "list");
    let data = out["data"].as_array().unwrap();
    assert_eq!(data.len(), 2);
    assert_eq!(data[0]["id"], "k3");
    assert_eq!(data[0]["object"], "model");
    assert!(data[0]["created"].as_u64().unwrap() > 0);
    assert_eq!(data[1]["id"], "kimi-for-coding");
}

#[test]
fn anthropic_models_created_at_falls_back_to_zero() {
    let body = json!({
        "data": [
            {"type":"model","id":"bad-date","created_at":"not-a-date"},
            {"type":"model","id":"no-date"}
        ]
    });
    let out = anthropic_models_to_openai_models(&body);
    let data = out["data"].as_array().unwrap();
    assert_eq!(data[0]["created"], 0);
    assert_eq!(data[1]["created"], 0);
}

#[test]
fn anthropic_models_created_at_supports_offset() {
    // 带 +08:00 偏移：2026-01-01T08:00:00+08:00 == 2026-01-01T00:00:00Z
    let body = json!({
        "data": [
            {"type":"model","id":"a","created_at":"2026-01-01T08:00:00+08:00"},
            {"type":"model","id":"b","created_at":"2026-01-01T00:00:00Z"}
        ]
    });
    let out = anthropic_models_to_openai_models(&body);
    let data = out["data"].as_array().unwrap();
    assert_eq!(data[0]["created"], data[1]["created"]);
    assert!(data[0]["created"].as_u64().unwrap() > 0);
}

#[test]
fn passes_through_non_anthropic_payload() {
    let body = json!({"object":"list","data":[{"id":"gpt-5.5","object":"model"}]});
    let out = anthropic_models_to_openai_models(&body);
    assert_eq!(out, body);
}

#[test]
fn anthropic_models_url_rules() {
    // 带路径的 base 同样补 /v1（与 anthropic_messages_url 语义一致）
    assert_eq!(
        anthropic_models_url("https://api.kimi.com/coding"),
        "https://api.kimi.com/coding/v1/models"
    );
    assert_eq!(
        anthropic_models_url("https://api.kimi.com/coding/"),
        "https://api.kimi.com/coding/v1/models"
    );
    // `#` 后缀跳过版本前缀
    assert_eq!(
        anthropic_models_url("https://proxy.example.com/anthropic#"),
        "https://proxy.example.com/anthropic/models"
    );
    // 已带版本号不重复拼接
    assert_eq!(
        anthropic_models_url("https://api.example.com/v1"),
        "https://api.example.com/v1/models"
    );
    // 已是完整端点或以 /models 结尾则原样
    assert_eq!(
        anthropic_models_url("https://api.example.com/v1/models"),
        "https://api.example.com/v1/models"
    );
}

// ── Task 8：端到端集成测试（本地 mock 上游 + open_responses_proxy_request_with_settings）──
//
// mock 模式复用 tests/protocol_proxy.rs 的写法：tokio TcpListener 起本地 mock 上游，
// 按 Content-Length 读全请求体后返回固定响应；不引入新 mock 框架。

use codex_plus_core::protocol_proxy::{
    UpstreamWireApi, finalize_non_streaming_responses_response, open_responses_proxy_request,
    open_responses_proxy_request_with_settings,
};
use codex_plus_core::settings::{
    AggregateRelayMember, AggregateRelayProfile, AggregateRelayStrategy, RelayMode,
};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::task::JoinHandle;
use tokio::time::timeout;

const TASK8_TIMEOUT: Duration = Duration::from_secs(5);

/// 聚合测试共享进程级 GLOBAL_SELECTOR（relay_rotation.rs），串行执行避免相互干扰。
fn aggregate_test_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn settings_path_test_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

struct SettingsPathGuard {
    previous: Option<PathBuf>,
}

impl SettingsPathGuard {
    fn set(path: PathBuf) -> Self {
        let previous = codex_plus_core::paths::set_settings_path_for_tests(Some(path));
        Self { previous }
    }
}

impl Drop for SettingsPathGuard {
    fn drop(&mut self) {
        codex_plus_core::paths::set_settings_path_for_tests(self.previous.take());
    }
}

fn request_header_value<'a>(head: &'a str, expected_name: &str) -> &'a str {
    let mut values = head.lines().skip(1).filter_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.trim()
            .eq_ignore_ascii_case(expected_name)
            .then_some(value.trim())
    });
    let value = values
        .next()
        .unwrap_or_else(|| panic!("missing request header {expected_name:?} in:\n{head}"));
    assert!(
        values.next().is_none(),
        "duplicate request header {expected_name:?} in:\n{head}"
    );
    value
}

async fn read_complete_request(stream: &mut tokio::net::TcpStream, context: &str) -> String {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let read = timeout(TASK8_TIMEOUT, stream.read(&mut chunk))
            .await
            .unwrap_or_else(|_| panic!("timed out reading {context}"))
            .unwrap_or_else(|error| panic!("failed reading {context}: {error}"));
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
        let request = String::from_utf8_lossy(&buffer);
        let Some((headers, body)) = request.split_once("\r\n\r\n") else {
            continue;
        };
        let content_length = headers
            .lines()
            .find_map(|line| {
                line.split_once(':').and_then(|(name, value)| {
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())
                        .flatten()
                })
            })
            .unwrap_or(0);
        if body.len() >= content_length {
            break;
        }
    }
    String::from_utf8_lossy(&buffer).to_string()
}

async fn join_server<T>(handle: &mut JoinHandle<T>, context: &str) -> T {
    match timeout(TASK8_TIMEOUT, &mut *handle).await {
        Ok(result) => result.unwrap_or_else(|error| panic!("{context} task failed: {error}")),
        Err(_) => {
            handle.abort();
            panic!("timed out joining {context}");
        }
    }
}

/// 按 Content-Length 读全一个 HTTP 请求，返回请求原文，然后写回固定响应。
async fn capture_request_and_respond(
    listener: tokio::net::TcpListener,
    response: String,
) -> String {
    let (mut stream, _) = timeout(TASK8_TIMEOUT, listener.accept())
        .await
        .expect("timed out accepting mock upstream connection")
        .expect("failed accepting mock upstream connection");
    let request = read_complete_request(&mut stream, "mock upstream request").await;
    timeout(TASK8_TIMEOUT, stream.write_all(response.as_bytes()))
        .await
        .expect("timed out writing mock upstream response")
        .expect("failed writing mock upstream response");
    request
}

/// 构造 200 JSON 响应（content-length 按字节数计算）。
fn json_ok_response(body: &str) -> String {
    format!(
        "HTTP/1.1 200 OK\r\ncontent-length: {}\r\ncontent-type: application/json\r\nconnection: close\r\n\r\n{}",
        body.len(),
        body
    )
}

/// 构造单个 anthropic profile 的 BackendSettings。
fn anthropic_settings(base_url: String) -> BackendSettings {
    BackendSettings {
        relay_profiles: vec![RelayProfile {
            id: "task8-anthropic-profile-unique".to_string(),
            name: "Task 8 Anthropic Profile".to_string(),
            base_url,
            api_key: "sk-task8-anthropic-test".to_string(),
            protocol: RelayProtocol::Anthropic,
            relay_mode: RelayMode::MixedApi,
            ..RelayProfile::default()
        }],
        active_relay_id: "task8-anthropic-profile-unique".to_string(),
        ..BackendSettings::default()
    }
}

#[tokio::test]
async fn anthropic_responses_proxy_prefers_original_user_agent_over_profile_user_agent() {
    let _lock = settings_path_test_lock().lock().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let _guard = SettingsPathGuard::set(temp.path().join("settings.json"));
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    let response_body = json!({
        "id": "msg_user_agent",
        "type": "message",
        "role": "assistant",
        "model": "ua-model",
        "stop_reason": "end_turn",
        "usage": {"input_tokens": 1, "output_tokens": 1},
        "content": [{"type": "text", "text": "ok"}],
    })
    .to_string();
    let mut server = tokio::spawn(capture_request_and_respond(
        listener,
        json_ok_response(&response_body),
    ));

    let settings = json!({
        "relayProfiles": [{
            "id": "task8-anthropic-profile-unique",
            "name": "Task 8 Anthropic Profile",
            "baseUrl": format!("http://{addr}/v1"),
            "upstreamBaseUrl": format!("http://{addr}/v1"),
            "apiKey": "sk-task8-anthropic-test",
            "protocol": "anthropic",
            "relayMode": "mixedApi",
            "userAgent": "Configured-Anthropic-UA/1.0",
        }],
        "activeRelayId": "task8-anthropic-profile-unique",
    });
    std::fs::write(
        temp.path().join("settings.json"),
        serde_json::to_vec_pretty(&settings).unwrap(),
    )
    .unwrap();

    let upstream = timeout(
        TASK8_TIMEOUT,
        open_responses_proxy_request(
            r#"{"model":"ua-model","input":"hello","stream":false}"#,
            Some("Original-Anthropic-UA/1.0"),
            None,
        ),
    )
    .await
    .expect("timed out opening Anthropic User-Agent proxy request")
    .expect("Anthropic User-Agent proxy request failed");
    assert_eq!(upstream.status_code, 200);
    assert_eq!(upstream.wire_api, UpstreamWireApi::AnthropicMessages);

    let request = join_server(&mut server, "Anthropic User-Agent mock server").await;
    let (head, body) = request.split_once("\r\n\r\n").unwrap();
    assert!(head.starts_with("POST /v1/messages HTTP/1.1"), "{head}");
    let sent: serde_json::Value = serde_json::from_str(body).unwrap();
    assert_eq!(
        sent["messages"],
        json!([{"role":"user","content":[{"type":"text","text":"hello"}]}])
    );
    assert_eq!(
        request_header_value(head, "user-agent"),
        "Original-Anthropic-UA/1.0"
    );
}

#[tokio::test]
async fn end_to_end_responses_to_anthropic_roundtrip() {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    let anthropic_body = json!({
        "id": "msg_task8_roundtrip_unique",
        "type": "message",
        "role": "assistant",
        "model": "task8-roundtrip-model",
        "stop_reason": "tool_use",
        "usage": {"input_tokens": 10, "output_tokens": 5},
        "content": [
            {"type": "text", "text": "Task 8 roundtrip text"},
            {"type": "tool_use", "id": "toolu_task8_roundtrip_unique", "name": "shell", "input": {"command": ["ls", "task8-roundtrip"]}}
        ]
    })
    .to_string();
    let mut server = tokio::spawn(capture_request_and_respond(
        listener,
        json_ok_response(&anthropic_body),
    ));

    let settings = anthropic_settings(format!("http://{addr}/v1"));
    let codex_body = json!({
        "model": "task8-roundtrip-model",
        "instructions": "Task 8 roundtrip system instruction.",
        "max_output_tokens": 1234,
        "input": [{"type": "message", "role": "user", "content": [{"type": "input_text", "text": "Task 8 roundtrip user text"}]}],
        "stream": false
    });
    let upstream = timeout(
        TASK8_TIMEOUT,
        open_responses_proxy_request_with_settings(&codex_body.to_string(), settings, None),
    )
    .await
    .expect("timed out opening Task 8 roundtrip proxy request")
    .expect("Task 8 roundtrip proxy request failed");

    let request = join_server(&mut server, "Task 8 roundtrip mock server").await;
    let (head, body) = request.split_once("\r\n\r\n").unwrap();
    assert!(head.starts_with("POST /v1/messages HTTP/1.1"), "{head}");
    assert_eq!(
        request_header_value(head, "x-api-key"),
        "sk-task8-anthropic-test"
    );
    assert_eq!(
        request_header_value(head, "anthropic-version"),
        "2023-06-01"
    );
    assert_eq!(
        request_header_value(head, "authorization"),
        "Bearer sk-task8-anthropic-test"
    );
    assert_eq!(
        request_header_value(head, "content-type"),
        "application/json"
    );
    let sent: serde_json::Value = serde_json::from_str(body).unwrap();
    assert_eq!(sent["model"], "task8-roundtrip-model");
    assert_eq!(
        sent["system"],
        json!([{
            "type": "text",
            "text": "Task 8 roundtrip system instruction.",
            "cache_control": {"type": "ephemeral"}
        }])
    );
    assert_eq!(sent["max_tokens"], 1234);
    assert_eq!(
        sent["messages"],
        json!([{
            "role": "user",
            "content": [{"type": "text", "text": "Task 8 roundtrip user text"}]
        }])
    );

    assert_eq!(upstream.status_code, 200);
    assert_eq!(upstream.wire_api, UpstreamWireApi::AnthropicMessages);
    let wire_api = upstream.wire_api;
    let upstream_content_type = upstream.content_type.clone();
    let upstream_bytes = timeout(TASK8_TIMEOUT, upstream.response.bytes())
        .await
        .expect("timed out reading Task 8 roundtrip upstream response")
        .expect("failed reading Task 8 roundtrip upstream response");
    let finalized = finalize_non_streaming_responses_response(
        wire_api,
        &upstream_content_type,
        &upstream_bytes,
        Some(&codex_body),
    )
    .unwrap();
    assert_eq!(finalized.status, "200 OK");
    assert_eq!(finalized.content_type, "application/json; charset=utf-8");
    let converted: serde_json::Value = serde_json::from_slice(&finalized.body).unwrap();
    assert_eq!(converted["id"], "msg_task8_roundtrip_unique");
    assert_eq!(converted["object"], "response");
    assert_eq!(converted["status"], "completed");
    assert_eq!(converted["model"], "task8-roundtrip-model");
    assert_eq!(converted["usage"]["input_tokens"], 10);
    assert_eq!(converted["usage"]["output_tokens"], 5);
    assert_eq!(converted["usage"]["total_tokens"], 15);
    assert_eq!(converted["output"][0]["type"], "message");
    assert_eq!(converted["output"][0]["role"], "assistant");
    assert_eq!(converted["output"][0]["content"][0]["type"], "output_text");
    assert_eq!(
        converted["output"][0]["content"][0]["text"],
        "Task 8 roundtrip text"
    );
    assert_eq!(converted["output"][1]["type"], "function_call");
    assert_eq!(
        converted["output"][1]["call_id"],
        "toolu_task8_roundtrip_unique"
    );
    assert_eq!(converted["output"][1]["name"], "shell");
    assert_eq!(
        converted["output"][1]["arguments"],
        "{\"command\":[\"ls\",\"task8-roundtrip\"]}"
    );
}

#[tokio::test]
async fn anthropic_strip_image_handling_applies_before_conversion() {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    let anthropic_body = json!({
        "id": "msg_task8_vlm_strip_unique",
        "type": "message",
        "role": "assistant",
        "model": "task8-vlm-strip-model",
        "stop_reason": "end_turn",
        "usage": {"input_tokens": 5, "output_tokens": 2},
        "content": [{"type": "text", "text": "Task 8 VLM response"}]
    })
    .to_string();
    let mut server = tokio::spawn(capture_request_and_respond(
        listener,
        json_ok_response(&anthropic_body),
    ));

    let mut settings = anthropic_settings(format!("http://{addr}/v1"));
    settings.relay_profiles[0].model_vlm = r#"{"task8-vlm-strip-model":"strip"}"#.to_string();
    let codex_body = json!({
        "model": "task8-vlm-strip-model",
        "input": [{"type": "message", "role": "user", "content": [
            {"type": "input_text", "text": "Task 8 VLM retained text"},
            {"type": "input_image", "image_url": "data:image/png;base64,TASK8VLMUNIQUE="}
        ]}],
        "stream": false
    });
    let upstream = timeout(
        TASK8_TIMEOUT,
        open_responses_proxy_request_with_settings(&codex_body.to_string(), settings, None),
    )
    .await
    .expect("timed out opening Task 8 VLM proxy request")
    .expect("Task 8 VLM proxy request failed");
    assert_eq!(upstream.status_code, 200);
    assert_eq!(upstream.wire_api, UpstreamWireApi::AnthropicMessages);

    let request = join_server(&mut server, "Task 8 VLM mock server").await;
    let (head, body) = request.split_once("\r\n\r\n").unwrap();
    assert!(head.starts_with("POST /v1/messages HTTP/1.1"), "{head}");
    assert_eq!(
        request_header_value(head, "content-type"),
        "application/json"
    );
    let sent: serde_json::Value = serde_json::from_str(body).unwrap();
    assert_eq!(sent["model"], "task8-vlm-strip-model");
    assert_eq!(sent["stream"], false);
    assert_eq!(sent["messages"].as_array().unwrap().len(), 1);
    assert_eq!(sent["messages"][0]["role"], "user");
    let content = sent["messages"][0]["content"].as_array().unwrap();
    assert_eq!(content.len(), 1, "{sent}");
    assert_eq!(content[0]["type"], "text");
    assert_eq!(content[0]["text"], "Task 8 VLM retained text");
    assert!(
        content.iter().all(|block| block["type"] != "image"),
        "{sent}"
    );
    assert!(!body.contains("TASK8VLMUNIQUE="), "{sent}");
    timeout(TASK8_TIMEOUT, upstream.response.bytes())
        .await
        .expect("timed out reading Task 8 VLM upstream response")
        .expect("failed reading Task 8 VLM upstream response");
}

#[tokio::test]
async fn aggregate_failover_to_anthropic_member() {
    let _lock = aggregate_test_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let first = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let first_addr = first.local_addr().unwrap();
    let mut first_server = tokio::spawn(async move {
        let (mut stream, _) = timeout(TASK8_TIMEOUT, first.accept())
            .await
            .expect("timed out accepting Task 8 aggregate member A connection")
            .expect("failed accepting Task 8 aggregate member A connection");
        let request = read_complete_request(&mut stream, "Task 8 aggregate member A request").await;
        assert!(
            request.starts_with("POST /v1/chat/completions HTTP/1.1"),
            "{request}"
        );
        let (head, body) = request.split_once("\r\n\r\n").unwrap();
        assert_eq!(
            request_header_value(head, "authorization"),
            "Bearer sk-task8-aggregate-member-a"
        );
        assert_eq!(
            request_header_value(head, "content-type"),
            "application/json"
        );
        let sent: serde_json::Value = serde_json::from_str(body).unwrap();
        assert_eq!(sent["model"], "task8-aggregate-model");
        assert_eq!(sent["stream"], false);
        assert_eq!(sent["messages"].as_array().unwrap().len(), 1);
        assert_eq!(sent["messages"][0]["role"], "user");
        assert_eq!(sent["messages"][0]["content"], "Task 8 aggregate user text");
        assert_eq!(
            sent["messages"],
            json!([{"role": "user", "content": "Task 8 aggregate user text"}])
        );
        timeout(
            TASK8_TIMEOUT,
            stream.write_all(
                b"HTTP/1.1 500 Internal Server Error\r\ncontent-length: 11\r\ncontent-type: application/json\r\nconnection: close\r\n\r\n{\"error\":1}",
            ),
        )
        .await
        .expect("timed out writing Task 8 aggregate member A response")
        .expect("failed writing Task 8 aggregate member A response");
    });

    let second = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let second_addr = second.local_addr().unwrap();
    let anthropic_body = json!({
        "id": "msg_task8_aggregate_member_b_unique",
        "type": "message",
        "role": "assistant",
        "model": "task8-aggregate-model",
        "stop_reason": "end_turn",
        "usage": {"input_tokens": 8, "output_tokens": 3},
        "content": [{"type": "text", "text": "Task 8 response from aggregate member B"}]
    })
    .to_string();
    let mut second_server = tokio::spawn(capture_request_and_respond(
        second,
        json_ok_response(&anthropic_body),
    ));

    let first_id = "task8-aggregate-member-a-chat-unique".to_string();
    let second_id = "task8-aggregate-member-b-anthropic-unique".to_string();
    let aggregate_id = "task8-aggregate-profile-unique".to_string();
    let settings = BackendSettings {
        relay_profiles: vec![
            RelayProfile {
                id: first_id.clone(),
                name: "Task 8 Aggregate Member A".to_string(),
                base_url: format!("http://{first_addr}/v1"),
                api_key: "sk-task8-aggregate-member-a".to_string(),
                protocol: RelayProtocol::ChatCompletions,
                relay_mode: RelayMode::MixedApi,
                ..RelayProfile::default()
            },
            RelayProfile {
                id: second_id.clone(),
                name: "Task 8 Aggregate Member B".to_string(),
                base_url: format!("http://{second_addr}/v1"),
                api_key: "sk-task8-aggregate-member-b".to_string(),
                protocol: RelayProtocol::Anthropic,
                relay_mode: RelayMode::MixedApi,
                ..RelayProfile::default()
            },
            RelayProfile {
                id: aggregate_id.clone(),
                name: "Task 8 Aggregate Profile".to_string(),
                relay_mode: RelayMode::Aggregate,
                ..RelayProfile::default()
            },
        ],
        active_relay_id: aggregate_id.clone(),
        active_aggregate_relay_id: aggregate_id.clone(),
        aggregate_relay_profiles: vec![AggregateRelayProfile {
            id: aggregate_id,
            name: "Task 8 Aggregate Profile".to_string(),
            strategy: AggregateRelayStrategy::Failover,
            members: vec![
                AggregateRelayMember {
                    relay_id: first_id,
                    weight: 1,
                },
                AggregateRelayMember {
                    relay_id: second_id,
                    weight: 1,
                },
            ],
        }],
        ..BackendSettings::default()
    };

    let codex_body = json!({
        "model": "task8-aggregate-model",
        "input": [{"type": "message", "role": "user", "content": [{"type": "input_text", "text": "Task 8 aggregate user text"}]}],
        "stream": false
    });
    let upstream = timeout(
        TASK8_TIMEOUT,
        open_responses_proxy_request_with_settings(&codex_body.to_string(), settings, None),
    )
    .await
    .expect("timed out opening Task 8 aggregate proxy request")
    .expect("Task 8 aggregate proxy request failed");

    join_server(&mut first_server, "Task 8 aggregate member A server").await;
    let second_request = join_server(&mut second_server, "Task 8 aggregate member B server").await;
    let (second_head, second_body) = second_request.split_once("\r\n\r\n").unwrap();
    assert!(
        second_head.starts_with("POST /v1/messages HTTP/1.1"),
        "{second_head}"
    );
    assert_eq!(
        request_header_value(second_head, "x-api-key"),
        "sk-task8-aggregate-member-b"
    );
    assert_eq!(
        request_header_value(second_head, "anthropic-version"),
        "2023-06-01"
    );
    assert_eq!(
        request_header_value(second_head, "authorization"),
        "Bearer sk-task8-aggregate-member-b"
    );
    assert_eq!(
        request_header_value(second_head, "content-type"),
        "application/json"
    );
    let second_sent: serde_json::Value = serde_json::from_str(second_body).unwrap();
    assert_eq!(second_sent["model"], "task8-aggregate-model");
    assert_eq!(second_sent["stream"], false);
    assert_eq!(second_sent["messages"].as_array().unwrap().len(), 1);
    assert_eq!(second_sent["messages"][0]["role"], "user");
    assert_eq!(
        second_sent["messages"][0]["content"][0]["text"],
        "Task 8 aggregate user text"
    );
    assert_eq!(
        second_sent["messages"],
        json!([{
            "role": "user",
            "content": [{"type": "text", "text": "Task 8 aggregate user text"}]
        }])
    );
    assert_eq!(upstream.status_code, 200);
    assert_eq!(upstream.wire_api, UpstreamWireApi::AnthropicMessages);
    let wire_api = upstream.wire_api;
    let upstream_content_type = upstream.content_type.clone();
    let upstream_bytes = timeout(TASK8_TIMEOUT, upstream.response.bytes())
        .await
        .expect("timed out reading Task 8 aggregate upstream response")
        .expect("failed reading Task 8 aggregate upstream response");
    let finalized = finalize_non_streaming_responses_response(
        wire_api,
        &upstream_content_type,
        &upstream_bytes,
        Some(&codex_body),
    )
    .unwrap();
    assert_eq!(finalized.status, "200 OK");
    assert_eq!(finalized.content_type, "application/json; charset=utf-8");
    let converted: serde_json::Value = serde_json::from_slice(&finalized.body).unwrap();
    assert_eq!(converted["id"], "msg_task8_aggregate_member_b_unique");
    assert_eq!(converted["object"], "response");
    assert_eq!(converted["status"], "completed");
    assert_eq!(converted["model"], "task8-aggregate-model");
    assert_eq!(converted["usage"]["input_tokens"], 8);
    assert_eq!(converted["usage"]["output_tokens"], 3);
    assert_eq!(converted["usage"]["total_tokens"], 11);
    assert_eq!(converted["output"].as_array().unwrap().len(), 1);
    assert_eq!(converted["output"][0]["type"], "message");
    assert_eq!(converted["output"][0]["role"], "assistant");
    assert_eq!(converted["output"][0]["content"][0]["type"], "output_text");
    assert_eq!(
        converted["output"][0]["content"][0]["text"],
        "Task 8 response from aggregate member B"
    );
}

#[test]
fn stream_terminal_eof_and_post_terminal_rules() {
    for (reason, terminal_name, status) in [
        ("end_turn", "response.completed", "completed"),
        ("max_tokens", "response.incomplete", "incomplete"),
    ] {
        let mut converter = AnthropicSseToResponsesConverter::with_request(&json!({}));
        let stream = format!(
            "{}{}{}",
            message_start("msg_terminal"),
            sse_event(
                "message_delta",
                json!({"type":"message_delta","delta":{"stop_reason":reason},"usage":{"output_tokens":2}}),
            ),
            raw_message_stop(),
        );
        let output = converter.push_bytes(stream.as_bytes());
        let events = collect_events(&output);
        assert_eq!(
            events
                .iter()
                .filter(|(name, _)| matches!(
                    name.as_str(),
                    "response.completed" | "response.incomplete" | "response.failed"
                ))
                .count(),
            1
        );
        let response = &events
            .iter()
            .find(|(name, _)| name == terminal_name)
            .unwrap()
            .1["response"];
        assert_eq!(response["status"], status);
        if reason == "max_tokens" {
            assert_eq!(
                response["incomplete_details"],
                json!({"reason": "max_output_tokens"})
            );
        }
        assert!(
            converter
                .push_bytes(raw_message_stop().as_bytes())
                .is_empty()
        );
        assert!(
            converter
                .fail("late".into(), Some("late".into()))
                .is_empty()
        );
        assert!(converter.finish().is_empty());
        assert!(
            converter
                .push_bytes(message_start("late").as_bytes())
                .is_empty()
        );
    }

    let mut converter = AnthropicSseToResponsesConverter::with_request(&json!({}));
    let mut output = converter.push_bytes(message_start("msg_eof").as_bytes());
    output.extend(converter.finish());
    let events = collect_events(&output);
    assert_eq!(
        events
            .iter()
            .filter(|(name, _)| name == "response.completed")
            .count(),
        0
    );
    assert_eq!(
        events
            .iter()
            .filter(|(name, _)| name == "response.incomplete")
            .count(),
        0
    );
    let failed = events
        .iter()
        .filter(|(name, _)| name == "response.failed")
        .collect::<Vec<_>>();
    assert_eq!(failed.len(), 1);
    let error = &failed[0].1["response"]["error"];
    assert_eq!(error["code"], "unexpected_eof");
    assert_eq!(error["type"], "unexpected_eof");
}

#[test]
fn failed_response_preserves_done_output_and_request_fields() {
    let request = json!({
        "instructions": "keep this",
        "max_output_tokens": 17,
        "metadata": {"trace": "failed"},
        "store": false,
    });
    for invalid_tail in [None, Some("event: message_delta\ndata: not-json\n\n")] {
        let mut converter = AnthropicSseToResponsesConverter::with_request(&request);
        let stream = format!(
            "{}{}{}{}",
            message_start("msg_failed_output"),
            sse_event(
                "content_block_start",
                json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}),
            ),
            sse_event(
                "content_block_delta",
                json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"done"}}),
            ),
            sse_event(
                "content_block_stop",
                json!({"type":"content_block_stop","index":0}),
            ),
        );
        let mut out = converter.push_bytes(stream.as_bytes());
        if let Some(tail) = invalid_tail {
            out.extend(converter.push_bytes(tail.as_bytes()));
        } else {
            out.extend(converter.finish());
        }
        let events = collect_events(&out);
        let done = events
            .iter()
            .find(|(name, _)| name == "response.output_item.done")
            .expect("missing completed output item")
            .1["item"]
            .clone();
        let failed = &events
            .iter()
            .find(|(name, _)| name == "response.failed")
            .expect("missing response.failed")
            .1["response"];
        assert_eq!(failed["output"], json!([done]));
        for key in ["instructions", "max_output_tokens", "metadata", "store"] {
            assert_eq!(failed[key], request[key], "{key}");
        }
    }
}

#[test]
fn invalid_sse_validation_matrix() {
    for name in [
        "message_start",
        "content_block_start",
        "content_block_delta",
        "content_block_stop",
        "message_delta",
        "message_stop",
        "error",
    ] {
        assert_invalid_sse_event(&format!("event: {name}\n\n"), None);
        let marker = format!("RAW_{name}");
        assert_invalid_sse_event(&format!("event: {name}\ndata: {marker}\n\n"), Some(&marker));
        assert_invalid_sse_event(&sse_event(name, json!({"type":"ping"})), None);
    }
    for payload in [
        json!({"type":"message_start"}),
        json!({"type":"message_start","message":null}),
        json!({"type":"message_start","message":{"id":7,"model":"k3","usage":{}}}),
        json!({"type":"message_start","message":{"id":"m","model":7,"usage":{}}}),
        json!({"type":"message_start","message":{"id":"m","model":"k3","usage":7}}),
    ] {
        assert_invalid_sse_event(&sse_event("message_start", payload), None);
    }
    for payload in [
        json!({"type":"message_start","message":{"model":"k3","usage":{}}}),
        json!({"type":"message_start","message":{"id":"m","usage":{}}}),
        json!({"type":"message_start","message":{"id":"m","model":"k3"}}),
    ] {
        assert_invalid_sse_event(&sse_event("message_start", payload), None);
    }
    for payload in [
        json!({"type":"content_block_start","content_block":{"type":"text","text":""}}),
        json!({"type":"content_block_start","index":"0","content_block":{"type":"text","text":""}}),
        json!({"type":"content_block_start","index":0}),
        json!({"type":"content_block_start","index":0,"content_block":null}),
        json!({"type":"content_block_start","index":0,"content_block":{}}),
        json!({"type":"content_block_start","index":0,"content_block":{"type":7}}),
        json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":7}}),
        json!({"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":7,"name":"shell","input":{}}}),
        json!({"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"call","name":7,"input":{}}}),
        json!({"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"call","name":"shell","input":7}}),
        json!({"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":7,"signature":"sig"}}),
        json!({"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"why","signature":7}}),
    ] {
        assert_invalid_sse_event(&sse_event("content_block_start", payload), None);
    }
    for payload in [
        json!({"type":"content_block_delta","delta":{"type":"text_delta","text":"x"}}),
        json!({"type":"content_block_delta","index":"0","delta":{"type":"text_delta","text":"x"}}),
        json!({"type":"content_block_delta","index":0}),
        json!({"type":"content_block_delta","index":0,"delta":null}),
        json!({"type":"content_block_delta","index":0,"delta":{}}),
        json!({"type":"content_block_delta","index":0,"delta":{"type":7}}),
        json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":7}}),
        json!({"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":7}}),
        json!({"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":7}}),
        json!({"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":7}}),
    ] {
        assert_invalid_sse_event(&sse_event("content_block_delta", payload), None);
    }
    for payload in [
        json!({"type":"content_block_start","index":0,"content_block":{"type":"text"}}),
        json!({"type":"content_block_start","index":0,"content_block":{"type":"tool_use","name":"shell","input":{}}}),
        json!({"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"call","input":{}}}),
        json!({"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"call","name":"shell"}}),
        json!({"type":"content_block_start","index":0,"content_block":{"type":"thinking","signature":"sig"}}),
    ] {
        assert_invalid_sse_event(&sse_event("content_block_start", payload), None);
    }
    for subtype in [
        "text_delta",
        "input_json_delta",
        "thinking_delta",
        "signature_delta",
    ] {
        assert_invalid_sse_event(
            &sse_event(
                "content_block_delta",
                json!({"type":"content_block_delta","index":0,"delta":{"type":subtype}}),
            ),
            None,
        );
    }
    for payload in [
        json!({"type":"content_block_stop"}),
        json!({"type":"content_block_stop","index":"0"}),
    ] {
        assert_invalid_sse_event(&sse_event("content_block_stop", payload), None);
    }
    for payload in [
        json!({"type":"message_delta"}),
        json!({"type":"message_delta","delta":null,"usage":{}}),
        json!({"type":"message_delta","delta":{},"usage":null}),
        json!({"type":"message_delta","delta":{"stop_reason":7},"usage":{"output_tokens":1}}),
        json!({"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":null}}),
        json!({"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":"1"}}),
        json!({"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"input_tokens":-1,"output_tokens":1}}),
        json!({"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"cache_creation_input_tokens":"1","output_tokens":1}}),
        json!({"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"cache_read_input_tokens":false,"output_tokens":1}}),
        json!({"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":1,"output_tokens_details":7}}),
        json!({"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":1,"server_tool_use":[]}}),
    ] {
        assert_invalid_sse_event(&sse_event("message_delta", payload), None);
    }
    for payload in [
        json!({"type":"message_delta","usage":{"output_tokens":1}}),
        json!({"type":"message_delta","delta":{"stop_reason":"end_turn"}}),
        json!({"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{}}),
    ] {
        assert_invalid_sse_event(&sse_event("message_delta", payload), None);
    }
    assert_invalid_sse_event(&sse_event("message_stop", json!({"type":7})), None);
    for payload in [
        json!({"type":"error"}),
        json!({"type":"error","error":null}),
        json!({"type":"error","error":{}}),
        json!({"type":"error","error":{"type":7,"message":"bad"}}),
        json!({"type":"error","error":{"type":"bad","message":false}}),
    ] {
        assert_invalid_sse_event(&sse_event("error", payload), None);
    }
    for payload in [
        json!({"type":"error","error":{"message":"bad"}}),
        json!({"type":"error","error":{"type":"bad"}}),
    ] {
        assert_invalid_sse_event(&sse_event("error", payload), None);
    }
    for tail in [
        "event: message_delta\ndata: {\"type\":\"message_delta\"",
        "event: content_block_stop\ndata: RAW_TRAILING",
    ] {
        assert_invalid_sse_event(
            &format!("{}{tail}", message_start("bad_tail")),
            Some("RAW_TRAILING"),
        );
    }
    let open = format!(
        "{}{}{}",
        message_start("open"),
        sse_event(
            "content_block_start",
            json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}})
        ),
        message_stop()
    );
    assert_invalid_sse_event(&open, None);
}

#[test]
fn ping_unknown_and_block_index_rules() {
    let mut c = AnthropicSseToResponsesConverter::with_request(&json!({}));
    let stream = format!(
        "{}{}{}{}",
        sse_event("ping", json!({"type":"ping"})),
        sse_event("vendor", json!({"x":1})),
        message_start("ignored"),
        message_stop()
    );
    let mut out = c.push_bytes(stream.as_bytes());
    out.extend(c.finish());
    assert_eq!(
        event_names(&collect_events(&out)),
        vec![
            "response.created",
            "response.in_progress",
            "response.completed"
        ]
    );
    for name in ["content_block_delta", "content_block_stop"] {
        let payload = if name == "content_block_delta" {
            json!({"type":name,"index":99,"delta":{"type":"text_delta","text":"x"}})
        } else {
            json!({"type":name,"index":99})
        };
        assert_invalid_sse_event(
            &format!("{}{}", message_start("unknown"), sse_event(name, payload)),
            None,
        );
    }
}

#[test]
fn duplicate_and_abnormal_start_fail_without_overwrite() {
    let first = sse_event(
        "content_block_start",
        json!({"type":"content_block_start","index":4,"content_block":{"type":"text","text":"original"}}),
    );
    let duplicate = sse_event(
        "content_block_start",
        json!({"type":"content_block_start","index":4,"content_block":{"type":"tool_use","id":"replacement","name":"bad","input":{}}}),
    );
    let mut c = AnthropicSseToResponsesConverter::with_request(&json!({}));
    let out = c.push_bytes(format!("{}{}{}", message_start("dup"), first, duplicate).as_bytes());
    let events = collect_events(&out);
    assert_eq!(event_names(&events).last(), Some(&"response.failed"));
    assert_eq!(
        events
            .iter()
            .filter(|(n, _)| n == "response.output_item.added")
            .count(),
        1
    );
    assert!(!String::from_utf8(out).unwrap().contains("replacement"));

    let stopped_then_reused = format!(
        "{}{}{}{}",
        message_start("reused"),
        first,
        sse_event(
            "content_block_stop",
            json!({"type":"content_block_stop","index":4}),
        ),
        duplicate,
    );
    let mut c = AnthropicSseToResponsesConverter::with_request(&json!({}));
    let out = c.push_bytes(stopped_then_reused.as_bytes());
    let events = collect_events(&out);
    assert_eq!(event_names(&events).last(), Some(&"response.failed"));
    assert_eq!(
        events
            .iter()
            .filter(|(name, _)| name == "response.output_item.added")
            .count(),
        1
    );
    assert!(!String::from_utf8(out).unwrap().contains("replacement"));

    assert_invalid_sse_event(
        &sse_event(
            "content_block_start",
            json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}),
        ),
        None,
    );
}

#[test]
fn completed_and_nonstream_echo_request_fields_and_done_output() {
    let request = json!({"instructions":"precise","max_output_tokens":17,"parallel_tool_calls":true,"previous_response_id":"prev","reasoning":{"effort":"high"},"temperature":0.25,"tool_choice":{"type":"function","name":"shell"},"tools":[{"type":"function","name":"shell","parameters":{"type":"object"}}],"top_p":0.9,"metadata":{"trace":"abc"},"store":false});
    let keys = [
        "instructions",
        "max_output_tokens",
        "parallel_tool_calls",
        "previous_response_id",
        "reasoning",
        "temperature",
        "tool_choice",
        "tools",
        "top_p",
        "metadata",
        "store",
    ];
    let mut c = AnthropicSseToResponsesConverter::with_request(&request);
    let stream = format!(
        "{}{}{}{}{}{}{}{}",
        message_start("msg_echo"),
        sse_event(
            "content_block_start",
            json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}})
        ),
        sse_event(
            "content_block_delta",
            json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"done"}})
        ),
        sse_event(
            "content_block_stop",
            json!({"type":"content_block_stop","index":0})
        ),
        sse_event(
            "content_block_start",
            json!({"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"call_echo","name":"shell","input":{}}})
        ),
        sse_event(
            "content_block_delta",
            json!({"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"ok\":true}"}})
        ),
        sse_event(
            "content_block_stop",
            json!({"type":"content_block_stop","index":1})
        ),
        message_stop()
    );
    let mut out = c.push_bytes(stream.as_bytes());
    out.extend(c.finish());
    let events = collect_events(&out);
    let response = completed_response(&events);
    let done: Vec<_> = events
        .iter()
        .filter(|(n, _)| n == "response.output_item.done")
        .map(|(_, data)| data["item"].clone())
        .collect();
    assert_eq!(done.len(), 2);
    assert_eq!(response["output"], serde_json::Value::Array(done));
    for key in keys {
        assert_eq!(response[key], request[key], "SSE echo {key}");
    }
    let body =
        json!({"id":"msg_nonstream_echo","model":"k3","stop_reason":"end_turn","content":[]});
    let response = anthropic_message_to_response(&body, Some(&request)).unwrap();
    for key in keys {
        assert_eq!(response[key], request[key], "nonstream echo {key}");
    }
}

#[test]
fn function_call_output_normalization() {
    for (output, expected) in [
        (json!("unchanged"), json!("unchanged")),
        (serde_json::Value::Null, json!("")),
        (
            json!({"b":[2,true],"a":1}),
            json!("{\"a\":1,\"b\":[2,true]}"),
        ),
        (
            json!([1, "x", false]),
            json!([
                {"type":"text","text":"1"},
                {"type":"text","text":"\"x\""},
                {"type":"text","text":"false"}
            ]),
        ),
    ] {
        let converted = responses_to_anthropic_messages(
            &json!({"input":[{"type":"function_call_output","call_id":"c1","output":output}]}),
        )
        .unwrap();
        assert_eq!(converted["messages"][0]["content"][0]["content"], expected);
    }
    let converted = responses_to_anthropic_messages(
        &json!({"input":[{"type":"function_call_output","call_id":"c1"}]}),
    )
    .unwrap();
    assert_eq!(converted["messages"][0]["content"][0]["content"], "");
}

#[test]
fn signed_reasoning_and_thinking_lifecycle() {
    for (item, expected) in [
        (
            json!({"type":"reasoning","signature":"summary","summary":[{"text":"one"},{"text":"two"}],"content":[{"text":"fallback"}]}),
            "one\ntwo",
        ),
        (
            json!({"type":"reasoning","signature":"content","content":[{"text":"alpha"},{"text":"beta"}]}),
            "alpha\nbeta",
        ),
        (json!({"type":"reasoning","signature":"empty"}), ""),
    ] {
        let converted = responses_to_anthropic_messages(&json!({"input":[item]})).unwrap();
        assert_eq!(converted["messages"][0]["content"][0]["type"], "thinking");
        assert_eq!(converted["messages"][0]["content"][0]["thinking"], expected);
    }
    let body = json!({"id":"msg_thinking","model":"k3","stop_reason":"end_turn","content":[{"type":"thinking","thinking":"initial","signature":"sig-"}]});
    let nonstream = anthropic_message_to_response(&body, None).unwrap();
    assert_eq!(nonstream["output"][0]["summary"][0]["text"], "initial");
    assert_eq!(nonstream["output"][0]["signature"], "sig-");
    let mut c = AnthropicSseToResponsesConverter::with_request(&json!({}));
    let stream = format!(
        "{}{}{}{}{}{}",
        message_start("msg_thinking"),
        sse_event(
            "content_block_start",
            json!({"type":"content_block_start","index":3,"content_block":{"type":"thinking","thinking":"initial","signature":"sig-"}})
        ),
        sse_event(
            "content_block_delta",
            json!({"type":"content_block_delta","index":3,"delta":{"type":"thinking_delta","thinking":"more"}})
        ),
        sse_event(
            "content_block_delta",
            json!({"type":"content_block_delta","index":3,"delta":{"type":"signature_delta","signature":"tail"}})
        ),
        sse_event(
            "content_block_stop",
            json!({"type":"content_block_stop","index":3})
        ),
        message_stop()
    );
    let mut out = c.push_bytes(stream.as_bytes());
    out.extend(c.finish());
    let events = collect_events(&out);
    for required in [
        "response.output_item.added",
        "response.reasoning_summary_part.added",
        "response.reasoning_summary_text.delta",
        "response.reasoning_summary_text.done",
        "response.reasoning_summary_part.done",
        "response.output_item.done",
        "response.completed",
    ] {
        assert!(
            event_names(&events).contains(&required),
            "missing {required}"
        );
    }
    let done = &events
        .iter()
        .find(|(n, _)| n == "response.output_item.done")
        .unwrap()
        .1["item"];
    assert_eq!(done["summary"][0]["text"], "initialmore");
    assert_eq!(done["signature"], "sig-tail");
    assert_eq!(completed_response(&events)["output"][0], *done);
}

#[test]
fn thinking_signature_may_arrive_only_in_delta() {
    let mut converter = AnthropicSseToResponsesConverter::with_request(&json!({}));
    let stream = format!(
        "{}{}{}{}{}",
        message_start("msg_signature_delta"),
        sse_event(
            "content_block_start",
            json!({"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"why"}}),
        ),
        sse_event(
            "content_block_delta",
            json!({"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"sig-from-delta"}}),
        ),
        sse_event(
            "content_block_stop",
            json!({"type":"content_block_stop","index":0}),
        ),
        message_stop(),
    );
    let mut out = converter.push_bytes(stream.as_bytes());
    out.extend(converter.finish());
    let events = collect_events(&out);
    let done = &events
        .iter()
        .find(|(name, _)| name == "response.output_item.done")
        .expect("missing response.output_item.done")
        .1["item"];
    assert_eq!(done["signature"], "sig-from-delta");
    assert_eq!(completed_response(&events)["output"][0], *done);

    let mut converter = AnthropicSseToResponsesConverter::with_request(&json!({}));
    let stream = format!(
        "{}{}{}{}",
        message_start("msg_missing_signature"),
        sse_event(
            "content_block_start",
            json!({"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"why"}}),
        ),
        sse_event(
            "content_block_stop",
            json!({"type":"content_block_stop","index":0}),
        ),
        message_stop(),
    );
    let mut out = converter.push_bytes(stream.as_bytes());
    out.extend(converter.finish());
    let events = collect_events(&out);
    let item = &completed_response(&events)["output"][0];
    assert!(item.get("signature").is_none());
    let replay = responses_to_anthropic_messages(&json!({"input":[item]})).unwrap();
    assert_eq!(
        replay["messages"][0]["content"][0],
        json!({"type":"thinking","thinking":"why"})
    );
}

#[test]
fn missing_message_start_usage_fails_immediately_and_stays_failed() {
    let mut converter = AnthropicSseToResponsesConverter::with_request(&json!({}));
    let missing_usage = sse_event(
        "message_start",
        json!({"type":"message_start","message":{"id":"msg_missing_usage","model":"k3"}}),
    );
    let out = converter.push_bytes(missing_usage.as_bytes());
    let events = collect_events(&out);
    assert_eq!(event_names(&events), vec!["response.failed"]);
    assert_eq!(
        events[0].1["response"]["error"]["code"],
        "invalid_sse_event"
    );

    let later = format!("{}{}", message_start("msg_later"), message_stop(),);
    assert!(converter.push_bytes(later.as_bytes()).is_empty());
    assert!(converter.finish().is_empty());
}

#[test]
fn stream_response_created_at_is_present_and_stable() {
    let mut converter = AnthropicSseToResponsesConverter::with_request(&json!({}));
    let stream = format!("{}{}", message_start("msg_created_at"), message_stop(),);
    let mut out = converter.push_bytes(stream.as_bytes());
    out.extend(converter.finish());
    let events = collect_events(&out);
    let timestamps: Vec<_> = events
        .iter()
        .filter(|(name, _)| {
            matches!(
                name.as_str(),
                "response.created" | "response.in_progress" | "response.completed"
            )
        })
        .map(|(_, data)| {
            data["response"]["created_at"]
                .as_u64()
                .expect("created_at must be a unix timestamp")
        })
        .collect();
    assert_eq!(timestamps.len(), 3);
    assert!(timestamps[0] > 0);
    assert!(
        timestamps
            .iter()
            .all(|timestamp| *timestamp == timestamps[0])
    );
}

#[test]
fn tool_use_start_input_is_fallback_when_no_delta_arrives() {
    let mut converter = AnthropicSseToResponsesConverter::with_request(&json!({}));
    let stream = format!(
        "{}{}{}{}",
        message_start("msg_initial_tool_input"),
        sse_event(
            "content_block_start",
            json!({"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"call_initial","name":"shell","input":{"seed":1}}}),
        ),
        sse_event(
            "content_block_stop",
            json!({"type":"content_block_stop","index":0}),
        ),
        message_stop(),
    );
    let mut out = converter.push_bytes(stream.as_bytes());
    out.extend(converter.finish());
    let events = collect_events(&out);
    let arguments_done = events
        .iter()
        .find(|(name, _)| name == "response.function_call_arguments.done")
        .expect("missing response.function_call_arguments.done");
    assert_eq!(arguments_done.1["arguments"], "{\"seed\":1}");
    assert_eq!(
        completed_response(&events)["output"][0]["arguments"],
        "{\"seed\":1}"
    );

    let mut converter = AnthropicSseToResponsesConverter::with_request(&json!({}));
    let stream = format!(
        "{}{}{}{}{}",
        message_start("msg_delta_overrides_initial"),
        sse_event(
            "content_block_start",
            json!({"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"call_delta","name":"shell","input":{"seed":1}}}),
        ),
        sse_event(
            "content_block_delta",
            json!({"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"from_delta\":true}"}}),
        ),
        sse_event(
            "content_block_stop",
            json!({"type":"content_block_stop","index":0}),
        ),
        message_stop(),
    );
    let mut out = converter.push_bytes(stream.as_bytes());
    out.extend(converter.finish());
    let events = collect_events(&out);
    assert_eq!(
        completed_response(&events)["output"][0]["arguments"],
        "{\"from_delta\":true}"
    );
}

#[test]
fn tool_use_delta_arguments_must_be_a_json_object() {
    for arguments in ["{\"command\":", "not json", "[]", "null"] {
        let mut converter = AnthropicSseToResponsesConverter::with_request(&json!({}));
        let stream = format!(
            "{}{}{}{}",
            message_start("msg_invalid_arguments"),
            sse_event(
                "content_block_start",
                json!({"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"call","name":"shell","input":{}}}),
            ),
            sse_event(
                "content_block_delta",
                json!({"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":arguments}}),
            ),
            sse_event(
                "content_block_stop",
                json!({"type":"content_block_stop","index":0}),
            ),
        );
        let mut out = converter.push_bytes(stream.as_bytes());
        out.extend(converter.finish());
        let events = collect_events(&out);
        let names = event_names(&events);
        assert_eq!(
            names
                .iter()
                .filter(|name| **name == "response.failed")
                .count(),
            1,
            "{arguments:?}"
        );
        for forbidden in [
            "response.function_call_arguments.done",
            "response.output_item.done",
            "response.completed",
        ] {
            assert!(!names.contains(&forbidden), "{arguments:?}: {names:?}");
        }
    }
}

#[test]
fn item_ids_unique_and_stable_nonstream_and_sse() {
    let expected_ids = [
        "msg_ids-reasoning-0",
        "msg_ids-message-1",
        "msg_ids-function_call-2",
    ];
    let body = json!({"id":"msg_ids","model":"k3","stop_reason":"tool_use","content":[{"type":"thinking","thinking":"why","signature":"sig"},{"type":"text","text":"hello"},{"type":"tool_use","id":"call","name":"shell","input":{}}]});
    let first = anthropic_message_to_response(&body, None).unwrap();
    let second = anthropic_message_to_response(&body, None).unwrap();
    let ids: Vec<_> = first["output"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["id"].as_str().unwrap())
        .collect();
    let ids2: Vec<_> = second["output"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ids2);
    assert_eq!(
        ids.iter()
            .copied()
            .collect::<std::collections::HashSet<_>>()
            .len(),
        ids.len()
    );
    assert_eq!(ids, expected_ids);
    fn convert() -> Vec<(String, serde_json::Value)> {
        let mut c = AnthropicSseToResponsesConverter::with_request(&json!({}));
        let stream = format!(
            "{}{}{}{}{}",
            message_start("msg_stable"),
            sse_event(
                "content_block_start",
                json!({"type":"content_block_start","index":7,"content_block":{"type":"text","text":""}})
            ),
            sse_event(
                "content_block_delta",
                json!({"type":"content_block_delta","index":7,"delta":{"type":"text_delta","text":"x"}})
            ),
            sse_event(
                "content_block_stop",
                json!({"type":"content_block_stop","index":7})
            ),
            message_stop()
        );
        let mut out = c.push_bytes(stream.as_bytes());
        out.extend(c.finish());
        collect_events(&out)
    }
    let events = convert();
    let added = events
        .iter()
        .find(|(n, _)| n == "response.output_item.added")
        .unwrap();
    let delta = events
        .iter()
        .find(|(n, _)| n == "response.output_text.delta")
        .unwrap();
    let done = events
        .iter()
        .find(|(n, _)| n == "response.output_item.done")
        .unwrap();
    let id = added.1["item"]["id"].as_str().unwrap();
    assert!(id.contains("msg_stable") && id.contains('7'));
    assert_eq!(delta.1["item_id"], id);
    assert_eq!(done.1["item"]["id"], id);
    assert_eq!(completed_response(&events)["output"][0]["id"], id);
    assert_eq!(completed_response(&convert())["output"][0]["id"], id);
    assert_eq!(id, "msg_stable-message-7");
}

#[test]
fn empty_input_and_tool_descriptions_are_safe() {
    assert!(responses_to_anthropic_messages(&json!({"input":[]})).is_err());
    let converted=responses_to_anthropic_messages(&json!({"input":"tool descriptions","tools":[{"type":"function","name":"missing","parameters":{}},{"type":"function","name":"null","description":null,"parameters":{}},{"type":"function","name":"verbatim","description":"  keep spacing  ","parameters":{}}]})).unwrap();
    assert!(converted["tools"][0].get("description").is_none());
    assert!(converted["tools"][1].get("description").is_none());
    assert_eq!(converted["tools"][2]["description"], "  keep spacing  ");
}

#[test]
fn empty_model_lists_preserve_or_normalize_by_shape() {
    let anthropic = json!({"data":[],"has_more":false,"first_id":null,"last_id":null});
    assert_eq!(
        anthropic_models_to_openai_models(&anthropic),
        json!({"object":"list","data":[]})
    );
    let openai = json!({"object":"list","data":[]});
    assert_eq!(anthropic_models_to_openai_models(&openai), openai);
}

#[test]
fn model_created_at_strict_rfc3339_calendar_validation() {
    let valid = [
        "2024-02-29T00:00:00Z",
        "2025-12-31T23:59:59Z",
        "2024-02-29T23:59:59+23:59",
        "2024-02-29T00:00:00.123Z",
        "2024-02-29T00:00:00.123456-08:30",
    ];
    let invalid = [
        "2025-02-29T00:00:00Z",
        "2024-02-30T00:00:00Z",
        "2024-04-31T00:00:00Z",
        "2024-13-01T00:00:00Z",
        "2024-01-01T24:00:00Z",
        "2024-01-01T00:60:00Z",
        "2024-01-01T00:00:60Z",
        "2024-01-01T00:00:00+24:00",
        "2024-01-01T00:00:00+01:60",
    ];
    let mut models = Vec::new();
    for (index, date) in valid.iter().chain(invalid.iter()).enumerate() {
        models.push(json!({"type":"model","id":format!("m{index}"),"created_at":date}));
    }
    let converted = anthropic_models_to_openai_models(&json!({"data":models,"has_more":false}));
    let data = converted["data"].as_array().unwrap();
    for (index, date) in valid.iter().enumerate() {
        assert!(
            data[index]["created"].as_i64().unwrap() > 0,
            "valid date rejected: {date}"
        );
    }
    for (offset, date) in invalid.iter().enumerate() {
        assert_eq!(
            data[valid.len() + offset]["created"],
            0,
            "invalid date accepted: {date}"
        );
    }
    let equal = anthropic_models_to_openai_models(
        &json!({"data":[{"type":"model","id":"z","created_at":"2024-02-29T00:00:00Z"},{"type":"model","id":"o","created_at":"2024-02-29T08:00:00+08:00"}]}),
    );
    assert_eq!(equal["data"][0]["created"], equal["data"][1]["created"]);
}

#[test]
fn responses_string_input_and_validation() {
    let converted = responses_to_anthropic_messages(&json!({"input":"hello"})).unwrap();
    assert_eq!(
        converted["messages"],
        json!([{"role":"user","content":[{"type":"text","text":"hello"}]}])
    );
    for body in [
        json!({"input":""}),
        json!({"input":"   "}),
        json!({"input":null}),
        json!({"input":{}}),
        json!({"input":7}),
    ] {
        let error = responses_to_anthropic_messages(&body)
            .unwrap_err()
            .to_string();
        assert!(error.contains("input"), "{error}");
    }
    assert!(responses_to_anthropic_messages(&json!({"input":[]})).is_err());
}

#[test]
fn thinking_envelope_roundtrips_and_does_not_misread_opaque_ciphertext() {
    let anthropic = json!({"id":"msg_envelope","model":"k3","stop_reason":"end_turn","usage":{},"content":[
        {"type":"thinking","thinking":"private chain","signature":"sig-secret"},
        {"type":"redacted_thinking","data":"redacted-secret"}
    ]});
    let response = anthropic_message_to_response(&anthropic, None).unwrap();
    let output = response["output"].as_array().unwrap();
    assert_eq!(output.len(), 2);
    for item in output {
        assert!(
            item["encrypted_content"]
                .as_str()
                .unwrap()
                .starts_with("codexplusplus-anthropic-v1:")
        );
    }
    let replay = responses_to_anthropic_messages(&json!({"input":output})).unwrap();
    assert_eq!(replay["messages"][0]["content"], anthropic["content"]);

    let opaque = responses_to_anthropic_messages(&json!({"input":[{"type":"reasoning","encrypted_content":"ordinary-provider-ciphertext","summary":[{"text":"do not replay"}]}]})).unwrap_err();
    assert!(opaque.to_string().contains("input"));
    let prefixed_invalid = [
        "codexplusplus-anthropic-v1:not-hex",
        "codexplusplus-anthropic-v1:7b7d",
        "codexplusplus-anthropic-v1:7b2274797065223a2274657874222c2274657874223a226e6f74207468696e6b696e67227d",
        "codexplusplus-anthropic-v1:7b2274797065223a227468696e6b696e67222c227468696e6b696e67223a317d",
        "codexplusplus-anthropic-v1:7b2274797065223a2272656461637465645f7468696e6b696e67222c2264617461223a317d",
    ];
    for encrypted_content in prefixed_invalid {
        let result = responses_to_anthropic_messages(&json!({"input":[{
            "type":"reasoning",
            "encrypted_content":encrypted_content,
            "signature":"must-not-fallback",
            "summary":[{"text":"must not replay"}]
        }]}));
        assert!(
            result.is_err(),
            "accepted invalid envelope: {encrypted_content}"
        );
    }
    let legacy = responses_to_anthropic_messages(&json!({"input":[{"type":"reasoning","signature":"legacy","summary":[{"text":"legacy thought"}]}]})).unwrap();
    assert_eq!(
        legacy["messages"][0]["content"][0],
        json!({"type":"thinking","thinking":"legacy thought","signature":"legacy"})
    );
}

#[test]
fn streams_redacted_thinking_without_exposing_sensitive_data_and_replays_it() {
    let secret = "opaque-redacted-secret";
    let stream = format!(
        "{}{}{}{}{}",
        message_start("redacted"),
        sse_event(
            "content_block_start",
            json!({"type":"content_block_start","index":0,"content_block":{"type":"redacted_thinking","data":secret}}),
        ),
        sse_event(
            "content_block_stop",
            json!({"type":"content_block_stop","index":0}),
        ),
        sse_event(
            "message_delta",
            json!({"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":1}}),
        ),
        raw_message_stop(),
    );
    let mut converter = AnthropicSseToResponsesConverter::with_request(&json!({}));
    let events = collect_events(&converter.push_bytes(stream.as_bytes()));
    let names = event_names(&events);
    assert!(!names.contains(&"response.reasoning_summary_part.added"));
    assert!(!names.contains(&"response.reasoning_summary_text.delta"));
    assert!(!names.contains(&"response.reasoning_summary_text.done"));
    assert!(!names.contains(&"response.reasoning_summary_part.done"));

    let response = completed_response(&events);
    assert_eq!(response["output"][0]["summary"], json!([]));
    let encrypted = response["output"][0]["encrypted_content"].as_str().unwrap();
    assert!(encrypted.starts_with("codexplusplus-anthropic-v1:"));
    assert!(!serde_json::to_string(&events).unwrap().contains(secret));

    let replay = responses_to_anthropic_messages(&json!({"input":response["output"]})).unwrap();
    assert_eq!(
        replay["messages"][0]["content"],
        json!([{"type":"redacted_thinking","data":secret}])
    );
}

#[test]
fn redacted_thinking_start_requires_string_data() {
    let stream = format!(
        "{}{}",
        message_start("redacted-invalid"),
        sse_event(
            "content_block_start",
            json!({"type":"content_block_start","index":0,"content_block":{"type":"redacted_thinking","data":{"secret":true}}}),
        ),
    );
    assert_invalid_sse_event(&stream, Some("secret"));
}

#[test]
fn tool_choice_and_thinking_budget_boundaries() {
    for (choice, expected) in [
        (json!("auto"), json!({"type":"auto"})),
        (json!("required"), json!({"type":"any"})),
        (
            json!({"type":"function","name":"lookup"}),
            json!({"type":"tool","name":"lookup"}),
        ),
    ] {
        let out = responses_to_anthropic_messages(&json!({"input":"hi","tools":[{"type":"function","name":"lookup","parameters":{}}],"tool_choice":choice})).unwrap();
        assert_eq!(out["tool_choice"], expected);
    }
    let none = responses_to_anthropic_messages(&json!({"input":"hi","tools":[{"type":"function","name":"lookup","parameters":{}}],"tool_choice":"none"})).unwrap();
    assert!(none.get("tools").is_none());
    assert!(none.get("tool_choice").is_none());

    for (max, effort, expected) in [
        (1024, "high", None),
        (1025, "high", Some(1024)),
        (2048, "low", Some(1024)),
        (8192, "medium", Some(7168)),
        (32000, "high", Some(16384)),
        (32000, "xhigh", Some(30976)),
    ] {
        let out = responses_to_anthropic_messages(
            &json!({"input":"hi","max_output_tokens":max,"reasoning":{"effort":effort}}),
        )
        .unwrap();
        assert_eq!(
            out.get("thinking")
                .and_then(|v| v["budget_tokens"].as_u64()),
            expected,
            "max={max} effort={effort}"
        );
        assert_eq!(out["max_tokens"], max);
    }
}

#[test]
fn explicit_tool_choice_disables_thinking_but_auto_can_coexist() {
    let tools = json!([{"type":"function","name":"lookup","parameters":{}}]);
    for (choice, expected) in [
        (json!("required"), json!({"type":"any"})),
        (
            json!({"type":"function","name":"lookup"}),
            json!({"type":"tool","name":"lookup"}),
        ),
    ] {
        let out = responses_to_anthropic_messages(&json!({
            "input":"hi",
            "tools":tools,
            "tool_choice":choice,
            "reasoning":{"effort":"high"}
        }))
        .unwrap();
        assert_eq!(out["tool_choice"], expected);
        assert!(out.get("thinking").is_none());
    }

    let auto = responses_to_anthropic_messages(&json!({
        "input":"hi",
        "tools":tools,
        "tool_choice":"auto",
        "reasoning":{"effort":"high"}
    }))
    .unwrap();
    assert_eq!(auto["tool_choice"], json!({"type":"auto"}));
    assert_eq!(
        auto["thinking"],
        json!({"type":"enabled","budget_tokens":16384})
    );
}

#[test]
fn function_output_content_items_become_nested_tool_result_blocks() {
    let out = responses_to_anthropic_messages(
        &json!({"input":[{"type":"function_call_output","call_id":"c1","output":[
            {"type":"input_text","text":"in"},{"type":"output_text","text":"out"},
            {"type":"input_image","image_url":"data:image/png;base64,AAAA"},
            {"type":"other","value":{"b":2,"a":1}}, 7
        ]}]}),
    )
    .unwrap();
    assert_eq!(
        out["messages"][0]["content"][0]["content"],
        json!([
            {"type":"text","text":"in"},{"type":"text","text":"out"},
            {"type":"image","source":{"type":"base64","media_type":"image/png","data":"AAAA"}},
            {"type":"text","text":"{\"type\":\"other\",\"value\":{\"a\":1,\"b\":2}}"},
            {"type":"text","text":"7"}
        ])
    );
}

#[test]
fn function_output_unknown_only_array_converts_each_item_and_empty_array_stays_empty() {
    let unknown = responses_to_anthropic_messages(&json!({
        "input":[{"type":"function_call_output","call_id":"c1","output":[
            {"z":1,"a":2}, 7, true
        ]}]
    }))
    .unwrap();
    assert_eq!(
        unknown["messages"][0]["content"][0]["content"],
        json!([
            {"type":"text","text":"{\"a\":2,\"z\":1}"},
            {"type":"text","text":"7"},
            {"type":"text","text":"true"}
        ])
    );

    let empty = responses_to_anthropic_messages(&json!({
        "input":[{"type":"function_call_output","call_id":"c2","output":[]}]
    }))
    .unwrap();
    assert_eq!(empty["messages"][0]["content"][0]["content"], json!([]));
}

#[test]
fn usage_rejects_invalid_values_and_saturates_extremes() {
    for usage in [
        json!({"input_tokens":-1}),
        json!({"output_tokens":1.5}),
        json!({"cache_read_input_tokens":"1"}),
    ] {
        assert!(anthropic_message_to_response(&json!({"content":[],"usage":usage}), None).is_err());
    }
    let max = u64::MAX;
    let out = anthropic_message_to_response(&json!({"content":[],"usage":{"input_tokens":max,"output_tokens":max,"cache_read_input_tokens":max,"cache_creation_input_tokens":max}}), None).unwrap();
    assert_eq!(out["usage"]["input_tokens"], max);
    assert_eq!(out["usage"]["total_tokens"], max);
}

#[test]
fn sse_rejects_starting_a_second_content_block_while_one_is_open() {
    let stream = format!(
        "{}{}{}",
        message_start("overlap"),
        sse_event(
            "content_block_start",
            json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}),
        ),
        sse_event(
            "content_block_start",
            json!({"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}}),
        ),
    );
    assert_invalid_sse_event(&stream, None);
}

#[test]
fn sse_enforces_message_phase_and_saturating_usage() {
    let block_start = sse_event(
        "content_block_start",
        json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}),
    );
    let block_stop = sse_event(
        "content_block_stop",
        json!({"type":"content_block_stop","index":0}),
    );
    for stream in [
        format!("{}{}", message_start("a"), message_start("b")),
        format!(
            "{}{}{}",
            message_start("a"),
            block_start.clone(),
            sse_event(
                "message_delta",
                json!({"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":1}})
            )
        ),
        format!(
            "{}{}{}{}{}",
            message_start("a"),
            block_start.clone(),
            block_stop.clone(),
            sse_event(
                "message_delta",
                json!({"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":1}})
            ),
            block_start.clone()
        ),
    ] {
        assert_invalid_sse_event(&stream, None);
    }
    assert_invalid_sse_event(
        &sse_event("message_stop", json!({"type":"message_stop"})),
        None,
    );

    for (stop_reason, expected_event, expected_tokens) in [
        ("end_turn", "response.completed", 3_u64),
        ("max_tokens", "response.incomplete", 5_u64),
    ] {
        let mut converter = AnthropicSseToResponsesConverter::with_request(&json!({}));
        let stream = format!(
            "{}{}{}{}",
            message_start("multi_delta"),
            sse_event(
                "message_delta",
                json!({"type":"message_delta","delta":{"stop_sequence":"END"},"usage":{"output_tokens":1}}),
            ),
            sse_event(
                "message_delta",
                json!({"type":"message_delta","delta":{"stop_reason":stop_reason},"usage":{"output_tokens":expected_tokens}}),
            ),
            raw_message_stop(),
        );
        let events = collect_events(&converter.push_bytes(stream.as_bytes()));
        let response = &events
            .iter()
            .find(|(name, _)| name == expected_event)
            .unwrap()
            .1["response"];
        assert_eq!(response["usage"]["output_tokens"], expected_tokens);
    }

    let mut converter = AnthropicSseToResponsesConverter::with_request(&json!({}));
    let stream = format!(
        "{}{}{}",
        sse_event(
            "message_start",
            json!({"type":"message_start","message":{"id":"max","model":"k3","usage":{"input_tokens":u64::MAX,"cache_read_input_tokens":u64::MAX}}})
        ),
        sse_event(
            "message_delta",
            json!({"type":"message_delta","delta":{"stop_reason":"max_tokens"},"usage":{"output_tokens":u64::MAX}})
        ),
        sse_event("message_stop", json!({"type":"message_stop"}))
    );
    let events = collect_events(&converter.push_bytes(stream.as_bytes()));
    let response = &events
        .iter()
        .find(|(name, _)| name == "response.incomplete")
        .unwrap()
        .1["response"];
    assert_eq!(response["usage"]["total_tokens"], u64::MAX);
}

#[test]
fn sse_nullable_message_delta_fields_preserve_existing_usage() {
    let mut converter = AnthropicSseToResponsesConverter::with_request(&json!({}));
    let stream = format!(
        "{}{}{}{}",
        sse_event(
            "message_start",
            json!({
                "type":"message_start",
                "message":{
                    "id":"nullable_delta",
                    "model":"k3",
                    "usage":{
                        "input_tokens":11,
                        "cache_creation_input_tokens":12,
                        "cache_read_input_tokens":13,
                        "output_tokens_details":{"thinking_tokens":14},
                        "server_tool_use":{"web_fetch_requests":15,"web_search_requests":16}
                    }
                }
            }),
        ),
        sse_event(
            "message_delta",
            json!({
                "type":"message_delta",
                "delta":{"stop_reason":null},
                "usage":{
                    "input_tokens":null,
                    "cache_creation_input_tokens":null,
                    "cache_read_input_tokens":null,
                    "output_tokens":1,
                    "output_tokens_details":null,
                    "server_tool_use":null
                }
            }),
        ),
        sse_event(
            "message_delta",
            json!({
                "type":"message_delta",
                "delta":{"stop_reason":"end_turn"},
                "usage":{"output_tokens":2}
            }),
        ),
        raw_message_stop(),
    );
    let events = collect_events(&converter.push_bytes(stream.as_bytes()));
    let response = &events
        .iter()
        .find(|(name, _)| name == "response.completed")
        .unwrap()
        .1["response"];
    assert_eq!(response["usage"]["input_tokens"], 36);
    assert_eq!(response["usage"]["output_tokens"], 2);
    assert_eq!(response["usage"]["total_tokens"], 38);
}

#[test]
fn sse_message_stop_requires_message_delta_and_invalid_final_usage_fails() {
    let start_then_stop = format!("{}{}", message_start("missing_delta"), raw_message_stop(),);
    assert_invalid_sse_event(&start_then_stop, None);
    let null_stop_then_stop = format!(
        "{}{}{}",
        message_start("null_stop"),
        sse_event(
            "message_delta",
            json!({"type":"message_delta","delta":{"stop_reason":null},"usage":{"output_tokens":1}}),
        ),
        raw_message_stop(),
    );
    assert_invalid_sse_event(&null_stop_then_stop, None);

    let mut converter = AnthropicSseToResponsesConverter::with_request(&json!({}));
    let stream = format!(
        "{}{}{}",
        sse_event(
            "message_start",
            json!({"type":"message_start","message":{"id":"bad_usage","model":"k3","usage":{"input_tokens":"invalid"}}}),
        ),
        sse_event(
            "message_delta",
            json!({"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":1}}),
        ),
        raw_message_stop(),
    );
    let events = collect_events(&converter.push_bytes(stream.as_bytes()));
    assert_eq!(event_names(&events).last(), Some(&"response.failed"));
    assert!(!event_names(&events).contains(&"response.completed"));
}
