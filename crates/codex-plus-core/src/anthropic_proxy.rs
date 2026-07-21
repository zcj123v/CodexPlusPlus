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
