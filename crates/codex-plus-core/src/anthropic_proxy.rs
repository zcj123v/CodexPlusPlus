//! Anthropic Messages API 转换层。
//! codex Responses API ↔ Anthropic Messages API 的请求/响应/SSE 双向转换。
//! 硬约束：工具定义、调用参数、工具结果、system prompt 语义零改写（见设计文档第 3 节）。

use serde_json::{json, Value};

pub const ANTHROPIC_VERSION: &str = "2023-06-01";
pub const DEFAULT_ANTHROPIC_MAX_TOKENS: u64 = 32000;

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

    if let Some(items) = body.get("input").and_then(Value::as_array) {
        for item in items {
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
                    let input = serde_json::from_str::<Value>(arguments)
                        .unwrap_or_else(|_| json!({}));
                    pending_assistant.push(json!({
                        "type": "tool_use",
                        "id": item.get("call_id").and_then(Value::as_str).unwrap_or(""),
                        "name": item.get("name").and_then(Value::as_str).unwrap_or(""),
                        "input": input,
                    }));
                }
                Some("function_call_output") => {
                    flush_assistant(&mut messages, &mut pending_assistant);
                    let output = item.get("output").cloned().unwrap_or(json!(""));
                    pending_user.push(json!({
                        "type": "tool_result",
                        "tool_use_id": item.get("call_id").and_then(Value::as_str).unwrap_or(""),
                        "content": output,
                    }));
                }
                Some("reasoning") => {
                    // 仅当带有我们先前转换时存下的 signature 才回传 thinking block
                    if let Some(signature) = item.get("signature").and_then(Value::as_str) {
                        flush_user(&mut messages, &mut pending_user);
                        let thinking = reasoning_item_text(item);
                        pending_assistant.push(json!({
                            "type": "thinking",
                            "thinking": thinking,
                            "signature": signature,
                        }));
                    }
                }
                _ => { /* 未知 item 类型：跳过，不中断请求 */ }
            }
        }
    }
    flush_assistant(&mut messages, &mut pending_assistant);
    flush_user(&mut messages, &mut pending_user);
    if messages.is_empty() {
        messages.push(json!({"role":"user","content":[{"type":"text","text":""}]}));
    }

    if !system_blocks.is_empty() {
        if let Some(last) = system_blocks.last_mut() {
            last["cache_control"] = json!({"type": "ephemeral"});
        }
        result["system"] = json!(system_blocks);
    }
    result["messages"] = json!(messages);

    // ── tools 1:1（schema 原文 clone）──
    if let Some(tools) = body.get("tools").and_then(Value::as_array) {
        let mut converted: Vec<Value> = tools
            .iter()
            .filter(|tool| tool.get("type").and_then(Value::as_str) == Some("function"))
            .map(|tool| {
                json!({
                    "name": tool.get("name").and_then(Value::as_str).unwrap_or(""),
                    "description": tool.get("description").and_then(Value::as_str).unwrap_or(""),
                    "input_schema": tool.get("parameters").cloned().unwrap_or(json!({"type":"object"})),
                })
            })
            .collect();
        if !converted.is_empty() {
            if let Some(last) = converted.last_mut() {
                last["cache_control"] = json!({"type": "ephemeral"});
            }
            result["tools"] = json!(converted);
        }
    }

    // ── 采样与输出参数 ──
    result["max_tokens"] = body
        .get("max_output_tokens")
        .and_then(Value::as_u64)
        .map(Value::from)
        .unwrap_or_else(|| json!(DEFAULT_ANTHROPIC_MAX_TOKENS));
    for key in ["temperature", "top_p", "stream"] {
        if let Some(value) = body.get(key) {
            result[key] = value.clone();
        }
    }

    // ── reasoning effort → thinking 预算 ──
    if let Some(effort) = body
        .get("reasoning")
        .and_then(|r| r.get("effort"))
        .and_then(Value::as_str)
    {
        let budget = match effort {
            "none" | "minimal" => None,
            "low" => Some(2048),
            "high" => Some(16384),
            "xhigh" => Some(32768),
            _ => Some(8192), // medium 及未知值
        };
        if let Some(budget) = budget {
            result["thinking"] = json!({"type": "enabled", "budget_tokens": budget});
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

/// 提取 reasoning item 的文本（summary 首段优先，其次 content 首段）。
fn reasoning_item_text(item: &Value) -> String {
    for key in ["summary", "content"] {
        if let Some(parts) = item.get(key).and_then(Value::as_array) {
            for part in parts {
                if let Some(text) = part.get("text").and_then(Value::as_str) {
                    return text.to_string();
                }
            }
        }
    }
    String::new()
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
        for block in blocks {
            match block.get("type").and_then(Value::as_str) {
                Some("thinking") => {
                    let mut item = json!({
                        "type": "reasoning",
                        "id": response_id,
                        "summary": [{
                            "type": "summary_text",
                            "text": block.get("thinking").and_then(Value::as_str).unwrap_or("")
                        }]
                    });
                    // 存下签名供多轮回传（Task 2 请求转换读取）
                    if let Some(signature) = block.get("signature").and_then(Value::as_str) {
                        item["signature"] = json!(signature);
                    }
                    output.push(item);
                }
                Some("text") => {
                    output.push(json!({
                        "type": "message",
                        "id": response_id,
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
                        "id": response_id,
                        "call_id": block.get("id").and_then(Value::as_str).unwrap_or(""),
                        "name": block.get("name").and_then(Value::as_str).unwrap_or(""),
                        "arguments": serde_json::to_string(&input).unwrap_or_else(|_| "{}".to_string()),
                        "status": "completed"
                    }));
                }
                _ => { /* 未知 block：跳过 */ }
            }
        }
    }

    let stop_reason = body.get("stop_reason").and_then(Value::as_str).unwrap_or("");
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
        "usage": anthropic_usage_to_responses_usage(body.get("usage")),
    });
    if stop_reason == "max_tokens" {
        response["incomplete_details"] = json!({"reason": "max_output_tokens"});
    }
    // 回显请求侧关键字段（与 chat 转换 copy_response_request_fields 对齐的最小子集）
    if let Some(request) = original_request {
        for key in ["instructions", "parallel_tool_calls", "store"] {
            if let Some(value) = request.get(key) {
                response[key] = value.clone();
            }
        }
    }
    Ok(response)
}

/// Anthropic usage → Responses usage（缓存 token 计入总输入）。
pub(crate) fn anthropic_usage_to_responses_usage(usage: Option<&Value>) -> Value {
    let Some(usage) = usage else {
        return json!({"input_tokens": 0, "output_tokens": 0, "total_tokens": 0});
    };
    let input = usage.get("input_tokens").and_then(Value::as_u64).unwrap_or(0);
    let output = usage.get("output_tokens").and_then(Value::as_u64).unwrap_or(0);
    let cached = usage
        .get("cache_read_input_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let cache_creation = usage
        .get("cache_creation_input_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    // Anthropic 的 input_tokens 不含缓存读/写，而 OpenAI Responses 的
    // input_tokens 是含缓存的总输入（codex 依赖它做上下文窗口核算），
    // 因此需要把 cache_read 与 cache_creation 都计入总输入。
    let total_input = input + cached + cache_creation;
    json!({
        "input_tokens": total_input,
        "output_tokens": output,
        "total_tokens": total_input + output,
        "input_tokens_details": {"cached_tokens": cached},
        "output_tokens_details": {"reasoning_tokens": 0}
    })
}

// ── SSE 流式转换 ──

/// 打开的 content block 状态。
#[derive(Default)]
struct OpenBlock {
    kind: String, // "text" | "tool_use" | "thinking"
    item_id: String,
    output_index: u64,
    content_index: u64,
    call_id: String,
    name: String,
    text_buffer: String,
    args_buffer: String,
    signature: String,
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
    next_output_index: u64,
    started: bool,
    completed: bool,
    failed: bool,
}

impl AnthropicSseToResponsesConverter {
    pub fn with_request(_original_request: &Value) -> Self {
        Self {
            buffer: String::new(),
            utf8_remainder: Vec::new(),
            response_id: String::new(),
            model: String::new(),
            input_usage: json!({}),
            stop_reason: String::new(),
            output_usage: json!({}),
            blocks: std::collections::HashMap::new(),
            next_output_index: 0,
            started: false,
            completed: false,
            failed: false,
        }
    }

    pub fn push_bytes(&mut self, bytes: &[u8]) -> Vec<u8> {
        crate::protocol_proxy::append_utf8_safe(&mut self.buffer, &mut self.utf8_remainder, bytes);
        let mut output = String::new();
        while let Some(block) = crate::protocol_proxy::take_sse_block(&mut self.buffer) {
            if block.trim().is_empty() {
                continue;
            }
            self.handle_block(&block, &mut output);
            if self.failed {
                break;
            }
        }
        output.into_bytes()
    }

    pub fn finish(&mut self) -> Vec<u8> {
        if !self.utf8_remainder.is_empty() {
            self.buffer
                .push_str(&String::from_utf8_lossy(&self.utf8_remainder));
            self.utf8_remainder.clear();
        }
        let mut output = String::new();
        // 与 push_bytes 相同的逻辑：先把 buffer 中完整的 SSE block 处理完
        while let Some(block) = crate::protocol_proxy::take_sse_block(&mut self.buffer) {
            if block.trim().is_empty() {
                continue;
            }
            self.handle_block(&block, &mut output);
            if self.failed {
                break;
            }
        }
        // buffer 中若仍有非空残留，说明最后一个事件没带尾部空行，
        // 上游已结束，按完整 block 尝试解析一次（如末尾的 message_delta/message_stop）
        if !self.failed {
            let tail = std::mem::take(&mut self.buffer);
            if !tail.trim().is_empty() {
                self.handle_block(&tail, &mut output);
            }
        }
        // 上游提前断流：尽量收尾，未收到 message_stop 也补一个 completed
        if self.started && !self.completed && !self.failed {
            self.emit_completed(&mut output);
        }
        output.into_bytes()
    }

    pub fn fail(&mut self, message: String, error_type: Option<String>) -> Vec<u8> {
        let mut output = String::new();
        self.emit_failed(&mut output, message, error_type);
        output.into_bytes()
    }

    fn handle_block(&mut self, block: &str, output: &mut String) {
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
        if data_parts.is_empty() {
            return;
        }
        let Ok(data) = serde_json::from_str::<Value>(&data_parts.join("\n")) else {
            return;
        };
        match event_name.as_str() {
            "message_start" => self.on_message_start(&data, output),
            "content_block_start" => self.on_block_start(&data, output),
            "content_block_delta" => self.on_block_delta(&data, output),
            "content_block_stop" => self.on_block_stop(&data, output),
            "message_delta" => {
                if let Some(stop) = data
                    .get("delta")
                    .and_then(|d| d.get("stop_reason"))
                    .and_then(Value::as_str)
                {
                    self.stop_reason = stop.to_string();
                }
                if let Some(usage) = data.get("usage") {
                    self.output_usage = usage.clone();
                }
            }
            "message_stop" => self.emit_completed(output),
            "error" => {
                let message = data
                    .get("error")
                    .and_then(|e| e.get("message"))
                    .and_then(Value::as_str)
                    .unwrap_or("upstream stream error")
                    .to_string();
                let error_type = data
                    .get("error")
                    .and_then(|e| e.get("type"))
                    .and_then(Value::as_str)
                    .map(str::to_string);
                self.emit_failed(output, message, error_type);
            }
            _ => { /* ping 等：忽略 */ }
        }
    }

    fn response_skeleton(&self, status: &str) -> Value {
        json!({
            "id": self.response_id,
            "object": "response",
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
        let message = data.get("message").cloned().unwrap_or(json!({}));
        self.response_id = message
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("msg_unknown")
            .to_string();
        self.model = message
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        self.input_usage = message.get("usage").cloned().unwrap_or(json!({}));
        self.started = true;
        Self::emit(output, "response.created", json!({
            "type": "response.created",
            "response": self.response_skeleton("in_progress"),
        }));
        Self::emit(output, "response.in_progress", json!({
            "type": "response.in_progress",
            "response": self.response_skeleton("in_progress"),
        }));
    }

    fn on_block_start(&mut self, data: &Value, output: &mut String) {
        let index = data.get("index").and_then(Value::as_u64).unwrap_or(0);
        let block = data.get("content_block").cloned().unwrap_or(json!({}));
        let kind = block
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let output_index = self.next_output_index;
        self.next_output_index += 1;
        let item_id = format!("{}-item-{output_index}", self.response_id);

        match kind.as_str() {
            "text" => {
                Self::emit(output, "response.output_item.added", json!({
                    "type": "response.output_item.added",
                    "output_index": output_index,
                    "item": {"type":"message","id":item_id,"role":"assistant","status":"in_progress","content":[]}
                }));
                Self::emit(output, "response.content_part.added", json!({
                    "type": "response.content_part.added",
                    "item_id": item_id,
                    "output_index": output_index,
                    "content_index": 0,
                    "part": {"type":"output_text","text":"","annotations":[]}
                }));
            }
            "tool_use" => {
                Self::emit(output, "response.output_item.added", json!({
                    "type": "response.output_item.added",
                    "output_index": output_index,
                    "item": {
                        "type":"function_call","id":item_id,
                        "call_id": block.get("id").and_then(Value::as_str).unwrap_or(""),
                        "name": block.get("name").and_then(Value::as_str).unwrap_or(""),
                        "arguments": "", "status": "in_progress"
                    }
                }));
            }
            "thinking" => {
                Self::emit(output, "response.output_item.added", json!({
                    "type": "response.output_item.added",
                    "output_index": output_index,
                    "item": {"type":"reasoning","id":item_id,"summary":[]}
                }));
            }
            _ => { /* 未知 block 类型：忽略 */ }
        }
        self.blocks.insert(
            index,
            OpenBlock {
                kind,
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
                ..OpenBlock::default()
            },
        );
    }

    fn on_block_delta(&mut self, data: &Value, output: &mut String) {
        let index = data.get("index").and_then(Value::as_u64).unwrap_or(0);
        let Some(block) = self.blocks.get_mut(&index) else {
            return;
        };
        let delta = data.get("delta").cloned().unwrap_or(json!({}));
        match delta.get("type").and_then(Value::as_str) {
            Some("text_delta") => {
                let text = delta.get("text").and_then(Value::as_str).unwrap_or("");
                block.text_buffer.push_str(text);
                Self::emit(output, "response.output_text.delta", json!({
                    "type": "response.output_text.delta",
                    "item_id": block.item_id,
                    "output_index": block.output_index,
                    "content_index": block.content_index,
                    "delta": text,
                }));
            }
            Some("input_json_delta") => {
                // partial_json 片段原样转发，不重序列化
                let partial = delta
                    .get("partial_json")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                block.args_buffer.push_str(partial);
                Self::emit(output, "response.function_call_arguments.delta", json!({
                    "type": "response.function_call_arguments.delta",
                    "item_id": block.item_id,
                    "output_index": block.output_index,
                    "delta": partial,
                }));
            }
            Some("thinking_delta") => {
                let thinking = delta.get("thinking").and_then(Value::as_str).unwrap_or("");
                block.text_buffer.push_str(thinking);
                Self::emit(output, "response.reasoning_summary_text.delta", json!({
                    "type": "response.reasoning_summary_text.delta",
                    "item_id": block.item_id,
                    "output_index": block.output_index,
                    "summary_index": 0,
                    "delta": thinking,
                }));
            }
            Some("signature_delta") => {
                if let Some(sig) = delta.get("signature").and_then(Value::as_str) {
                    block.signature.push_str(sig);
                }
            }
            _ => {}
        }
    }

    fn on_block_stop(&mut self, data: &Value, output: &mut String) {
        let index = data.get("index").and_then(Value::as_u64).unwrap_or(0);
        let Some(block) = self.blocks.remove(&index) else {
            return;
        };
        match block.kind.as_str() {
            "text" => {
                Self::emit(output, "response.output_text.done", json!({
                    "type": "response.output_text.done",
                    "item_id": block.item_id,
                    "output_index": block.output_index,
                    "content_index": block.content_index,
                    "text": block.text_buffer,
                }));
                Self::emit(output, "response.content_part.done", json!({
                    "type": "response.content_part.done",
                    "item_id": block.item_id,
                    "output_index": block.output_index,
                    "content_index": block.content_index,
                    "part": {"type":"output_text","text":block.text_buffer,"annotations":[]}
                }));
                Self::emit(output, "response.output_item.done", json!({
                    "type": "response.output_item.done",
                    "output_index": block.output_index,
                    "item": {"type":"message","id":block.item_id,"role":"assistant","status":"completed",
                             "content":[{"type":"output_text","text":block.text_buffer,"annotations":[]}]}
                }));
            }
            "tool_use" => {
                Self::emit(output, "response.function_call_arguments.done", json!({
                    "type": "response.function_call_arguments.done",
                    "item_id": block.item_id,
                    "output_index": block.output_index,
                    "arguments": block.args_buffer,
                }));
                Self::emit(output, "response.output_item.done", json!({
                    "type": "response.output_item.done",
                    "output_index": block.output_index,
                    "item": {"type":"function_call","id":block.item_id,"call_id":block.call_id,
                             "name":block.name,"arguments":block.args_buffer,"status":"completed"}
                }));
            }
            "thinking" => {
                let mut item = json!({"type":"reasoning","id":block.item_id,
                    "summary":[{"type":"summary_text","text":block.text_buffer}]});
                if !block.signature.is_empty() {
                    item["signature"] = json!(block.signature);
                }
                Self::emit(output, "response.reasoning_summary_text.done", json!({
                    "type": "response.reasoning_summary_text.done",
                    "item_id": block.item_id,
                    "output_index": block.output_index,
                    "summary_index": 0,
                    "text": block.text_buffer,
                }));
                Self::emit(output, "response.output_item.done", json!({
                    "type": "response.output_item.done",
                    "output_index": block.output_index,
                    "item": item,
                }));
            }
            _ => {}
        }
    }

    fn emit_completed(&mut self, output: &mut String) {
        // 已失败或已完成的流不再补 completed，避免 failed 之后再发 completed
        if self.completed || self.failed {
            return;
        }
        self.completed = true;
        // message_start 的 input usage 与 message_delta 的 output usage 合并
        let mut usage = self.input_usage.clone();
        if let (Some(target), Some(source)) = (usage.as_object_mut(), self.output_usage.as_object())
        {
            for (key, value) in source {
                target.insert(key.clone(), value.clone());
            }
        }
        let mut response = self.response_skeleton(if self.stop_reason == "max_tokens" {
            "incomplete"
        } else {
            "completed"
        });
        response["usage"] = anthropic_usage_to_responses_usage(Some(&usage));
        if self.stop_reason == "max_tokens" {
            response["incomplete_details"] = json!({"reason": "max_output_tokens"});
        }
        Self::emit(output, "response.completed", json!({
            "type": "response.completed",
            "response": response,
        }));
    }

    fn emit_failed(&mut self, output: &mut String, message: String, error_type: Option<String>) {
        if self.failed {
            return;
        }
        self.failed = true;
        let mut response = self.response_skeleton("failed");
        response["error"] = json!({
            "code": error_type.unwrap_or_else(|| "upstream_error".to_string()),
            "message": message,
        });
        Self::emit(output, "response.failed", json!({
            "type": "response.failed",
            "response": response,
        }));
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

/// Anthropic 错误体 → codex Responses 错误结构。
pub fn anthropic_error_to_responses_error(status_code: u16, body: &[u8]) -> Value {
    let parsed = serde_json::from_slice::<Value>(body).ok();
    let (message, error_type) = parsed
        .as_ref()
        .and_then(|value| value.get("error"))
        .map(|error| {
            (
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
            )
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
