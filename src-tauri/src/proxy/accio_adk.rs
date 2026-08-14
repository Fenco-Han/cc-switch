//! Accio gateway adapter.
//!
//! Accio 0.27.x talks to `${gateway}/api/adk/llm/generateContent` using a
//! protobuf-JSON/SSE dialect from `@ali/accio-adk-ts`.  This adapter translates
//! that dialect to OpenAI Chat Completions and streams the result back in the
//! shape Accio's local agent expects.

use axum::{
    body::Body,
    extract::{Request, State},
    http::{header, HeaderMap, StatusCode},
    response::Response,
    Json,
};
use bytes::Bytes;
use futures::{Stream, StreamExt};
use serde_json::{json, Map, Value};

use super::{server::ProxyState, ProxyError};

const ACCIO_DEFAULT_CONTEXT_WINDOW: u64 = 131_072;
const ACCIO_DEEPSEEK_V4_LEGACY_CONTEXT_WINDOW: u64 = 131_072;
const ACCIO_DEEPSEEK_V4_CONTEXT_WINDOW: u64 = 1_000_000;
const ACCIO_OFFICIAL_GATEWAY: &str = "https://phoenix-gw.alibaba.com";
const ACCIO_PROXY_PREFIX: &str = "/accio";

fn request_text(value: Option<&Value>) -> String {
    value
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string()
}

fn deepseek_models() -> Vec<Value> {
    vec![
        json!({
            "id": "deepseek-chat",
            "name": "DeepSeek Chat",
            "reasoningEfforts": [],
            "contextWindow": 65_536,
        }),
        json!({
            "id": "deepseek-v4-flash",
            "name": "DeepSeek V4 Flash",
            "reasoningEfforts": ["low", "medium", "high"],
            "defaultReasoningEffort": "medium",
            "contextWindow": ACCIO_DEEPSEEK_V4_CONTEXT_WINDOW,
        }),
        json!({
            "id": "deepseek-v4-pro",
            "name": "DeepSeek V4 Pro",
            "reasoningEfforts": ["low", "medium", "high"],
            "defaultReasoningEffort": "high",
            "contextWindow": ACCIO_DEEPSEEK_V4_CONTEXT_WINDOW,
        }),
    ]
}

fn normalize_accio_context_window(id: &str, context_window: u64) -> u64 {
    let normalized_id = id.to_ascii_lowercase();
    if context_window == ACCIO_DEEPSEEK_V4_LEGACY_CONTEXT_WINDOW
        && matches!(
            normalized_id.as_str(),
            "deepseek-v4-flash" | "deepseek-v4-pro"
        )
    {
        return ACCIO_DEEPSEEK_V4_CONTEXT_WINDOW;
    }
    context_window
}

fn normalized_accio_models(provider: &Value) -> Vec<Value> {
    if let Some(raw_models) = provider.get("models").and_then(Value::as_array) {
        let models = raw_models
            .iter()
            .filter_map(|entry| {
                let id = request_text(entry.get("id").or_else(|| entry.get("model")));
                if id.is_empty() {
                    return None;
                }
                let name = request_text(entry.get("name").or_else(|| entry.get("display_name")));
                let display_name = if name.is_empty() { id.clone() } else { name };
                let reasoning_efforts = entry
                    .get("reasoning_efforts")
                    .or_else(|| entry.get("reasoningEfforts"))
                    .cloned()
                    .unwrap_or_else(|| json!([]));
                let default_reasoning_effort = entry
                    .get("default_reasoning_effort")
                    .or_else(|| entry.get("defaultReasoningEffort"))
                    .cloned()
                    .unwrap_or(Value::Null);
                let context_window = entry
                    .get("context_window")
                    .or_else(|| entry.get("contextWindow"))
                    .and_then(Value::as_u64)
                    .unwrap_or(ACCIO_DEFAULT_CONTEXT_WINDOW);
                let context_window = normalize_accio_context_window(&id, context_window);

                Some(json!({
                    "id": id,
                    "name": display_name,
                    "reasoningEfforts": reasoning_efforts,
                    "defaultReasoningEffort": default_reasoning_effort,
                    "contextWindow": context_window,
                }))
            })
            .collect::<Vec<_>>();
        if !models.is_empty() {
            return models;
        }
    }

    if request_text(provider.get("name")).eq_ignore_ascii_case("deepseek") {
        return deepseek_models();
    }

    let model = request_text(provider.get("model"));
    if model.is_empty() {
        return Vec::new();
    }
    vec![json!({
        "id": model,
        "name": model,
        "reasoningEfforts": [],
        "defaultReasoningEffort": Value::Null,
        "contextWindow": ACCIO_DEFAULT_CONTEXT_WINDOW,
    })]
}

fn model_matches(value: &Value, requested: &str) -> bool {
    if requested.is_empty() {
        return false;
    }
    request_text(value.get("id")).eq_ignore_ascii_case(requested)
        || request_text(value.get("name")).eq_ignore_ascii_case(requested)
}

fn accio_providers(state: &ProxyState) -> Result<Map<String, Value>, ProxyError> {
    let mut providers = crate::accio_config::get_providers()
        .map_err(|error| ProxyError::ConfigError(error.to_string()))?;
    if providers.is_empty() {
        providers = state
            .db
            .get_all_providers("accio")
            .map_err(|error| ProxyError::DatabaseError(error.to_string()))?
            .into_iter()
            .map(|(id, provider)| (id, provider.settings_config))
            .collect();
    }
    Ok(providers)
}

fn select_accio_provider(state: &ProxyState, body: &Value) -> Result<Value, ProxyError> {
    let providers = accio_providers(state)?;
    let request_model = request_text(body.get("model"));
    let request_tenant = request_text(body.get("tenant"));

    providers
        .values()
        .find(|provider| {
            let name = request_text(provider.get("name"));
            let model = request_text(provider.get("model"));
            let supports_requested_model = normalized_accio_models(provider)
                .iter()
                .any(|entry| model_matches(entry, &request_model));
            (!request_model.is_empty()
                && (supports_requested_model
                    || model == request_model
                    || name == request_model
                    || name == request_tenant))
                || (!request_tenant.is_empty() && name == request_tenant)
        })
        .or_else(|| providers.values().next())
        .cloned()
        .ok_or(ProxyError::NoProvidersConfigured)
}

fn provider_base_url(provider: &Value) -> String {
    request_text(provider.get("base_url"))
        .trim_end_matches('/')
        .to_string()
}

fn provider_api_key(provider: &Value) -> String {
    request_text(provider.get("api_key"))
}

fn provider_model(provider: &Value, requested: &str) -> String {
    let models = normalized_accio_models(provider);
    if models.iter().any(|entry| model_matches(entry, requested)) {
        return requested.to_string();
    }

    let configured = request_text(provider.get("model"));
    if configured.is_empty() {
        models
            .first()
            .and_then(|entry| entry.get("id"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| request_text(provider.get("name")))
    } else {
        configured
    }
}

fn provider_model_reasoning_effort(provider: &Value, model: &str) -> Option<String> {
    normalized_accio_models(provider)
        .iter()
        .find(|entry| model_matches(entry, model))
        .and_then(|entry| entry.get("defaultReasoningEffort"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty() && !value.eq_ignore_ascii_case("default"))
        .map(str::to_string)
}

fn role_for_openai(role: &str) -> &'static str {
    match role {
        "model" | "assistant" => "assistant",
        "tool" | "function_response" => "tool",
        _ => "user",
    }
}

fn tool_response_content(response_json: &str) -> Value {
    match serde_json::from_str::<Value>(response_json) {
        Ok(value) => value
            .get("content")
            .or_else(|| value.get("response"))
            .cloned()
            .unwrap_or(value),
        Err(_) => Value::String(response_json.to_string()),
    }
}

fn convert_accio_contents(body: &Value) -> Vec<Value> {
    let mut messages = Vec::new();

    if let Some(system) = body
        .get("system_instruction")
        .or_else(|| body.get("systemInstruction"))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
    {
        messages.push(json!({
            "role": "system",
            "content": system,
        }));
    }

    let Some(contents) = body.get("contents").and_then(Value::as_array) else {
        return messages;
    };

    for content in contents {
        let role = role_for_openai(
            content
                .get("role")
                .and_then(Value::as_str)
                .unwrap_or("user"),
        );
        let Some(parts) = content.get("parts").and_then(Value::as_array) else {
            continue;
        };

        for part in parts {
            if let Some(text) = part.get("text").and_then(Value::as_str) {
                if !text.is_empty() {
                    messages.push(json!({ "role": role, "content": text }));
                }
                continue;
            }

            if let Some(call) = part.get("functionCall").or_else(|| part.get("function_call")) {
                let id = request_text(call.get("id"));
                let name = request_text(call.get("name"));
                let arguments = call
                    .get("argsJson")
                    .or_else(|| call.get("args_json"))
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .or_else(|| {
                        call.get("args").and_then(|args| {
                            if let Some(args) = args.as_str() {
                                Some(args.to_string())
                            } else {
                                serde_json::to_string(args).ok()
                            }
                        })
                    })
                    .unwrap_or_default();
                messages.push(json!({
                    "role": "assistant",
                    "content": Value::Null,
                    "tool_calls": [{
                        "id": if id.is_empty() { format!("call_{}", messages.len()) } else { id },
                        "type": "function",
                        "function": { "name": name, "arguments": arguments },
                    }],
                }));
                continue;
            }

            if let Some(response) = part
                .get("functionResponse")
                .or_else(|| part.get("function_response"))
            {
                let id = request_text(response.get("id"));
                let content = response
                    .get("responseJson")
                    .or_else(|| response.get("response_json"))
                    .and_then(Value::as_str)
                    .map(tool_response_content)
                    .unwrap_or_else(|| response.get("response").cloned().unwrap_or(Value::Null));
                messages.push(json!({
                    "role": "tool",
                    "tool_call_id": if id.is_empty() { format!("call_{}", messages.len()) } else { id },
                    "content": content,
                }));
            }
        }
    }

    messages
}

fn convert_accio_tools(body: &Value) -> Vec<Value> {
    let Some(tools) = body.get("tools").and_then(Value::as_array) else {
        return Vec::new();
    };

    tools
        .iter()
        .filter_map(|tool| {
            let name = request_text(tool.get("name"));
            if name.is_empty() {
                return None;
            }

            let mut function = Map::new();
            function.insert("name".to_string(), Value::String(name));
            if let Some(description) = tool.get("description").and_then(Value::as_str) {
                function.insert(
                    "description".to_string(),
                    Value::String(description.to_string()),
                );
            }

            if let Some(raw_parameters) = tool
                .get("parametersJson")
                .or_else(|| tool.get("parameters_json"))
                .and_then(Value::as_str)
            {
                if let Ok(parameters) = serde_json::from_str::<Value>(raw_parameters) {
                    function.insert("parameters".to_string(), parameters);
                }
            } else if let Some(parameters) = tool.get("parameters") {
                function.insert("parameters".to_string(), parameters.clone());
            }

            Some(json!({
                "type": "function",
                "function": Value::Object(function),
            }))
        })
        .collect()
}

fn build_openai_request_body(body: &Value, provider: &Value) -> Value {
    let requested_model = request_text(body.get("model"));
    let model = provider_model(provider, &requested_model);
    let mut openai = json!({
        "model": model,
        "messages": convert_accio_contents(body),
        "stream": true,
        "stream_options": { "include_usage": true },
    });

    let obj = openai.as_object_mut().expect("json object");
    for (source, target) in [
        ("temperature", "temperature"),
        ("max_output_tokens", "max_tokens"),
        ("maxOutputTokens", "max_tokens"),
        ("top_p", "top_p"),
        ("topP", "top_p"),
    ] {
        if let Some(value) = body.get(source) {
            if !value.is_null() {
                obj.insert(target.to_string(), value.clone());
            }
        }
    }

    let requested_reasoning_effort = body
        .get("reasoning_effort")
        .or_else(|| body.get("reasoningEffort"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let reasoning_effort = if matches!(requested_reasoning_effort, Some("default")) {
        provider_model_reasoning_effort(provider, &model)
    } else {
        requested_reasoning_effort
            .map(str::to_string)
            .or_else(|| provider_model_reasoning_effort(provider, &model))
    };
    if let Some(reasoning_effort) = reasoning_effort {
        obj.insert(
            "reasoning_effort".to_string(),
            Value::String(reasoning_effort),
        );
    }

    if let Some(stop_sequences) = body
        .get("stop_sequences")
        .or_else(|| body.get("stopSequences"))
    {
        if stop_sequences.is_array() {
            obj.insert("stop".to_string(), stop_sequences.clone());
        }
    }

    let tools = convert_accio_tools(body);
    if !tools.is_empty() {
        obj.insert("tools".to_string(), Value::Array(tools));
        if let Some(tool_choice) = body.get("tool_choice").or_else(|| body.get("toolChoice")) {
            if !tool_choice.is_null() {
                obj.insert("tool_choice".to_string(), tool_choice.clone());
            }
        }
    }

    openai
}

fn finish_reason_for_adk(reason: &str) -> String {
    match reason {
        "length" | "max_tokens" => "MAX_TOKENS".to_string(),
        "content_filter" | "safety" => "SAFETY".to_string(),
        "tool_calls" | "function_call" | "stop" | "end_turn" | "stop_sequence" => {
            "STOP".to_string()
        }
        other => other.to_ascii_uppercase(),
    }
}

fn adk_frame(parts: Vec<Value>, finish_reason: Option<&str>, usage: Option<&Value>) -> Value {
    let turn_complete = finish_reason.is_some();
    let mut frame = json!({
        "content": {
            "role": "model",
            "parts": parts,
        },
        "turnComplete": turn_complete,
        "partial": !turn_complete,
    });
    if let Some(reason) = finish_reason {
        frame["finishReason"] = Value::String(finish_reason_for_adk(reason));
    }
    if let Some(usage) = usage {
        frame["usageMetadata"] = json!({
            "promptTokenCount": usage.get("prompt_tokens").unwrap_or(&Value::Null),
            "candidatesTokenCount": usage.get("completion_tokens").unwrap_or(&Value::Null),
            "totalTokenCount": usage.get("total_tokens").unwrap_or(&Value::Null),
        });
    }
    frame
}

fn sse_block(value: &Value) -> Bytes {
    Bytes::from(format!("data: {}\n\n", value))
}

fn parse_openai_sse_object(line: &str) -> Option<Value> {
    let data = line.strip_prefix("data:")?.trim();
    if data.is_empty() || data == "[DONE]" {
        return None;
    }
    serde_json::from_str(data).ok()
}

#[derive(Default)]
struct OpenAiToolCall {
    id: String,
    name: String,
    arguments: String,
}

fn adk_stream(upstream: reqwest::Response) -> impl Stream<Item = Result<Bytes, std::io::Error>> {
    async_stream::stream! {
        let start = std::time::Instant::now();
        let mut reasoning_chars = 0usize;
        let mut stream = upstream.bytes_stream();
        let mut buffer = String::new();
        let mut tool_calls = std::collections::HashMap::<u64, OpenAiToolCall>::new();
        let mut reasoning_buffer = String::new();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|error| std::io::Error::other(error.to_string()))?;
            buffer.push_str(&String::from_utf8_lossy(&chunk));
            buffer = buffer.replace("\r\n", "\n");

            while let Some(block_end) = buffer.find("\n\n") {
                let block = buffer[..block_end].to_string();
                buffer.drain(..=block_end + 1);

                let Some(object) = block.lines().find_map(parse_openai_sse_object) else {
                    continue;
                };
                let Some(choices) = object.get("choices").and_then(Value::as_array) else {
                    continue;
                };
                let Some(choice) = choices.first() else {
                    continue;
                };
                let delta = choice.get("delta").unwrap_or(&Value::Null);
                let finish_reason = choice
                    .get("finish_reason")
                    .or_else(|| choice.get("finishReason"))
                    .and_then(Value::as_str);

                let mut parts = Vec::new();
                if let Some(text) = delta.get("content").and_then(Value::as_str) {
                    if !text.is_empty() {
                        if !reasoning_buffer.is_empty() {
                            parts.push(json!({
                                "thought": true,
                                "text": std::mem::take(&mut reasoning_buffer),
                            }));
                        }
                        parts.push(json!({ "text": text }));
                    }
                }
                if let Some(reasoning) = delta.get("reasoning_content").and_then(Value::as_str) {
                    if !reasoning.is_empty() {
                        reasoning_chars += reasoning.chars().count();
                        reasoning_buffer.push_str(reasoning);
                    }
                }

                if let Some(raw_tool_calls) = delta.get("tool_calls").and_then(Value::as_array) {
                    if !raw_tool_calls.is_empty() && !reasoning_buffer.is_empty() {
                        parts.push(json!({
                            "thought": true,
                            "text": std::mem::take(&mut reasoning_buffer),
                        }));
                    }
                    for raw_call in raw_tool_calls {
                        let index = raw_call
                            .get("index")
                            .and_then(Value::as_u64)
                            .unwrap_or(tool_calls.len() as u64);
                        let entry = tool_calls.entry(index).or_default();
                        if let Some(id) = raw_call.get("id").and_then(Value::as_str) {
                            entry.id = id.to_string();
                        }
                        if let Some(function) = raw_call.get("function") {
                            if let Some(name) = function.get("name").and_then(Value::as_str) {
                                entry.name = name.to_string();
                            }
                            if let Some(arguments) = function.get("arguments").and_then(Value::as_str) {
                                entry.arguments.push_str(arguments);
                            }
                        }
                    }
                }

                if finish_reason.is_some() {
                    if !reasoning_buffer.is_empty() {
                        parts.push(json!({
                            "thought": true,
                            "text": std::mem::take(&mut reasoning_buffer),
                        }));
                    }
                    let mut ordered_indices = tool_calls.keys().copied().collect::<Vec<_>>();
                    ordered_indices.sort_unstable();
                    for index in ordered_indices {
                        let call = &tool_calls[&index];
                        log::info!(
                            "[Accio] emit functionCall id={} name={} args={}",
                            call.id,
                            call.name,
                            call.arguments
                        );
                        parts.push(json!({
                            "functionCall": {
                                "id": call.id,
                                "name": call.name,
                                "argsJson": call.arguments,
                            },
                        }));
                    }
                    tool_calls.clear();
                }

                if !parts.is_empty() || finish_reason.is_some() {
                    let usage = object.get("usage").filter(|usage| !usage.is_null());
                    yield Ok(sse_block(&adk_frame(parts, finish_reason, usage)));
                }
            }
        }

        if !reasoning_buffer.is_empty() {
            yield Ok(sse_block(&adk_frame(
                vec![json!({ "thought": true, "text": std::mem::take(&mut reasoning_buffer) })],
                None,
                None,
            )));
        }
        if !tool_calls.is_empty() {
            let mut parts = Vec::new();
            let mut ordered_indices = tool_calls.keys().copied().collect::<Vec<_>>();
            ordered_indices.sort_unstable();
            for index in ordered_indices {
                let call = &tool_calls[&index];
                parts.push(json!({
                    "functionCall": {
                        "id": call.id,
                        "name": call.name,
                        "argsJson": call.arguments,
                    },
                }));
            }
            yield Ok(sse_block(&adk_frame(parts, Some("STOP"), None)));
        }
        log::info!(
            "[Accio] stream done, reasoning_chars={reasoning_chars}, elapsed={:?}",
            start.elapsed()
        );
        yield Ok(bytes::Bytes::from_static(b"data: [DONE]\n\n"));
    }
}

pub async fn handle_adk_llm(
    State(state): State<ProxyState>,
    request: Request,
) -> Result<Response, ProxyError> {
    let (parts, body) = request.into_parts();
    let body_bytes = axum::body::to_bytes(body, usize::MAX)
        .await
        .map_err(|error| ProxyError::Internal(format!("failed to read Accio ADK body: {error}")))?;
    let body: Value = serde_json::from_slice(&body_bytes)
        .map_err(|error| ProxyError::InvalidRequest(format!("invalid Accio ADK JSON: {error}")))?;

    if let Some(tools) = body.get("tools").and_then(Value::as_array) {
        log::info!("[Accio] request tools={}", tools.len());
    }
    if let Some(contents) = body.get("contents").and_then(Value::as_array) {
        let mut part_keys = std::collections::HashSet::new();
        let mut fc = 0usize;
        let mut fr = 0usize;
        for content in contents {
            if let Some(parts) = content.get("parts").and_then(Value::as_array) {
                for part in parts {
                    if let Some(obj) = part.as_object() {
                        for key in obj.keys() {
                            part_keys.insert(key.clone());
                        }
                    }
                    if part.get("functionCall").is_some() {
                        fc += 1;
                    }
                    if part.get("functionResponse").is_some() {
                        fr += 1;
                    }
                }
            }
        }
        log::info!(
            "[Accio] request part_keys={:?} functionCall={fc} functionResponse={fr}",
            part_keys
        );
    }

    let provider = select_accio_provider(&state, &body)?;
    log::info!(
        "[Accio] provider={} request_model={}",
        request_text(provider.get("name")),
        request_text(body.get("model")),
    );
    let base_url = provider_base_url(&provider);
    if base_url.is_empty() {
        return Err(ProxyError::ConfigError(
            "Accio provider is missing base_url".to_string(),
        ));
    }

    let api_key = provider_api_key(&provider);
    let upstream_url = if base_url.ends_with("/chat/completions") {
        base_url
    } else {
        format!("{base_url}/chat/completions")
    };

    let openai_body = build_openai_request_body(&body, &provider);

    // Nvidia 等免费端点会按请求频率限流（429）。多轮对话会连续发多个
    // 请求，容易撞上限流窗口，这里对 429 做一次退避重试。
    let mut attempt = 0u32;
    let upstream = loop {
        attempt += 1;
        let mut request_builder = super::http_client::get()
            .post(&upstream_url)
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::ACCEPT, "text/event-stream")
            .header(header::ACCEPT_ENCODING, "identity")
            .body(openai_body.to_string());
        if !api_key.is_empty() {
            request_builder = request_builder.bearer_auth(api_key.as_str());
        }

        let upstream = request_builder
            .send()
            .await
            .map_err(|error| ProxyError::ForwardFailed(error.to_string()))?;
        let status = upstream.status();
        log::info!("[Accio] upstream status={status} url={upstream_url}");
        if status == StatusCode::TOO_MANY_REQUESTS && attempt < 2 {
            log::warn!(
                "[Accio] upstream 429 rate-limited, retrying in 10s (attempt {attempt})"
            );
            tokio::time::sleep(std::time::Duration::from_secs(10)).await;
            continue;
        }
        break upstream;
    };

    let status = upstream.status();
    if !status.is_success() {
        let text = upstream.text().await.unwrap_or_default();
        return Err(ProxyError::UpstreamError {
            status: status.as_u16(),
            body: Some(text),
        });
    }

    let stream = adk_stream(upstream);
    let response = axum::response::Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream")
        .header(header::CACHE_CONTROL, "no-cache")
        .body(Body::from_stream(stream))
        .map_err(|error| ProxyError::Internal(error.to_string()))?;

    let _ = parts;
    Ok(response)
}

fn model_catalog_response(providers: &serde_json::Map<String, Value>) -> Value {
    let data = providers
        .values()
        .filter_map(|provider| {
            let name = request_text(provider.get("name"));
            if name.is_empty() {
                return None;
            }
            let models = normalized_accio_models(provider);
            if models.is_empty() {
                return None;
            }
            let default_model = request_text(provider.get("model"));
            let default_model = if models
                .iter()
                .any(|entry| model_matches(entry, &default_model))
            {
                default_model
            } else {
                request_text(models.first().and_then(|entry| entry.get("id")))
            };
            let model_list = models
                .into_iter()
                .map(|entry| {
                    let model_code = request_text(entry.get("id"));
                    let display_name = request_text(entry.get("name"));
                    json!({
                        "modelDisplayName": display_name,
                        "modelCode": model_code,
                        "modelName": model_code,
                        "multimodal": true,
                        "visible": true,
                        "reasoningEfforts": entry.get("reasoningEfforts").cloned().unwrap_or_else(|| json!([])),
                        "defaultReasoningEffort": entry.get("defaultReasoningEffort").cloned().unwrap_or(Value::Null),
                        "contextWindow": entry.get("contextWindow").cloned().unwrap_or_else(|| json!(ACCIO_DEFAULT_CONTEXT_WINDOW)),
                        "isDefault": model_code == default_model,
                        "modelDesc": "CC Switch managed provider",
                        "hoverDesc": "CC Switch managed provider",
                        "usageLevel": 0,
                        "usageDesc": "External",
                        "freeUse": true,
                        "usageMultiple": 1.0,
                        "effectiveMultiple": 1.0,
                        "discountList": Value::Null,
                        "locked": Value::Null,
                    })
                })
                .collect::<Vec<_>>();

            Some(json!({
                "provider": name,
                "providerDisplayName": name,
                "modelList": model_list,
            }))
        })
        .collect::<Vec<_>>();

    json!({
        "success": true,
        "code": "200",
        "message": "0",
        "data": data,
    })
}

pub async fn handle_accio_models(
    State(state): State<ProxyState>,
) -> Result<Json<Value>, ProxyError> {
    let providers = accio_providers(&state)?;
    Ok(Json(model_catalog_response(&providers)))
}

const HOP_BY_HOP_HEADERS: &[&str] = &[
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
    "host",
    "content-length",
];

fn is_hop_by_hop(name: &str) -> bool {
    HOP_BY_HOP_HEADERS
        .iter()
        .any(|header| name.eq_ignore_ascii_case(header))
}

fn copy_forward_headers(source: &HeaderMap) -> HeaderMap {
    let mut headers = HeaderMap::new();
    for (name, value) in source.iter() {
        if !is_hop_by_hop(name.as_str()) {
            headers.append(name.clone(), value.clone());
        }
    }
    headers
}

pub async fn handle_accio_fallback(
    State(_state): State<ProxyState>,
    request: Request,
) -> Result<Response, ProxyError> {
    let (parts, body) = request.into_parts();
    let uri = parts.uri.clone();
    let path_and_query = uri
        .path_and_query()
        .map(|value| value.as_str())
        .unwrap_or("/");
    let path_and_query = path_and_query
        .strip_prefix(ACCIO_PROXY_PREFIX)
        .unwrap_or(path_and_query);
    let target = format!("{ACCIO_OFFICIAL_GATEWAY}{path_and_query}");

    let body_bytes = axum::body::to_bytes(body, usize::MAX)
        .await
        .map_err(|error| {
            ProxyError::Internal(format!("failed to read Accio fallback body: {error}"))
        })?;

    let mut builder = super::http_client::get().request(parts.method.clone(), target);
    for (name, value) in copy_forward_headers(&parts.headers).iter() {
        builder = builder.header(name, value);
    }
    builder = builder
        .header(header::ACCEPT_ENCODING, "identity")
        .body(body_bytes);

    let upstream = builder
        .send()
        .await
        .map_err(|error| ProxyError::ForwardFailed(error.to_string()))?;
    let status = upstream.status();
    let mut response_headers = HeaderMap::new();
    for (name, value) in upstream.headers() {
        if !is_hop_by_hop(name.as_str()) {
            response_headers.append(name.clone(), value.clone());
        }
    }

    let stream = upstream
        .bytes_stream()
        .map(|item| item.map_err(|error| std::io::Error::other(error.to_string())));
    let mut response = Response::builder()
        .status(status)
        .body(Body::from_stream(stream))
        .map_err(|error| ProxyError::Internal(error.to_string()))?;
    *response.headers_mut() = response_headers;
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_text_and_function_parts_to_openai_messages() {
        let body = json!({
            "system_instruction": "be concise",
            "contents": [
                {"role": "user", "parts": [{"text": "hello"}]},
                {"role": "model", "parts": [{
                    "functionCall": {"id": "call-1", "name": "weather", "args": "{\"city\":\"Shanghai\"}"}
                }]},
                {"role": "tool", "parts": [{
                    "functionResponse": {"id": "call-1", "responseJson": "{\"content\":\"sunny\"}"}
                }]}
            ]
        });

        let messages = convert_accio_contents(&body);
        assert_eq!(messages.len(), 4);
        assert_eq!(messages[0]["role"], "system");
        assert_eq!(messages[1]["role"], "user");
        assert_eq!(messages[2]["role"], "assistant");
        assert_eq!(messages[2]["tool_calls"][0]["function"]["name"], "weather");
        assert_eq!(messages[3]["role"], "tool");
        assert_eq!(messages[3]["content"], "sunny");
    }

    #[test]
    fn builds_openai_stream_request() {
        let provider = json!({
            "name": "deepseek",
            "base_url": "https://api.deepseek.com",
            "api_key": "sk-test",
            "model": "deepseek-chat"
        });
        let body = json!({
            "model": "deepseek-chat",
            "contents": [{"role": "user", "parts": [{"text": "hello"}]}],
            "maxOutputTokens": 256,
            "reasoningEffort": "medium",
        });

        let openai = build_openai_request_body(&body, &provider);
        assert_eq!(openai["model"], "deepseek-chat");
        assert_eq!(openai["stream"], true);
        assert_eq!(openai["max_tokens"], 256);
        assert_eq!(openai["reasoning_effort"], "medium");
        assert_eq!(openai["messages"][0]["content"], "hello");
    }

    #[test]
    fn model_catalog_contains_deepseek() {
        let mut providers = serde_json::Map::new();
        providers.insert(
            "deepseek".to_string(),
            json!({
                "name": "deepseek",
                "base_url": "https://api.deepseek.com",
                "api_key": "",
                "model": "deepseek-chat"
            }),
        );

        let response = model_catalog_response(&providers);
        let data = &response["data"];
        assert_eq!(data[0]["modelList"].as_array().map(Vec::len), Some(3));
        assert_eq!(data[0]["modelList"][0]["modelCode"], "deepseek-chat");
        assert_eq!(data[0]["modelList"][0]["freeUse"], true);
        assert_eq!(data[0]["modelList"][1]["modelCode"], "deepseek-v4-flash");
        assert_eq!(data[0]["modelList"][1]["reasoningEfforts"][0], "low");
        assert_eq!(data[0]["modelList"][1]["defaultReasoningEffort"], "medium");
        assert_eq!(
            data[0]["modelList"][1]["contextWindow"],
            ACCIO_DEEPSEEK_V4_CONTEXT_WINDOW
        );
        assert_eq!(
            data[0]["modelList"][2]["contextWindow"],
            ACCIO_DEEPSEEK_V4_CONTEXT_WINDOW
        );
    }

    #[test]
    fn model_catalog_upgrades_legacy_deepseek_v4_context_window() {
        let mut providers = serde_json::Map::new();
        providers.insert(
            "deepseek".to_string(),
            json!({
                "name": "deepseek",
                "models": [
                    {
                        "id": "deepseek-v4-flash",
                        "name": "DeepSeek V4 Flash",
                        "context_window": 131_072
                    },
                    {
                        "id": "deepseek-v4-pro",
                        "name": "DeepSeek V4 Pro",
                        "contextWindow": 131_072
                    }
                ]
            }),
        );

        let response = model_catalog_response(&providers);
        let data = &response["data"];
        assert_eq!(
            data[0]["modelList"][0]["contextWindow"],
            ACCIO_DEEPSEEK_V4_CONTEXT_WINDOW
        );
        assert_eq!(
            data[0]["modelList"][1]["contextWindow"],
            ACCIO_DEEPSEEK_V4_CONTEXT_WINDOW
        );
    }

    #[test]
    fn provider_model_uses_requested_model_from_configured_model_list() {
        let provider = json!({
            "name": "deepseek",
            "base_url": "https://api.deepseek.com",
            "api_key": "sk-test",
            "model": "deepseek-chat",
            "models": [
                {"id": "deepseek-chat", "name": "DeepSeek Chat"},
                {
                    "id": "deepseek-v4-flash",
                    "name": "DeepSeek V4 Flash",
                    "reasoning_efforts": ["low", "medium", "high"],
                    "default_reasoning_effort": "medium",
                }
            ]
        });

        assert_eq!(
            provider_model(&provider, "deepseek-v4-flash"),
            "deepseek-v4-flash"
        );
        assert_eq!(provider_model(&provider, "unknown"), "deepseek-chat");
        assert_eq!(provider_model(&provider, ""), "deepseek-chat");
    }

    #[test]
    fn provider_default_reasoning_effort_fills_accents_default_request() {
        let provider = json!({
            "name": "deepseek",
            "base_url": "https://api.deepseek.com",
            "api_key": "sk-test",
            "model": "deepseek-v4-flash",
            "models": [{
                "id": "deepseek-v4-flash",
                "name": "DeepSeek V4 Flash",
                "reasoning_efforts": ["low", "medium", "high"],
                "default_reasoning_effort": "medium",
            }]
        });

        let without_requested_effort = build_openai_request_body(
            &json!({
                "model": "deepseek-v4-flash",
                "contents": [{"role": "user", "parts": [{"text": "hello"}]}],
            }),
            &provider,
        );
        assert_eq!(without_requested_effort["reasoning_effort"], "medium");

        let with_default = build_openai_request_body(
            &json!({
                "model": "deepseek-v4-flash",
                "reasoningEffort": "default",
                "contents": [{"role": "user", "parts": [{"text": "hello"}]}],
            }),
            &provider,
        );
        assert_eq!(with_default["reasoning_effort"], "medium");
    }

    #[test]
    fn emits_top_level_adk_frames_for_text_chunks() {
        let frame = adk_frame(vec![json!({"text": "HELLO"})], None, None);
        assert_eq!(frame["content"]["role"], "model");
        assert_eq!(frame["content"]["parts"][0]["text"], "HELLO");
        assert_eq!(frame["turnComplete"], false);
        assert_eq!(frame["partial"], true);
        assert!(frame.get("candidates").is_none());

        let final_frame = adk_frame(
            vec![json!({"text": "DONE"})],
            Some("stop"),
            Some(&json!({
                "prompt_tokens": 5,
                "completion_tokens": 3,
                "total_tokens": 8,
            })),
        );
        assert_eq!(final_frame["turnComplete"], true);
        assert_eq!(final_frame["partial"], false);
        assert_eq!(final_frame["finishReason"], "STOP");
        assert_eq!(final_frame["usageMetadata"]["totalTokenCount"], 8);
        assert!(final_frame.get("candidates").is_none());
    }
}
