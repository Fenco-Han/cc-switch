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
    response::{IntoResponse, Response},
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

/// Accio 传输层对 SSE 流有约 90s 的空闲超时（`SSE stream idle timeout`）。
/// DeepSeek 思考模型在上下文很大时可能 200 后长时间不出首 token，上游静默
/// 期间代理不发任何字节，Accio 会在 90s 掐断整轮导致「网络中断，结果未传回」。
/// 缓解策略：
///   1. 上游静默达到 `ACCIO_KEEPALIVE_INTERVAL` 时，向 Accio 发 SSE 注释帧
///      （`: keepalive`）重置其空闲计时器——上游慢但还活着时不再被误杀；
///   2. 上游静默达到 `ACCIO_UPSTREAM_STALL_RETRY_AFTER` 时，主动断开并重新
///      发送同一请求（最多 `ACCIO_STALL_MAX_ATTEMPTS` 次），把「必然失败」
///      变成「重试后大概率成功」。
const ACCIO_KEEPALIVE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);
const ACCIO_UPSTREAM_STALL_RETRY_AFTER: std::time::Duration = std::time::Duration::from_secs(120);
const ACCIO_STALL_MAX_ATTEMPTS: u32 = 2;

fn request_text(value: Option<&Value>) -> String {
    value
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string()
}

/// 把上游(海康 MaaS 等)的错误响应翻译成对 Accio 友好的中文错误 body。
///
/// 海康返回的错误体结构多样(OpenAI 风格 `{"error":{"message":...}}` 或
/// `{"detail":{"error":{"message":...}}}`)，Accio 前端往往只显示到状态码
/// 就丢了详情(例如只显示「网络请求失败 (400)」)。这里先把原始 message 提取
/// 出来，再根据【原始错误内容 + 状态码】映射成清晰的中文说明，统一回成标准的
/// OpenAI 错误结构 `{"error":{"message":...,"type":"upstream_error","code":...}}`，
/// 让 Accio 页面能把原因真正展示出来。
///
/// `hint` 是 Accio 侧生成的额外提示(如请求上下文太大)，优先拼在最前。
fn upstream_error_body(status: u16, body_text: &str, hint: &str) -> Value {
    // 从上游 body 里提取原始错误 message(兼容 OpenAI 与海康 detail 两种结构)。
    let mut raw_message = String::new();
    if let Ok(value) = serde_json::from_str::<Value>(body_text) {
        if let Some(msg) = value
            .pointer("/error/message")
            .and_then(Value::as_str)
            .map(str::to_string)
        {
            raw_message = msg;
        } else if let Some(msg) = value
            .pointer("/detail/error/message")
            .and_then(Value::as_str)
            .map(str::to_string)
        {
            raw_message = msg;
        }
    }
    if raw_message.is_empty() && !body_text.trim().is_empty() {
        raw_message = body_text.trim().to_string();
    }

    let friendly = friendly_accio_error(status, &raw_message);

    // 把原始信息也带上，方便排障；hint 放最前，让 Accio 先看到结论。
    let mut message = String::new();
    if !hint.is_empty() {
        message.push_str(hint);
        message.push_str("。");
    }
    message.push_str(&friendly);
    if !raw_message.is_empty() {
        message.push_str(&format!("（原始信息: {raw_message}）"));
    }

    json!({
        "error": {
            "message": message,
            "type": "upstream_error",
            "code": status,
        }
    })
}

/// 状态码 + 原始错误内容 → 友好中文错误描述(优先精确匹配原始内容)。
fn friendly_accio_error(status: u16, raw: &str) -> String {
    let raw_lower = raw.to_ascii_lowercase();
    // 按原始错误内容精确/关键词匹配(优先于状态码，因为同一状态码多种含义)。
    let by_message = [
        ("bad request", "请求无效"),
        ("parameter invalid", "参数错误"),
        ("api key is invalid", "API-KEY 无效，请联系空间管理员确认后修改"),
        ("request invalid", "请求校验失败"),
        ("forbidden", "请求拒绝，服务限流"),
        ("model service request has reached the limit", "模型服务请求已到上限"),
        ("the api-key does not match the workspace", "apikey 以及用户关联的工作空间不匹配"),
        ("not found", "请求的资源不存在"),
        ("too many requests", "请求过于频繁"),
        ("internal server error", "服务器内部错误"),
        ("redis read or write failure", "redis 读写失败"),
        ("internal error", "内部错误"),
        ("knowledge not be authorized", "知识库未授权操作，该操作仅创建人可执行"),
        ("maas-web-config service is incorrect", "maas-web-config 服务不正确"),
        ("maas-knowledge-manage service is incorrect", "maas-knowledge-manage 服务不正确"),
        ("no deployments available", "所选模型当前没有可用部署，请稍后重试"),
    ];
    for (key, zh) in by_message {
        if raw_lower.contains(key) {
            return zh.to_string();
        }
    }

    // 未匹配到内容则退化为按状态码的通用中文。
    match status {
        400 => "请求无效".to_string(),
        401 => "认证失败，请检查 API-KEY".to_string(),
        403 => "请求拒绝，服务限流（模型服务访问已超承载上限，请稍后重试）".to_string(),
        404 => "请求的资源不存在".to_string(),
        429 => "请求过于频繁（模型服务访问已超承载上限，请稍后重试）".to_string(),
        500 => "服务器内部错误（系统出现异常，请联系技术支持）".to_string(),
        502 => "上游网关错误".to_string(),
        503 => "上游服务不可用".to_string(),
        504 => "上游网关超时".to_string(),
        _ => format!("上游服务错误（HTTP {status}）"),
    }
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

/// 把工具返回的 responseJson（一般是 JSON 字符串）转成海康能接受的字符串 content。
/// 海康/litellm 要求 `role:"tool"` 消息的 content 必须是字符串；不能是 JSON 对象。
/// 若解析出的是对象/数组，则整体序列化为紧凑 JSON 字符串下发。
fn serialize_tool_response_content(response_json: &str) -> Value {
    match serde_json::from_str::<Value>(response_json) {
        Ok(Value::String(s)) => Value::String(s),
        Ok(value) => serialize_json_value_to_string(&value),
        Err(_) => Value::String(response_json.to_string()),
    }
}

/// 把任意 JSON Value 序列化为字符串（对象/数组/数字/布尔都转字符串）。
fn serialize_json_value_to_string(value: &Value) -> Value {
    match value {
        Value::String(s) => Value::String(s.clone()),
        other => Value::String(other.to_string()),
    }
}

/// 把一个 ADK `functionResponse` part 转成 OpenAI `role:"tool"` 消息并压入 messages。
///
/// Accio/ADK 并不保证 functionResponse 一定放在 `role:"tool"` 的 content 里——
/// 实测它可能和 functionCall 一起放在同一个 `role:"model"` content 中，也可能
/// 单独出现在 model/user content 里。因此转换时不能按 content 角色过滤，必须在
/// 任意位置识别 functionResponse 并生成 tool 消息，否则 assistant 的 tool_calls
/// 后面没有对应的 tool 消息，DeepSeek 会返回 400
/// 「An assistant message with 'tool_calls' must be followed by tool messages...」。
fn push_tool_response(
    messages: &mut Vec<Value>,
    response: &Value,
    tool_call_counter: &mut usize,
) {
    let id = request_text(response.get("id"));
    // 海康/litellm 要求 `role:"tool"` 消息的 content 必须是字符串。
    // 若工具返回的是结构化 JSON 对象，将其序列化为字符串再下发，
    // 否则海康会以 400 Bad Request 拒绝。
    let content = response
        .get("responseJson")
        .or_else(|| response.get("response_json"))
        .and_then(Value::as_str)
        .map(serialize_tool_response_content)
        .or_else(|| {
            if let Some(value) = response.get("response") {
                Some(serialize_json_value_to_string(value))
            } else {
                None
            }
        })
        .unwrap_or_else(|| Value::Null);
    messages.push(json!({
        "role": "tool",
        "tool_call_id": if id.is_empty() {
            format!("call_{tool_call_counter}")
        } else {
            id
        },
        "content": content,
    }));
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

    /// 把累积的 pending assistant（thought/text/tool_calls 可能分散在多个
    /// assistant content/part 中）刷成一条 OpenAI assistant 消息。
    ///
    /// Accio/ADK 会把一个 assistant 回合拆成多条 `thought` / `text` /
    /// `functionCall` part。DeepSeek 的 thinking 模式要求 assistant 消息在
    /// 携带 `reasoning_content`（思考内容）时，后续请求必须把它原样回传，
    /// 否则返回「The `reasoning_content` in the thinking mode must be passed
    /// back to the API」400 错误。这里把 `thought` part 映射为
    /// `reasoning_content`，与同回合的 `text`（content）和 `functionCall`
    /// （tool_calls）合并为一条 assistant 消息，避免子代理等场景下的 400。
    fn flush_assistant(
        messages: &mut Vec<Value>,
        reasoning: &mut String,
        text: &mut String,
        tool_calls: &mut Vec<Value>,
    ) {
        let has_reasoning = !reasoning.is_empty();
        let has_text = !text.is_empty();
        let has_tools = !tool_calls.is_empty();
        if !has_reasoning && !has_text && !has_tools {
            return;
        }

        let mut msg = Map::new();
        msg.insert("role".to_string(), Value::String("assistant".to_string()));
        // DeepSeek thinking 模式下 content 可为 null（纯思考 + 工具调用），
        // 但普通非思考文本应保留为字符串。
        msg.insert(
            "content".to_string(),
            if has_text {
                Value::String(std::mem::take(text))
            } else {
                Value::Null
            },
        );
        if has_reasoning {
            msg.insert(
                "reasoning_content".to_string(),
                Value::String(std::mem::take(reasoning)),
            );
        }
        if has_tools {
            msg.insert("tool_calls".to_string(), Value::Array(std::mem::take(tool_calls)));
        }
        messages.push(Value::Object(msg));
    }

    let mut pending_reasoning = String::new();
    let mut pending_text = String::new();
    let mut pending_tool_calls: Vec<Value> = Vec::new();
    let mut tool_call_counter = 0usize;

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

        if role == "assistant" {
            // 累积 assistant 回合：thought / text / functionCall 合并。
            for part in parts {
                // functionResponse 可能与 functionCall 同在一个 model content
                // 里（Accio 实测如此）：先把已累积的 assistant（含 tool_calls）
                // 刷出，再补对应的 tool 消息，保证 DeepSeek 收到
                // assistant(tool_calls) → tool 的正确序列。
                if let Some(response) = part
                    .get("functionResponse")
                    .or_else(|| part.get("function_response"))
                {
                    flush_assistant(
                        &mut messages,
                        &mut pending_reasoning,
                        &mut pending_text,
                        &mut pending_tool_calls,
                    );
                    push_tool_response(&mut messages, response, &mut tool_call_counter);
                    continue;
                }

                let is_thought = part
                    .get("thought")
                    .map(|v| v.as_bool().unwrap_or(false))
                    .unwrap_or(false)
                    || part.get("type").and_then(Value::as_str) == Some("thought");
                if is_thought {
                    if let Some(t) = part.get("text").and_then(Value::as_str) {
                        if !pending_reasoning.is_empty() {
                            pending_reasoning.push('\n');
                        }
                        pending_reasoning.push_str(t);
                    }
                    continue;
                }

                if let Some(text) = part.get("text").and_then(Value::as_str) {
                    if !text.is_empty() {
                        if !pending_text.is_empty() {
                            pending_text.push('\n');
                        }
                        pending_text.push_str(text);
                    }
                    continue;
                }

                if let Some(call) =
                    part.get("functionCall").or_else(|| part.get("function_call"))
                {
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
                    tool_call_counter += 1;
                    pending_tool_calls.push(json!({
                        "id": if id.is_empty() {
                            format!("call_{tool_call_counter}")
                        } else {
                            id
                        },
                        "type": "function",
                        "function": { "name": name, "arguments": arguments },
                    }));
                    continue;
                }
            }
            continue;
        }

        // role == "tool" | "user"：先刷新已累积的 assistant，再按 part 处理。
        flush_assistant(
            &mut messages,
            &mut pending_reasoning,
            &mut pending_text,
            &mut pending_tool_calls,
        );

        for part in parts {
            // functionResponse 也可能出现在 role="user"/"tool" 的 content 里
            //（Gemini 风格），统一在此识别并转成 tool 消息。
            if let Some(response) = part
                .get("functionResponse")
                .or_else(|| part.get("function_response"))
            {
                push_tool_response(&mut messages, response, &mut tool_call_counter);
                continue;
            }
            if let Some(text) = part.get("text").and_then(Value::as_str) {
                if !text.is_empty() {
                    messages.push(json!({ "role": role, "content": text }));
                }
            }
        }
    }

    // 收尾：flush 最后一段 assistant 累积。
    flush_assistant(
        &mut messages,
        &mut pending_reasoning,
        &mut pending_text,
        &mut pending_tool_calls,
    );

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

/// Normalize `\r\n` to `\n` on the raw SSE byte buffer so that both `\n\n` and
/// `\r\n\r\n` terminators collapse to the same `\n\n` block delimiter.
fn normalize_crlf(bytes: Vec<u8>) -> Vec<u8> {
    if !bytes.contains(&b'\r') {
        return bytes;
    }
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\r' && i + 1 < bytes.len() && bytes[i + 1] == b'\n' {
            out.push(b'\n');
            i += 2;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    out
}

/// Index of the first `\n\n` block delimiter, or `None`. Mirrors the previous
/// string-buffer `buffer.find("\n\n")` semantics on the raw byte buffer.
fn find_double_newline(bytes: &[u8]) -> Option<usize> {
    bytes.windows(2).position(|window| window == b"\n\n")
}

/// Extract the next complete `\n\n`-terminated SSE block from the raw byte
/// buffer (which is normalized to `\n` line endings) and decode it to UTF-8.
/// Returns `None` when no complete block is present yet. Because the block is
/// delimited by `\n\n`, any multi-byte UTF-8 character that was split across
/// incoming network chunks is fully reassembled here before decoding, so it is
/// never corrupted into U+FFFD.
fn drain_next_sse_block(buffer: &mut Vec<u8>) -> Option<String> {
    let block_end = find_double_newline(buffer)?;
    let block = String::from_utf8_lossy(&buffer[..block_end]).into_owned();
    buffer.drain(..block_end + 2);
    Some(block)
}

fn adk_stream(
    first_response: reqwest::Response,
    retry_request: impl Fn() -> reqwest::RequestBuilder,
) -> impl Stream<Item = Result<Bytes, std::io::Error>> {
    async_stream::stream! {
        let mut pending: Option<reqwest::Response> = Some(first_response);
        let mut attempt = 0u32;

        'attempts: loop {
            let upstream = match pending.take() {
                Some(response) => response,
                None => match retry_request().send().await {
                    Ok(response) => response,
                    Err(error) => {
                        log::error!("[Accio] retry send failed: {error}");
                        yield Err(std::io::Error::other(error.to_string()));
                        break 'attempts;
                    }
                },
            };
            let status = upstream.status();
            if status == StatusCode::TOO_MANY_REQUESTS && attempt + 1 < ACCIO_STALL_MAX_ATTEMPTS {
                log::warn!("[Accio] retry upstream 429, sleeping 10s (attempt {attempt})");
                tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                attempt += 1;
                continue;
            }
            if !status.is_success() {
                // 仅重试会出现（首次响应已在 handle_adk_llm 校验并转成中文错误体）。
                let text = upstream.text().await.unwrap_or_default();
                log::error!("[Accio] retry upstream status={status} body={text}");
                yield Ok(sse_block(&adk_frame(
                    vec![json!({"text": "上游模型服务请求失败，请稍后重试。"})],
                    Some("STOP"),
                    None,
                )));
                break 'attempts;
            }

            let start = std::time::Instant::now();
            let mut reasoning_chars = 0usize;
            let mut stream = upstream.bytes_stream();
            // Buffer raw bytes, never lossily-decoded per chunk. Decoding each
            // incoming chunk with `from_utf8_lossy` corrupts any multi-byte UTF-8
            // character (Chinese, emoji, …) that happens to straddle a network
            // chunk boundary — the incomplete trailing bytes become U+FFFD (乱码).
            // Only a complete `\n\n`-terminated SSE block is decoded to UTF-8, so a
            // character split across chunks is reassembled before decoding.
            let mut buffer: Vec<u8> = Vec::new();
            let mut tool_calls = std::collections::HashMap::<u64, OpenAiToolCall>::new();
            let mut reasoning_buffer = String::new();
            // 上游静默计时：只被上游数据重置；keepalive 帧不重置它。
            let mut last_activity = std::time::Instant::now();
            // 下一次向下游发 keepalive 的绝对时间。
            let mut next_keepalive_at = last_activity + ACCIO_KEEPALIVE_INTERVAL;

            'stream: loop {
                let now = std::time::Instant::now();
                if now.duration_since(last_activity) >= ACCIO_UPSTREAM_STALL_RETRY_AFTER {
                    break 'stream;
                }
                let wait = next_keepalive_at.saturating_duration_since(now);
                tokio::select! {
                    chunk = stream.next() => match chunk {
                        Some(Ok(chunk)) => {
                            last_activity = std::time::Instant::now();
                            next_keepalive_at = last_activity + ACCIO_KEEPALIVE_INTERVAL;
                            buffer.extend_from_slice(&chunk);
                            buffer = normalize_crlf(buffer);

                            while let Some(block) = drain_next_sse_block(&mut buffer) {
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
                        Some(Err(error)) => {
                            yield Err(std::io::Error::other(error.to_string()));
                            break 'attempts;
                        }
                        None => {
                            // 上游流正常结束：收尾 flush 残留的 reasoning / tool_calls。
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
                            break 'attempts;
                        }
                    },
                    _ = tokio::time::sleep(wait) => {
                        // 上游静默：到 keepalive 节点时向 Accio 发 SSE 注释帧，
                        // 重置其空闲计时器（90s），避免「上游慢但还活着」被掐断。
                        if std::time::Instant::now() >= next_keepalive_at {
                            yield Ok(bytes::Bytes::from_static(b": keepalive\n\n"));
                            next_keepalive_at = std::time::Instant::now() + ACCIO_KEEPALIVE_INTERVAL;
                        }
                    }
                }
            }

            // 只有「上游静默超时」会从 'stream 循环走到这里（正常结束 /
            // 流错误均已 break 'attempts）。重发请求或放弃。
            attempt += 1;
            if attempt >= ACCIO_STALL_MAX_ATTEMPTS {
                log::error!(
                    "[Accio] upstream idle >{ACCIO_UPSTREAM_STALL_RETRY_AFTER:?}, giving up after {attempt} attempts"
                );
                yield Ok(bytes::Bytes::from_static(b"data: [DONE]\n\n"));
                break 'attempts;
            }
            log::warn!(
                "[Accio] upstream idle >{ACCIO_UPSTREAM_STALL_RETRY_AFTER:?}, re-sending request (attempt {attempt})"
            );
        }
    }
}

pub async fn handle_adk_llm(
    State(state): State<ProxyState>,
    request: Request,
) -> Result<Response, ProxyError> {
    let (parts, body) = request.into_parts();
    let raw_body = axum::body::to_bytes(body, usize::MAX)
        .await
        .map_err(|error| ProxyError::Internal(format!("failed to read Accio ADK body: {error}")))?;
    // Accio 对较大的请求体（例如带 34 个工具、长上下文的 generateContent，实测 >~几十 KB）
    // 会带 `Content-Encoding: gzip` 发送。若不解压，下面的 serde_json 会因为拿到的是
    // 压缩字节而解析失败，表现为「本地代理直接报 invalid Accio ADK JSON」。这里复用
    // 与 Codex 相同的解压工具，按 content-encoding 解压后再解析。
    let body_bytes = if let Some(encoding) =
        super::content_encoding::get_content_encoding(&parts.headers)
    {
        if !super::content_encoding::is_supported_content_encoding(&encoding) {
            return Err(ProxyError::InvalidRequest(format!(
                "Unsupported request content-encoding: {encoding}"
            )));
        }
        log::debug!("[Accio] 解压请求体: content-encoding={encoding}");
        match super::content_encoding::decompress_body(&encoding, &raw_body) {
            Ok(Some(decompressed)) => Bytes::from(decompressed),
            Ok(None) => {
                return Err(ProxyError::InvalidRequest(format!(
                    "Unsupported request content-encoding: {encoding}"
                )));
            }
            Err(error) => {
                return Err(ProxyError::InvalidRequest(format!(
                    "Failed to decompress Accio request body ({encoding}): {error}"
                )));
            }
        }
    } else {
        raw_body
    };
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
        // 把上游错误翻译成 Accio 能展示的中文错误体返回，而不是只透传原始 body
        // (原始 body 是海康的 detail 结构，Accio 前端往往只显示到状态码就丢了详情)。
        log::error!(
            "[Accio] upstream status={status} url={upstream_url} body={text}"
        );
        // 排障辅助：把发往上游的 OpenAI 消息骨架打出来（role / tool_call ids /
        // 是否带 reasoning_content / content 长度），400 一类的结构问题一眼可辨。
        let skeleton: Vec<Value> = openai_body
            .get("messages")
            .and_then(Value::as_array)
            .map(|messages| {
                messages
                    .iter()
                    .map(|message| {
                        let tool_call_ids = message
                            .get("tool_calls")
                            .and_then(Value::as_array)
                            .map(|calls| {
                                calls
                                    .iter()
                                    .filter_map(|call| {
                                        call.get("id").and_then(Value::as_str)
                                    })
                                    .collect::<Vec<_>>()
                            })
                            .unwrap_or_default();
                        json!({
                            "role": message.get("role"),
                            "tool_call_ids": tool_call_ids,
                            "tool_call_id": message.get("tool_call_id"),
                            "has_reasoning_content": message
                                .get("reasoning_content")
                                .and_then(Value::as_str)
                                .map(|value| !value.is_empty())
                                .unwrap_or(false),
                            "content_len": message
                                .get("content")
                                .and_then(Value::as_str)
                                .map(str::len)
                                .unwrap_or(0),
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        log::error!("[Accio] outgoing messages skeleton: {skeleton:?}");
        let error_body = upstream_error_body(status.as_u16(), &text, "");
        return Ok((status, Json(error_body)).into_response());
    }

    // 首响应已校验为 200；adk_stream 内部在遇到上游静默超时会用
    // retry_request 重建同一请求重发（参数、鉴权、地址与首次一致）。
    let retry_url = upstream_url.clone();
    let retry_api_key = api_key.clone();
    let retry_body = openai_body.to_string();
    let stream = adk_stream(upstream, move || {
        let mut builder = super::http_client::get()
            .post(&retry_url)
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::ACCEPT, "text/event-stream")
            .header(header::ACCEPT_ENCODING, "identity")
            .body(retry_body.clone());
        if !retry_api_key.is_empty() {
            builder = builder.bearer_auth(&retry_api_key);
        }
        builder
    });
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

/// 子代理模型路由策略：`POST /accio/api/llm/subAgent/routing`。
///
/// 官方网关下发的策略是 `defaultAction: { "type": "fixed_model",
/// "modelCode": "1Nova-..." }`（外加 `browser-pinned-model` 等规则），会让所有子代理
/// 固定走云端模型 —— 这正是「主会话走本地自定义模型、子代理却仍走云端」的根因。
///
/// 这里拦截该端点，把 `defaultAction` 改写成 `inherit_parent`（并清空 ruleList），
/// 使子代理继承主会话当前使用的自定义模型。返回体字段与官方网关保持同构，只是内容
/// 被改写，以保证 Accio 策略解析器（`FT(VT(r))`）能正常接受。
///
/// 说明：`accio_config::rewrite_subagent_model_routing`（启动时改磁盘缓存
/// `subagent_model_routing_cache.json`）是另一层兜底；本端点级拦截优先级更高，可
/// 在 Accio 每次重新拉取策略后立即生效，不依赖启动时刻。
pub async fn handle_accio_subagent_routing() -> Result<Json<Value>, ProxyError> {
    log::info!("[Accio] subAgent/routing -> inherit_parent (follow main session model)");
    Ok(Json(json!({
        "success": true,
        "code": "200",
        "message": "0",
        "data": {
            "version": "cc-switch-local",
            "ruleList": [],
            "defaultAction": { "type": "inherit_parent", "modelCode": Value::Null }
        }
    })))
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
        // 基础保真：text / functionCall / functionResponse 三种 part。
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
        assert_eq!(messages[1]["content"], "hello");
        assert_eq!(messages[2]["role"], "assistant");
        assert_eq!(messages[2]["content"], serde_json::Value::Null);
        assert_eq!(messages[2]["tool_calls"][0]["function"]["name"], "weather");
        assert_eq!(messages[3]["role"], "tool");
        // tool content 经序列化保持字符串（对象被 JSON 化）
        assert_eq!(messages[3]["content"], "{\"content\":\"sunny\"}");
    }

    #[test]
    fn converts_thought_parts_into_reasoning_content() {
        // 子代理（skill-executor）场景：assistant 回合包含 thinking(thought)
        // + 工具调用。DeepSeek 的 thinking 模式要求把 thought 转成
        // `reasoning_content` 回传，否则 400。
        let body = json!({
            "contents": [
                {"role": "model", "parts": [
                    {"thought": true, "text": "我需要先读取 SKILL.md 了解子代理如何执行。"},
                    {"functionCall": {"id": "call_1", "name": "read", "args": "{\"filePath\": \"SKILL.md\"}"}}
                ]},
                {"role": "tool", "parts": [
                    {"functionResponse": {"id": "call_1", "responseJson": "{\"content\":\"# 文档\"}"}}
                ]}
            ]
        });

        let messages = convert_accio_contents(&body);
        // 只有 assistant（reasoning + tool_call）与 tool 两条消息
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["role"], "assistant");
        assert_eq!(messages[0]["reasoning_content"], "我需要先读取 SKILL.md 了解子代理如何执行。");
        // 纯工具调用（无文本）时 content 为 null
        assert_eq!(messages[0]["content"], serde_json::Value::Null);
        assert_eq!(messages[0]["tool_calls"][0]["function"]["name"], "read");
        assert_eq!(messages[1]["role"], "tool");
    }

    #[test]
    fn converts_text_thought_and_tool_in_one_assistant() {
        // 同一个 assistant 回合里同时有 text / thought / functionCall，
        // 应合并进同一条 assistant 消息。
        let body = json!({
            "contents": [
                {"role": "model", "parts": [
                    {"text": "我来处理这个请求。"},
                    {"thought": true, "text": "先定位相关的 SKILL 文档"},
                    {"functionCall": {"id": "t1", "name": "read", "args": {"file": "SKILL.md"}}}
                ]}
            ]
        });

        let msg = convert_accio_contents(&body);
        assert_eq!(msg.len(), 1);
        assert_eq!(msg[0]["role"], "assistant");
        assert_eq!(msg[0]["content"], "我来处理这个请求。");
        assert_eq!(msg[0]["reasoning_content"], "先定位相关的 SKILL 文档");
        assert_eq!(msg[0]["tool_calls"][0]["id"], "t1");
        assert_eq!(msg[0]["tool_calls"][0]["function"]["name"], "read");
    }

    #[test]
    fn converts_thought_part_only_flushes_single_assistant() {
        // 只有 thought（无 text / tool_calls）时，仍应刷出一条带 reasoning 的
        // assistant 消息（content = null），不能丢失 thinking 记录。
        let body = json!({
            "contents": [
                {"role": "model", "parts": [
                    {"thought": true, "text": "思考但无文本回复"}
                ]}
            ]
        });
        let messages = convert_accio_contents(&body);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["role"], "assistant");
        assert_eq!(messages[0]["reasoning_content"], "思考但无文本回复");
        assert_eq!(messages[0]["content"], serde_json::Value::Null);
        assert!(messages[0].get("tool_calls").is_none());
    }

    #[test]
    fn function_response_in_same_model_content_follows_tool_calls() {
        // Accio 实测会把 functionResponse 和 functionCall 放在同一个
        // role="model" content 里。转换必须产出 assistant(tool_calls) →
        // tool 消息，否则 DeepSeek 400「insufficient tool messages」。
        let body = json!({
            "contents": [
                {"role": "user", "parts": [{"text": "请分析热销数据"}]},
                {"role": "model", "parts": [
                    {"thought": true, "text": "先读取技能说明"},
                    {"text": "我先读取技能说明。"},
                    {"functionCall": {"id": "call_skill_1", "name": "skill", "args": "{\"action\":\"read\"}"}},
                    {"functionResponse": {"id": "call_skill_1", "responseJson": "{\"content\":\"# 热销爆品洞察\"}"}}
                ]}
            ]
        });
        let messages = convert_accio_contents(&body);
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[1]["role"], "assistant");
        assert_eq!(messages[1]["reasoning_content"], "先读取技能说明");
        assert_eq!(messages[1]["content"], "我先读取技能说明。");
        assert_eq!(messages[1]["tool_calls"][0]["id"], "call_skill_1");
        assert_eq!(messages[2]["role"], "tool");
        assert_eq!(messages[2]["tool_call_id"], "call_skill_1");
        assert_eq!(messages[2]["content"], "{\"content\":\"# 热销爆品洞察\"}");
    }

    #[test]
    fn function_response_in_separate_model_content_is_kept() {
        // functionResponse 单独出现在另一个 role="model" content 里同样要识别。
        let body = json!({
            "contents": [
                {"role": "user", "parts": [{"text": "hi"}]},
                {"role": "model", "parts": [
                    {"thought": true, "text": "need tool"},
                    {"functionCall": {"id": "c1", "name": "read", "args": "{}"}}
                ]},
                {"role": "model", "parts": [
                    {"functionResponse": {"id": "c1", "responseJson": "{\"ok\":true}"}}
                ]}
            ]
        });
        let messages = convert_accio_contents(&body);
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[1]["role"], "assistant");
        assert_eq!(messages[1]["tool_calls"][0]["id"], "c1");
        assert_eq!(messages[2]["role"], "tool");
        assert_eq!(messages[2]["tool_call_id"], "c1");
        assert_eq!(messages[2]["content"], "{\"ok\":true}");
    }

    #[test]
    fn function_response_in_user_content_is_kept() {
        // Gemini 风格：functionResponse 也可能放在 role="user" content 里。
        let body = json!({
            "contents": [
                {"role": "user", "parts": [{"text": "hi"}]},
                {"role": "model", "parts": [
                    {"functionCall": {"id": "c2", "name": "bash", "args": "{\"cmd\":\"ls\"}"}}
                ]},
                {"role": "user", "parts": [
                    {"functionResponse": {"id": "c2", "responseJson": "file1"}}
                ]}
            ]
        });
        let messages = convert_accio_contents(&body);
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[1]["role"], "assistant");
        assert_eq!(messages[1]["tool_calls"][0]["id"], "c2");
        assert_eq!(messages[2]["role"], "tool");
        assert_eq!(messages[2]["tool_call_id"], "c2");
        assert_eq!(messages[2]["content"], "file1");
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

    #[test]
    fn sse_block_split_across_chunks_keeps_multibyte_utf8_intact() {
        // 采购 is 采(U+91C7, 3 bytes) + 购(U+8D2D, 2 bytes). Build one SSE data
        // line and split the raw UTF-8 bytes *inside* 采 so the old per-chunk
        // `from_utf8_lossy` decoding would corrupt it into U+FFFD.
        let mut block = Vec::new();
        block.extend_from_slice(b"data: {\"choices\":[{\"delta\":{\"content\":\"1688");
        block.extend_from_slice("采购".as_bytes());
        block.extend_from_slice(b"\"}}]}\n\n");

        // 采's first byte (0xE9) sits right after "...1688". Cut after its
        // second byte (0x87) so chunk 1 ends mid-character.
        let first_byte_of_cai = block
            .iter()
            .position(|&b| b == 0xE9)
            .expect("采 lead byte present");
        let split_inside_char = first_byte_of_cai + 2;
        assert_eq!(block[split_inside_char], 0x87, "chunk split must cut inside 采");

        let mut buffer = Vec::new();
        // Feed the block in two chunks, splitting mid-character.
        buffer.extend_from_slice(&block[..split_inside_char]);
        buffer = normalize_crlf(buffer);
        assert_eq!(drain_next_sse_block(&mut buffer), None);
        buffer.extend_from_slice(&block[split_inside_char..]);
        buffer = normalize_crlf(buffer);

        let decoded = drain_next_sse_block(&mut buffer);
        assert!(decoded.is_some());
        let decoded = decoded.unwrap();
        // Must not contain any U+FFFD replacement char (乱码).
        assert!(
            !decoded.contains('\u{FFFD}'),
            "decoded block contains corrupted replacement char: {decoded:?}"
        );
        assert!(decoded.contains("采购"), "expected 采购 intact, got {decoded:?}");
        assert!(buffer.is_empty());
    }

    #[test]
    fn crlf_terminated_sse_block_is_normalized() {
        let mut buffer: Vec<u8> = b"data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\r\n\r\n".to_vec();
        buffer = normalize_crlf(buffer);
        let decoded = drain_next_sse_block(&mut buffer).expect("block should parse");
        assert!(decoded.contains("\"content\":\"hi\""));
        assert!(buffer.is_empty());
    }
}
