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
    let body = json!({"model":"k3","input":[],"reasoning":{"effort":"high"}});
    let out = responses_to_anthropic_messages(&body).unwrap();
    assert_eq!(
        out["thinking"],
        json!({"type":"enabled","budget_tokens":16384})
    );

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
    UpstreamWireApi, finalize_non_streaming_responses_response,
    open_responses_proxy_request_with_settings,
};
use codex_plus_core::settings::{
    AggregateRelayMember, AggregateRelayProfile, AggregateRelayStrategy, RelayMode,
};
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
