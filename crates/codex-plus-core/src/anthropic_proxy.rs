//! Anthropic Messages API 转换层。
//! codex Responses API ↔ Anthropic Messages API 的请求/响应/SSE 双向转换。
//! 硬约束：工具定义、调用参数、工具结果、system prompt 语义零改写（见设计文档第 3 节）。

use serde_json::{Value, json};

pub const ANTHROPIC_VERSION: &str = "2023-06-01";
pub const DEFAULT_ANTHROPIC_MAX_TOKENS: u64 = 32000;
const THINKING_ENVELOPE_PREFIX: &str = "codexplusplus-anthropic-v1:";

fn encode_thinking_envelope(block: &Value) -> String {
    let bytes = serde_json::to_vec(block).unwrap_or_default();
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    format!("{THINKING_ENVELOPE_PREFIX}{hex}")
}

fn valid_thinking_envelope_block(block: Value) -> Option<Value> {
    let object = block.as_object()?;
    match object.get("type").and_then(Value::as_str)? {
        "thinking"
            if object.get("thinking").is_some_and(Value::is_string)
                && object.get("signature").is_none_or(Value::is_string) =>
        {
            Some(block)
        }
        "redacted_thinking" if object.get("data").is_some_and(Value::is_string) => Some(block),
        _ => None,
    }
}

fn decode_thinking_envelope(value: &str) -> Option<Value> {
    let hex = value.strip_prefix(THINKING_ENVELOPE_PREFIX)?;
    if hex.len() % 2 != 0 {
        return None;
    }
    let bytes: Option<Vec<u8>> = (0..hex.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&hex[index..index + 2], 16).ok())
        .collect();
    valid_thinking_envelope_block(serde_json::from_slice(&bytes?).ok()?)
}

/// Responses 请求体 → Anthropic Messages 请求体。
pub fn responses_to_anthropic_messages(body: &Value) -> anyhow::Result<Value> {
    let mut result = json!({});
    if let Some(model) = body.get("model") {
        result["model"] = model.clone();
    }

    // ── system：instructions + system/developer 消息 ──
    let mut system_blocks: Vec<Value> = Vec::new();
    if let Some(instructions) = body.get("instructions") {
        let text = crate::protocol_proxy::instruction_text(instructions);
        if !text.is_empty() {
            system_blocks.push(json!({"type": "text", "text": text}));
        }
    }

    // ── input items → messages ──
    let mut messages: Vec<Value> = Vec::new();
    let mut pending_assistant: Vec<Value> = Vec::new();
    let mut pending_user: Vec<Value> = Vec::new();

    fn flush_assistant(messages: &mut Vec<Value>, pending: &mut Vec<Value>) {
        if !pending.is_empty() {
            messages.push(json!({"role": "assistant", "content": std::mem::take(pending)}));
        }
    }
    fn flush_user(messages: &mut Vec<Value>, pending: &mut Vec<Value>) {
        if !pending.is_empty() {
            messages.push(json!({"role": "user", "content": std::mem::take(pending)}));
        }
    }

    let input = body
        .get("input")
        .ok_or_else(|| anyhow::anyhow!("Responses input is required"))?;
    let owned_items;
    let items = match input {
        Value::String(text) if !text.trim().is_empty() => {
            pending_user.push(json!({"type":"text","text":text}));
            &[][..]
        }
        Value::String(_) => anyhow::bail!("Responses input prompt must not be empty"),
        Value::Array(items) if !items.is_empty() => items.as_slice(),
        Value::Array(_) => anyhow::bail!("Responses input prompt must not be empty"),
        _ => anyhow::bail!("Responses input must be a string or an array"),
    };
    owned_items = items;
    for item in owned_items {
        match item.get("type").and_then(Value::as_str) {
            Some("message") => {
                let role = item.get("role").and_then(Value::as_str).unwrap_or("user");
                let parts = item
                    .get("content")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                match role {
                    "system" | "developer" => {
                        for part in &parts {
                            if let Some(text) = part.get("text").and_then(Value::as_str) {
                                system_blocks.push(json!({"type":"text","text":text}));
                            }
                        }
                    }
                    "assistant" => {
                        flush_user(&mut messages, &mut pending_user);
                        for part in &parts {
                            if let Some(block) = responses_part_to_anthropic_text(part) {
                                pending_assistant.push(block);
                            }
                        }
                    }
                    _ => {
                        flush_assistant(&mut messages, &mut pending_assistant);
                        for part in &parts {
                            if let Some(block) = responses_part_to_anthropic_content(part) {
                                pending_user.push(block);
                            }
                        }
                    }
                }
            }
            Some("function_call") => {
                flush_user(&mut messages, &mut pending_user);
                let arguments = item
                    .get("arguments")
                    .and_then(Value::as_str)
                    .unwrap_or("{}");
                let input = serde_json::from_str::<Value>(arguments).unwrap_or_else(|_| json!({}));
                pending_assistant.push(json!({
                    "type": "tool_use",
                    "id": item.get("call_id").and_then(Value::as_str).unwrap_or(""),
                    "name": item.get("name").and_then(Value::as_str).unwrap_or(""),
                    "input": input,
                }));
            }
            Some("function_call_output") => {
                flush_assistant(&mut messages, &mut pending_assistant);
                let content = match item.get("output") {
                    Some(Value::Array(parts)) => Value::Array(
                        parts
                            .iter()
                            .map(|part| {
                                responses_part_to_anthropic_content(part).unwrap_or_else(
                                    || json!({"type":"text","text":stable_json_string(part)}),
                                )
                            })
                            .collect(),
                    ),
                    Some(Value::String(value)) => Value::String(value.clone()),
                    None | Some(Value::Null) => Value::String(String::new()),
                    Some(value) => Value::String(stable_json_string(value)),
                };
                pending_user.push(json!({
                    "type": "tool_result",
                    "tool_use_id": item.get("call_id").and_then(Value::as_str).unwrap_or(""),
                    "content": content,
                }));
            }
            Some("reasoning") => {
                let encrypted_content = item.get("encrypted_content").and_then(Value::as_str);
                if let Some(block) = encrypted_content.and_then(decode_thinking_envelope) {
                    flush_user(&mut messages, &mut pending_user);
                    pending_assistant.push(block);
                } else if encrypted_content.is_none() {
                    if let Some(signature) = item.get("signature").and_then(Value::as_str) {
                        flush_user(&mut messages, &mut pending_user);
                        pending_assistant.push(json!({
                            "type": "thinking",
                            "thinking": reasoning_item_text(item),
                            "signature": signature,
                        }));
                    }
                }
            }
            _ => { /* 未知 item 类型：跳过，不中断请求 */ }
        }
    }
    flush_assistant(&mut messages, &mut pending_assistant);
    flush_user(&mut messages, &mut pending_user);
    if messages.is_empty() {
        anyhow::bail!("Responses input did not contain a usable prompt");
    }

    if !system_blocks.is_empty() {
        if let Some(last) = system_blocks.last_mut() {
            last["cache_control"] = json!({"type": "ephemeral"});
        }
        result["system"] = json!(system_blocks);
    }
    result["messages"] = json!(messages);

    // ── tools 1:1（schema 原文 clone）──
    let tool_choice = body.get("tool_choice");
    let tools_disabled = tool_choice.and_then(Value::as_str) == Some("none");
    if !tools_disabled {
        if let Some(tools) = body.get("tools").and_then(Value::as_array) {
            let mut converted: Vec<Value> = tools
                .iter()
                .filter(|tool| tool.get("type").and_then(Value::as_str) == Some("function"))
                .map(|tool| {
                    let mut converted = json!({
                        "name": tool.get("name").and_then(Value::as_str).unwrap_or(""),
                        "input_schema": tool.get("parameters").cloned().unwrap_or(json!({"type":"object"})),
                    });
                    if let Some(description) = tool.get("description").and_then(Value::as_str) {
                        converted["description"] = json!(description);
                    }
                    converted
                })
                .collect();
            if !converted.is_empty() {
                if let Some(last) = converted.last_mut() {
                    last["cache_control"] = json!({"type": "ephemeral"});
                }
                result["tools"] = json!(converted);
                match tool_choice {
                    Some(Value::String(value)) if value == "auto" => {
                        result["tool_choice"] = json!({"type":"auto"})
                    }
                    Some(Value::String(value)) if value == "required" => {
                        result["tool_choice"] = json!({"type":"any"})
                    }
                    Some(Value::Object(choice))
                        if choice.get("type").and_then(Value::as_str) == Some("function") =>
                    {
                        if let Some(name) = choice.get("name").and_then(Value::as_str) {
                            result["tool_choice"] = json!({"type":"tool","name":name});
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    // ── 采样与输出参数 ──
    let max_tokens = body
        .get("max_output_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_ANTHROPIC_MAX_TOKENS);
    result["max_tokens"] = json!(max_tokens);
    for key in ["temperature", "top_p", "stream"] {
        if let Some(value) = body.get(key) {
            result[key] = value.clone();
        }
    }

    // ── reasoning effort → thinking 预算 ──
    let explicit_tool_choice = matches!(
        result
            .get("tool_choice")
            .and_then(|choice| choice.get("type"))
            .and_then(Value::as_str),
        Some("any" | "tool")
    );
    if !explicit_tool_choice {
        if let Some(effort) = body
            .get("reasoning")
            .and_then(|r| r.get("effort"))
            .and_then(Value::as_str)
        {
            let requested = match effort {
                "none" | "minimal" => None,
                "low" => Some(2048_u64),
                "high" => Some(16384),
                "xhigh" => Some(32768),
                _ => Some(8192), // medium 及未知值
            };
            if max_tokens > 1024 {
                if let Some(requested) = requested {
                    let budget = requested.min(max_tokens.saturating_sub(1024)).max(1024);
                    result["thinking"] = json!({"type": "enabled", "budget_tokens": budget});
                }
            }
        }
    }

    Ok(result)
}

/// assistant 文本 part（output_text）。
fn responses_part_to_anthropic_text(part: &Value) -> Option<Value> {
    match part.get("type").and_then(Value::as_str) {
        Some("output_text") | Some("input_text") => part
            .get("text")
            .and_then(Value::as_str)
            .map(|text| json!({"type":"text","text":text})),
        _ => None,
    }
}

/// user 内容 part（文本 + 图片）。
fn responses_part_to_anthropic_content(part: &Value) -> Option<Value> {
    match part.get("type").and_then(Value::as_str) {
        Some("input_text") | Some("output_text") => part
            .get("text")
            .and_then(Value::as_str)
            .map(|text| json!({"type":"text","text":text})),
        Some("input_image") => input_image_to_anthropic(part),
        _ => None,
    }
}

/// input_image → Anthropic image block。
fn input_image_to_anthropic(part: &Value) -> Option<Value> {
    let url = match part.get("image_url") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Object(map)) => map.get("url")?.as_str()?.to_string(),
        _ => return None,
    };
    if let Some(data) = url.strip_prefix("data:") {
        let (meta, payload) = data.split_once(';')?;
        let base64 = payload.strip_prefix("base64,")?;
        return Some(json!({
            "type": "image",
            "source": {"type": "base64", "media_type": meta, "data": base64}
        }));
    }
    if url.starts_with("http://") || url.starts_with("https://") {
        return Some(json!({
            "type": "image",
            "source": {"type": "url", "url": url}
        }));
    }
    None
}

/// 将 JSON 值序列化为键顺序稳定的紧凑字符串。
fn stable_json_string(value: &Value) -> String {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<_> = map.keys().collect();
            keys.sort_unstable();
            let fields: Vec<String> = keys
                .into_iter()
                .map(|key| {
                    format!(
                        "{}:{}",
                        serde_json::to_string(key).unwrap_or_else(|_| "\"\"".to_string()),
                        stable_json_string(&map[key])
                    )
                })
                .collect();
            format!("{{{}}}", fields.join(","))
        }
        Value::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(stable_json_string)
                .collect::<Vec<_>>()
                .join(",")
        ),
        _ => serde_json::to_string(value).unwrap_or_default(),
    }
}

/// 提取 reasoning item 的文本：summary 全部文本优先，否则使用 content 全部文本。
fn reasoning_item_text(item: &Value) -> String {
    for key in ["summary", "content"] {
        if let Some(parts) = item.get(key).and_then(Value::as_array) {
            let texts: Vec<_> = parts
                .iter()
                .filter_map(|part| part.get("text").and_then(Value::as_str))
                .collect();
            if !texts.is_empty() {
                return texts.join("\n");
            }
        }
    }
    String::new()
}

const RESPONSE_REQUEST_FIELDS: [&str; 11] = [
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

fn copy_response_request_fields(response: &mut Value, request: &Value) {
    for key in RESPONSE_REQUEST_FIELDS {
        if let Some(value) = request.get(key) {
            response[key] = value.clone();
        }
    }
}

/// Anthropic Messages 响应体 → codex Responses 响应体（非流式）。
pub fn anthropic_message_to_response(
    body: &Value,
    original_request: Option<&Value>,
) -> anyhow::Result<Value> {
    let response_id = body
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or("msg_unknown")
        .to_string();
    let mut output: Vec<Value> = Vec::new();

    if let Some(blocks) = body.get("content").and_then(Value::as_array) {
        for (content_index, block) in blocks.iter().enumerate() {
            let kind = match block.get("type").and_then(Value::as_str) {
                Some("thinking") | Some("redacted_thinking") => "reasoning",
                Some("text") => "message",
                Some("tool_use") => "function_call",
                _ => continue,
            };
            let item_id = format!("{response_id}-{kind}-{content_index}");
            match block.get("type").and_then(Value::as_str) {
                Some("thinking") => {
                    let mut item = json!({
                        "type": "reasoning",
                        "id": item_id,
                        "summary": [{
                            "type": "summary_text",
                            "text": block.get("thinking").and_then(Value::as_str).unwrap_or("")
                        }],
                        "encrypted_content": encode_thinking_envelope(block)
                    });
                    if let Some(signature) = block.get("signature").and_then(Value::as_str) {
                        item["signature"] = json!(signature);
                    }
                    output.push(item);
                }
                Some("redacted_thinking") => {
                    output.push(json!({
                        "type":"reasoning",
                        "id":item_id,
                        "summary":[],
                        "encrypted_content":encode_thinking_envelope(block)
                    }));
                }
                Some("text") => {
                    output.push(json!({
                        "type": "message",
                        "id": item_id,
                        "role": "assistant",
                        "status": "completed",
                        "content": [{
                            "type": "output_text",
                            "text": block.get("text").and_then(Value::as_str).unwrap_or(""),
                            "annotations": []
                        }]
                    }));
                }
                Some("tool_use") => {
                    let input = block.get("input").cloned().unwrap_or(json!({}));
                    output.push(json!({
                        "type": "function_call",
                        "id": item_id,
                        "call_id": block.get("id").and_then(Value::as_str).unwrap_or(""),
                        "name": block.get("name").and_then(Value::as_str).unwrap_or(""),
                        "arguments": stable_json_string(&input),
                        "status": "completed"
                    }));
                }
                _ => {}
            }
        }
    }

    let stop_reason = body
        .get("stop_reason")
        .and_then(Value::as_str)
        .unwrap_or("");
    let mut response = json!({
        "id": response_id,
        "object": "response",
        "created_at": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
        "status": if stop_reason == "max_tokens" { "incomplete" } else { "completed" },
        "model": body.get("model").and_then(Value::as_str).unwrap_or(""),
        "output": output,
        "usage": anthropic_usage_to_responses_usage(body.get("usage"))?,
    });
    if stop_reason == "max_tokens" {
        response["incomplete_details"] = json!({"reason": "max_output_tokens"});
    }
    if let Some(request) = original_request {
        copy_response_request_fields(&mut response, request);
    }
    Ok(response)
}

/// Anthropic usage → Responses usage（缓存 token 计入总输入）。
pub(crate) fn anthropic_usage_to_responses_usage(usage: Option<&Value>) -> anyhow::Result<Value> {
    let Some(usage) = usage else {
        return Ok(json!({"input_tokens": 0, "output_tokens": 0, "total_tokens": 0}));
    };
    let object = usage
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("usage must be an object"))?;
    let token = |name: &str| -> anyhow::Result<u64> {
        match object.get(name) {
            None => Ok(0),
            Some(value) => value
                .as_u64()
                .ok_or_else(|| anyhow::anyhow!("usage.{name} must be a non-negative integer")),
        }
    };
    let input = token("input_tokens")?;
    let output = token("output_tokens")?;
    let cached = token("cache_read_input_tokens")?;
    let cache_creation = token("cache_creation_input_tokens")?;
    // Token accounting is externally supplied; saturating arithmetic keeps malformed
    // extreme values from wrapping into deceptively small context usage.
    let total_input = input.saturating_add(cached).saturating_add(cache_creation);
    Ok(json!({
        "input_tokens": total_input,
        "output_tokens": output,
        "total_tokens": total_input.saturating_add(output),
        "input_tokens_details": {"cached_tokens": cached},
        "output_tokens_details": {"reasoning_tokens": 0}
    }))
}

// ── SSE 流式转换 ──

/// 打开的 content block 状态。
#[derive(Default)]
struct OpenBlock {
    kind: String, // "text" | "tool_use" | "thinking" | "redacted_thinking"
    item_id: String,
    output_index: u64,
    content_index: u64,
    call_id: String,
    name: String,
    text_buffer: String,
    args_buffer: String,
    initial_tool_input: String,
    received_args_delta: bool,
    signature: Option<String>,
}

/// SSE 流的唯一终态。
#[derive(Clone, Copy, PartialEq, Eq)]
enum TerminalState {
    Open,
    Completed,
    Incomplete,
    Failed,
}

/// Anthropic SSE 流 → Responses SSE 流的增量转换器。
///
/// 与 `ChatSseToResponsesConverter` 结构对齐：push_bytes 增量喂入并返回
/// 转换后的字节，finish 在流正常结束时收尾，fail 在流异常时输出
/// `response.failed`。
pub struct AnthropicSseToResponsesConverter {
    buffer: String,
    utf8_remainder: Vec<u8>,
    response_id: String,
    model: String,
    input_usage: Value,
    stop_reason: String,
    output_usage: Value,
    blocks: std::collections::HashMap<u64, OpenBlock>,
    seen_block_indices: std::collections::HashSet<u64>,
    done_items: std::collections::BTreeMap<u64, Value>,
    original_request: Value,
    created_at: u64,
    started: bool,
    message_delta_seen: bool,
    terminal: TerminalState,
}

impl AnthropicSseToResponsesConverter {
    pub fn with_request(original_request: &Value) -> Self {
        Self {
            buffer: String::new(),
            utf8_remainder: Vec::new(),
            response_id: String::new(),
            model: String::new(),
            input_usage: json!({}),
            stop_reason: String::new(),
            output_usage: json!({}),
            blocks: std::collections::HashMap::new(),
            seen_block_indices: std::collections::HashSet::new(),
            done_items: std::collections::BTreeMap::new(),
            original_request: original_request.clone(),
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_secs())
                .unwrap_or(0),
            started: false,
            message_delta_seen: false,
            terminal: TerminalState::Open,
        }
    }

    pub fn push_bytes(&mut self, bytes: &[u8]) -> Vec<u8> {
        if self.terminal != TerminalState::Open {
            return Vec::new();
        }
        crate::protocol_proxy::append_utf8_safe(&mut self.buffer, &mut self.utf8_remainder, bytes);
        let mut output = String::new();
        while let Some(block) = crate::protocol_proxy::take_sse_block(&mut self.buffer) {
            if !block.trim().is_empty() {
                self.handle_block(&block, &mut output);
            }
            if self.terminal != TerminalState::Open {
                break;
            }
        }
        output.into_bytes()
    }

    pub fn finish(&mut self) -> Vec<u8> {
        if self.terminal != TerminalState::Open {
            return Vec::new();
        }
        if !self.utf8_remainder.is_empty() {
            self.buffer
                .push_str(&String::from_utf8_lossy(&self.utf8_remainder));
            self.utf8_remainder.clear();
        }
        let mut output = String::new();
        while let Some(block) = crate::protocol_proxy::take_sse_block(&mut self.buffer) {
            if !block.trim().is_empty() {
                self.handle_block(&block, &mut output);
            }
            if self.terminal != TerminalState::Open {
                return output.into_bytes();
            }
        }
        let tail = std::mem::take(&mut self.buffer);
        if !tail.trim().is_empty() {
            self.handle_block(&tail, &mut output);
        }
        if self.terminal == TerminalState::Open {
            self.emit_failed(
                &mut output,
                "upstream stream ended before message_stop".to_string(),
                Some("unexpected_eof".to_string()),
            );
        }
        output.into_bytes()
    }

    pub fn fail(&mut self, message: String, error_type: Option<String>) -> Vec<u8> {
        if self.terminal != TerminalState::Open {
            return Vec::new();
        }
        let mut output = String::new();
        self.emit_failed(&mut output, message, error_type);
        output.into_bytes()
    }

    fn handle_block(&mut self, block: &str, output: &mut String) {
        if self.terminal != TerminalState::Open {
            return;
        }
        let mut event_name = String::new();
        let mut data_parts = Vec::new();
        for line in block.lines() {
            if let Some(value) = crate::protocol_proxy::strip_sse_field(line, "event") {
                event_name = value.trim().to_string();
            }
            if let Some(value) = crate::protocol_proxy::strip_sse_field(line, "data") {
                data_parts.push(value.to_string());
            }
        }
        let recognized = matches!(
            event_name.as_str(),
            "message_start"
                | "content_block_start"
                | "content_block_delta"
                | "content_block_stop"
                | "message_delta"
                | "message_stop"
                | "error"
        );
        if !recognized {
            return;
        }
        if data_parts.is_empty() {
            self.emit_invalid_sse(output);
            return;
        }
        let Ok(data) = serde_json::from_str::<Value>(&data_parts.join("\n")) else {
            self.emit_invalid_sse(output);
            return;
        };
        if data.get("type").and_then(Value::as_str) != Some(event_name.as_str()) {
            self.emit_invalid_sse(output);
            return;
        }
        match event_name.as_str() {
            "message_start" => self.on_message_start(&data, output),
            "content_block_start" => self.on_block_start(&data, output),
            "content_block_delta" => self.on_block_delta(&data, output),
            "content_block_stop" => self.on_block_stop(&data, output),
            "message_delta" => self.on_message_delta(&data, output),
            "message_stop" => {
                if !self.started
                    || !self.blocks.is_empty()
                    || !self.message_delta_seen
                    || self.stop_reason.is_empty()
                {
                    self.emit_invalid_sse(output);
                } else {
                    self.emit_completed(output);
                }
            }
            "error" => {
                let Some(error) = data.get("error").and_then(Value::as_object) else {
                    self.emit_invalid_sse(output);
                    return;
                };
                let (Some(error_type), Some(message)) = (
                    error.get("type").and_then(Value::as_str),
                    error.get("message").and_then(Value::as_str),
                ) else {
                    self.emit_invalid_sse(output);
                    return;
                };
                self.emit_failed(output, message.to_string(), Some(error_type.to_string()));
            }
            _ => unreachable!(),
        }
    }

    fn response_skeleton(&self, status: &str) -> Value {
        json!({
            "id": self.response_id,
            "object": "response",
            "created_at": self.created_at,
            "status": status,
            "model": self.model,
            "output": [],
        })
    }

    fn emit(output: &mut String, event: &str, data: Value) {
        output.push_str("event: ");
        output.push_str(event);
        output.push_str("\ndata: ");
        output.push_str(&serde_json::to_string(&data).unwrap_or_else(|_| "{}".to_string()));
        output.push_str("\n\n");
    }

    fn on_message_start(&mut self, data: &Value, output: &mut String) {
        if self.started {
            self.emit_invalid_sse(output);
            return;
        }
        let Some(message) = data.get("message").and_then(Value::as_object) else {
            self.emit_invalid_sse(output);
            return;
        };
        let (Some(id), Some(model)) = (
            message.get("id").and_then(Value::as_str),
            message.get("model").and_then(Value::as_str),
        ) else {
            self.emit_invalid_sse(output);
            return;
        };
        let Some(usage) = message.get("usage").filter(|usage| usage.is_object()) else {
            self.emit_invalid_sse(output);
            return;
        };
        self.response_id = id.to_string();
        self.model = model.to_string();
        self.input_usage = usage.clone();
        self.started = true;
        Self::emit(
            output,
            "response.created",
            json!({
                "type": "response.created",
                "response": self.response_skeleton("in_progress"),
            }),
        );
        Self::emit(
            output,
            "response.in_progress",
            json!({
                "type": "response.in_progress",
                "response": self.response_skeleton("in_progress"),
            }),
        );
    }

    fn on_block_start(&mut self, data: &Value, output: &mut String) {
        if !self.started || self.message_delta_seen || !self.blocks.is_empty() {
            self.emit_invalid_sse(output);
            return;
        }
        let (Some(index), Some(block)) = (
            data.get("index").and_then(Value::as_u64),
            data.get("content_block").and_then(Value::as_object),
        ) else {
            self.emit_invalid_sse(output);
            return;
        };
        if self.seen_block_indices.contains(&index) {
            self.emit_invalid_sse(output);
            return;
        }
        let Some(kind) = block.get("type").and_then(Value::as_str) else {
            self.emit_invalid_sse(output);
            return;
        };
        let valid = match kind {
            "text" => block.get("text").is_some_and(Value::is_string),
            "tool_use" => {
                block.get("id").is_some_and(Value::is_string)
                    && block.get("name").is_some_and(Value::is_string)
                    && block.get("input").is_some_and(Value::is_object)
            }
            "thinking" => {
                block.get("thinking").is_some_and(Value::is_string)
                    && block.get("signature").is_none_or(Value::is_string)
            }
            "redacted_thinking" => block.get("data").is_some_and(Value::is_string),
            _ => false,
        };
        if !valid {
            self.emit_invalid_sse(output);
            return;
        }
        self.seen_block_indices.insert(index);
        let output_index = self.done_items.len() as u64 + self.blocks.len() as u64;
        let response_kind = match kind {
            "text" => "message",
            "tool_use" => "function_call",
            "thinking" | "redacted_thinking" => "reasoning",
            _ => unreachable!(),
        };
        let item_id = format!("{}-{response_kind}-{index}", self.response_id);
        match kind {
            "text" => {
                Self::emit(
                    output,
                    "response.output_item.added",
                    json!({
                        "type":"response.output_item.added","output_index":output_index,
                        "item":{"type":"message","id":item_id,"role":"assistant","status":"in_progress","content":[]}
                    }),
                );
                Self::emit(
                    output,
                    "response.content_part.added",
                    json!({
                        "type":"response.content_part.added","item_id":item_id,"output_index":output_index,
                        "content_index":0,"part":{"type":"output_text","text":"","annotations":[]}
                    }),
                );
            }
            "tool_use" => Self::emit(
                output,
                "response.output_item.added",
                json!({
                    "type":"response.output_item.added","output_index":output_index,
                    "item":{"type":"function_call","id":item_id,
                        "call_id":block.get("id").and_then(Value::as_str).unwrap_or(""),
                        "name":block.get("name").and_then(Value::as_str).unwrap_or(""),
                        "arguments":"","status":"in_progress"}
                }),
            ),
            "thinking" => {
                Self::emit(
                    output,
                    "response.output_item.added",
                    json!({
                        "type":"response.output_item.added","output_index":output_index,
                        "item":{"type":"reasoning","id":item_id,"summary":[]}
                    }),
                );
                Self::emit(
                    output,
                    "response.reasoning_summary_part.added",
                    json!({
                        "type":"response.reasoning_summary_part.added","item_id":item_id,
                        "output_index":output_index,"summary_index":0,
                        "part":{"type":"summary_text","text":""}
                    }),
                );
            }
            "redacted_thinking" => Self::emit(
                output,
                "response.output_item.added",
                json!({
                    "type":"response.output_item.added","output_index":output_index,
                    "item":{"type":"reasoning","id":item_id,"summary":[]}
                }),
            ),
            _ => unreachable!(),
        }
        self.blocks.insert(
            index,
            OpenBlock {
                kind: kind.to_string(),
                item_id,
                output_index,
                content_index: 0,
                call_id: block
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                name: block
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                text_buffer: block
                    .get(match kind {
                        "thinking" => "thinking",
                        "redacted_thinking" => "data",
                        _ => "text",
                    })
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                args_buffer: String::new(),
                initial_tool_input: block
                    .get("input")
                    .map(stable_json_string)
                    .unwrap_or_default(),
                received_args_delta: false,
                signature: block
                    .get("signature")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            },
        );
    }

    fn on_block_delta(&mut self, data: &Value, output: &mut String) {
        let (Some(index), Some(delta)) = (
            data.get("index").and_then(Value::as_u64),
            data.get("delta").and_then(Value::as_object),
        ) else {
            self.emit_invalid_sse(output);
            return;
        };
        let Some(delta_type) = delta.get("type").and_then(Value::as_str) else {
            self.emit_invalid_sse(output);
            return;
        };
        let field = match delta_type {
            "text_delta" => "text",
            "input_json_delta" => "partial_json",
            "thinking_delta" => "thinking",
            "signature_delta" => "signature",
            _ => {
                self.emit_invalid_sse(output);
                return;
            }
        };
        let Some(value) = delta.get(field).and_then(Value::as_str) else {
            self.emit_invalid_sse(output);
            return;
        };
        let Some(block) = self.blocks.get_mut(&index) else {
            self.emit_invalid_sse(output);
            return;
        };
        let compatible = matches!(
            (block.kind.as_str(), delta_type),
            ("text", "text_delta")
                | ("tool_use", "input_json_delta")
                | ("thinking", "thinking_delta")
                | ("thinking", "signature_delta")
        );
        if !compatible {
            self.emit_invalid_sse(output);
            return;
        }
        match delta_type {
            "text_delta" => {
                block.text_buffer.push_str(value);
                Self::emit(
                    output,
                    "response.output_text.delta",
                    json!({
                        "type":"response.output_text.delta","item_id":block.item_id,
                        "output_index":block.output_index,"content_index":block.content_index,"delta":value
                    }),
                );
            }
            "input_json_delta" => {
                block.received_args_delta = true;
                block.args_buffer.push_str(value);
                Self::emit(
                    output,
                    "response.function_call_arguments.delta",
                    json!({
                        "type":"response.function_call_arguments.delta","item_id":block.item_id,
                        "output_index":block.output_index,"delta":value
                    }),
                );
            }
            "thinking_delta" => {
                block.text_buffer.push_str(value);
                Self::emit(
                    output,
                    "response.reasoning_summary_text.delta",
                    json!({
                        "type":"response.reasoning_summary_text.delta","item_id":block.item_id,
                        "output_index":block.output_index,"summary_index":0,"delta":value
                    }),
                );
            }
            "signature_delta" => block
                .signature
                .get_or_insert_with(String::new)
                .push_str(value),
            _ => unreachable!(),
        }
    }

    fn on_block_stop(&mut self, data: &Value, output: &mut String) {
        let Some(index) = data.get("index").and_then(Value::as_u64) else {
            self.emit_invalid_sse(output);
            return;
        };
        let Some(block) = self.blocks.remove(&index) else {
            self.emit_invalid_sse(output);
            return;
        };
        let item = match block.kind.as_str() {
            "text" => {
                Self::emit(
                    output,
                    "response.output_text.done",
                    json!({
                        "type":"response.output_text.done","item_id":block.item_id,"output_index":block.output_index,
                        "content_index":block.content_index,"text":block.text_buffer
                    }),
                );
                Self::emit(
                    output,
                    "response.content_part.done",
                    json!({
                        "type":"response.content_part.done","item_id":block.item_id,"output_index":block.output_index,
                        "content_index":block.content_index,"part":{"type":"output_text","text":block.text_buffer,"annotations":[]}
                    }),
                );
                json!({"type":"message","id":block.item_id,"role":"assistant","status":"completed",
                    "content":[{"type":"output_text","text":block.text_buffer,"annotations":[]}]})
            }
            "tool_use" => {
                let arguments = if block.received_args_delta {
                    block.args_buffer.clone()
                } else {
                    block.initial_tool_input.clone()
                };
                if !serde_json::from_str::<Value>(&arguments).is_ok_and(|value| value.is_object()) {
                    self.emit_invalid_sse(output);
                    return;
                }
                Self::emit(
                    output,
                    "response.function_call_arguments.done",
                    json!({
                        "type":"response.function_call_arguments.done","item_id":block.item_id,
                        "output_index":block.output_index,"arguments":arguments
                    }),
                );
                json!({"type":"function_call","id":block.item_id,"call_id":block.call_id,
                    "name":block.name,"arguments":arguments,"status":"completed"})
            }
            "thinking" => {
                Self::emit(
                    output,
                    "response.reasoning_summary_text.done",
                    json!({
                        "type":"response.reasoning_summary_text.done","item_id":block.item_id,
                        "output_index":block.output_index,"summary_index":0,"text":block.text_buffer
                    }),
                );
                Self::emit(
                    output,
                    "response.reasoning_summary_part.done",
                    json!({
                        "type":"response.reasoning_summary_part.done","item_id":block.item_id,
                        "output_index":block.output_index,"summary_index":0,
                        "part":{"type":"summary_text","text":block.text_buffer}
                    }),
                );
                let mut envelope_block = json!({
                    "type":"thinking",
                    "thinking":block.text_buffer
                });
                let mut item = json!({"type":"reasoning","id":block.item_id,
                    "summary":[{"type":"summary_text","text":block.text_buffer}]});
                if let Some(signature) = block.signature {
                    envelope_block["signature"] = json!(signature);
                    item["signature"] = json!(signature);
                }
                item["encrypted_content"] = json!(encode_thinking_envelope(&envelope_block));
                item
            }
            "redacted_thinking" => {
                let envelope_block = json!({
                    "type":"redacted_thinking",
                    "data":block.text_buffer
                });
                json!({
                    "type":"reasoning",
                    "id":block.item_id,
                    "summary":[],
                    "encrypted_content":encode_thinking_envelope(&envelope_block)
                })
            }
            _ => unreachable!(),
        };
        Self::emit(
            output,
            "response.output_item.done",
            json!({
                "type":"response.output_item.done","output_index":block.output_index,"item":item
            }),
        );
        self.done_items.insert(block.output_index, item);
    }

    fn on_message_delta(&mut self, data: &Value, output: &mut String) {
        if !self.started || !self.blocks.is_empty() {
            self.emit_invalid_sse(output);
            return;
        }
        let (Some(delta), Some(usage)) = (
            data.get("delta").and_then(Value::as_object),
            data.get("usage").and_then(Value::as_object),
        ) else {
            self.emit_invalid_sse(output);
            return;
        };
        let invalid_stop_reason = delta
            .get("stop_reason")
            .is_some_and(|value| !value.is_null() && !value.is_string());
        let invalid_usage = usage.iter().any(|(key, value)| {
            if value.is_null() {
                return key == "output_tokens";
            }
            match key.as_str() {
                "input_tokens"
                | "cache_creation_input_tokens"
                | "cache_read_input_tokens"
                | "output_tokens" => !value.is_u64(),
                "output_tokens_details" | "server_tool_use" => !value.is_object(),
                _ => false,
            }
        });
        if invalid_stop_reason
            || !usage.get("output_tokens").is_some_and(Value::is_u64)
            || invalid_usage
        {
            self.emit_invalid_sse(output);
            return;
        }
        if let Some(stop) = delta.get("stop_reason").and_then(Value::as_str) {
            self.stop_reason = stop.to_string();
        }
        let Some(output_usage) = self.output_usage.as_object_mut() else {
            self.emit_invalid_sse(output);
            return;
        };
        for (key, value) in usage {
            if !value.is_null() {
                output_usage.insert(key.clone(), value.clone());
            }
        }
        self.message_delta_seen = true;
    }

    fn emit_completed(&mut self, output: &mut String) {
        if self.terminal != TerminalState::Open {
            return;
        }
        let mut usage = self.input_usage.clone();
        if let (Some(target), Some(source)) = (usage.as_object_mut(), self.output_usage.as_object())
        {
            for (key, value) in source {
                target.insert(key.clone(), value.clone());
            }
        }
        let Ok(converted_usage) = anthropic_usage_to_responses_usage(Some(&usage)) else {
            self.emit_invalid_sse(output);
            return;
        };
        let incomplete = self.stop_reason == "max_tokens";
        self.terminal = if incomplete {
            TerminalState::Incomplete
        } else {
            TerminalState::Completed
        };
        let status = if incomplete {
            "incomplete"
        } else {
            "completed"
        };
        let mut response = self.response_skeleton(status);
        response["output"] = Value::Array(self.done_items.values().cloned().collect());
        response["usage"] = converted_usage;
        copy_response_request_fields(&mut response, &self.original_request);
        if incomplete {
            response["incomplete_details"] = json!({"reason":"max_output_tokens"});
        }
        let event = if incomplete {
            "response.incomplete"
        } else {
            "response.completed"
        };
        Self::emit(output, event, json!({"type":event,"response":response}));
    }

    fn emit_invalid_sse(&mut self, output: &mut String) {
        self.emit_failed(
            output,
            "invalid Anthropic SSE event".to_string(),
            Some("invalid_sse_event".to_string()),
        );
    }

    fn emit_failed(&mut self, output: &mut String, message: String, error_type: Option<String>) {
        if self.terminal != TerminalState::Open {
            return;
        }
        self.terminal = TerminalState::Failed;
        let error_type = error_type.unwrap_or_else(|| "upstream_error".to_string());
        let mut response = self.response_skeleton("failed");
        response["output"] = Value::Array(self.done_items.values().cloned().collect());
        copy_response_request_fields(&mut response, &self.original_request);
        response["error"] = json!({"code":error_type,"type":error_type,"message":message});
        Self::emit(
            output,
            "response.failed",
            json!({
                "type":"response.failed","response":response
            }),
        );
    }
}

/// Anthropic 上游请求构造：三头认证（x-api-key + anthropic-version + Bearer）。
/// originator 非空时透传给上游（spec 硬约束：codex 的 originator 头需保留）。
pub fn anthropic_request_builder(
    client: reqwest::Client,
    endpoint: &str,
    api_key: &str,
    is_stream: bool,
    upstream_body: &Value,
    originator: Option<&str>,
) -> reqwest::RequestBuilder {
    let mut builder = client
        .post(endpoint)
        .header("x-api-key", api_key)
        .header("anthropic-version", ANTHROPIC_VERSION)
        .bearer_auth(api_key)
        .header(reqwest::header::CONTENT_TYPE, "application/json");
    if is_stream {
        builder = builder
            .header(reqwest::header::ACCEPT, "text/event-stream")
            .header(reqwest::header::CACHE_CONTROL, "no-cache");
    }
    if let Some(originator) = originator.map(str::trim).filter(|value| !value.is_empty()) {
        builder = builder.header("originator", originator);
    }
    builder.json(upstream_body)
}

/// Anthropic Models API 响应 → OpenAI /v1/models 格式。
/// 非 Anthropic 格式（无 "type":"model" 条目）原样返回，便于直接透传。
pub fn anthropic_models_to_openai_models(body: &Value) -> Value {
    let Some(data) = body.get("data").and_then(Value::as_array) else {
        return body.clone();
    };
    let is_anthropic = data
        .iter()
        .any(|item| item.get("type").and_then(Value::as_str) == Some("model"))
        || (data.is_empty()
            && body.get("has_more").is_some_and(Value::is_boolean)
            && (body.get("first_id").is_some() || body.get("last_id").is_some()));
    if !is_anthropic {
        return body.clone();
    }
    let models: Vec<Value> = data
        .iter()
        .filter(|item| item.get("type").and_then(Value::as_str) == Some("model"))
        .map(|item| {
            let created = item
                .get("created_at")
                .and_then(Value::as_str)
                .and_then(parse_rfc3339_unix_seconds)
                .unwrap_or(0);
            json!({
                "id": item.get("id").and_then(Value::as_str).unwrap_or(""),
                "object": "model",
                "created": created,
                "owned_by": "anthropic",
            })
        })
        .collect();
    json!({"object": "list", "data": models})
}

/// 解析常用 RFC 3339 子集为 unix 秒：四位年份、秒精度时间、可选小数秒及 `Z`/数值偏移。
/// 项目无 chrono/time 直接依赖，故手写；解析失败返回 None。
fn parse_rfc3339_unix_seconds(text: &str) -> Option<i64> {
    let bytes = text.as_bytes();
    if bytes.len() < 20 {
        return None;
    }
    let digit = |index: usize| -> Option<i64> {
        bytes
            .get(index)
            .filter(|ch| ch.is_ascii_digit())
            .map(|ch| (ch - b'0') as i64)
    };
    let num2 = |index: usize| -> Option<i64> { Some(digit(index)? * 10 + digit(index + 1)?) };
    let year = digit(0)? * 1000 + digit(1)? * 100 + digit(2)? * 10 + digit(3)?;
    let month = num2(5)?;
    let day = num2(8)?;
    let hour = num2(11)?;
    let minute = num2(14)?;
    let second = num2(17)?;
    if bytes.get(4) != Some(&b'-')
        || bytes.get(7) != Some(&b'-')
        || bytes.get(10) != Some(&b'T')
        || bytes.get(13) != Some(&b':')
        || bytes.get(16) != Some(&b':')
    {
        return None;
    }
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let max_day = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return None,
    };
    if !(1..=max_day).contains(&day) || hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    let mut timezone_index = 19;
    if bytes.get(timezone_index) == Some(&b'.') {
        timezone_index += 1;
        let fraction_start = timezone_index;
        while bytes
            .get(timezone_index)
            .is_some_and(|value| value.is_ascii_digit())
        {
            timezone_index += 1;
        }
        if timezone_index == fraction_start {
            return None;
        }
    }
    let offset_seconds = match bytes.get(timezone_index).copied() {
        Some(b'Z') if timezone_index + 1 == bytes.len() => 0,
        Some(sign @ (b'+' | b'-'))
            if timezone_index + 6 == bytes.len()
                && bytes.get(timezone_index + 3) == Some(&b':') =>
        {
            let offset_hour = num2(timezone_index + 1)?;
            let offset_minute = num2(timezone_index + 4)?;
            if offset_hour > 23 || offset_minute > 59 {
                return None;
            }
            let value = offset_hour * 3600 + offset_minute * 60;
            if sign == b'+' { value } else { -value }
        }
        _ => return None,
    };
    // Howard Hinnant 的 days_from_civil：公历日期 → 距 1970-01-01 的天数
    let shifted_year = if month <= 2 { year - 1 } else { year };
    let era = if shifted_year >= 0 {
        shifted_year
    } else {
        shifted_year - 399
    } / 400;
    let year_of_era = shifted_year - era * 400;
    let shifted_month = if month > 2 { month - 3 } else { month + 9 };
    let day_of_year = (153 * shifted_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146_097 + day_of_era - 719_468;
    Some(days * 86_400 + hour * 3600 + minute * 60 + second - offset_seconds)
}

/// Anthropic 错误体 → codex Responses 错误结构。
pub fn anthropic_error_to_responses_error(status_code: u16, body: &[u8]) -> Value {
    let parsed = serde_json::from_slice::<Value>(body).ok();
    let (message, error_type) = parsed
        .as_ref()
        .and_then(|value| value.get("error"))
        .map(|error| match error {
            Value::String(message) => (message.clone(), "upstream_error".to_string()),
            _ => (
                error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("upstream error")
                    .to_string(),
                error
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or("upstream_error")
                    .to_string(),
            ),
        })
        .unwrap_or_else(|| {
            (
                String::from_utf8_lossy(body).chars().take(256).collect(),
                "upstream_error".to_string(),
            )
        });
    json!({
        "error": {
            "message": message,
            "type": error_type,
            "code": status_code,
        }
    })
}
