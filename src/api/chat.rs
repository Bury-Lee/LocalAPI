//! OpenAI 对话协议：把请求翻译成 driver 的 streamAsk，并把回复转成 JSON/SSE。

use std::convert::Infallible;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::{Extension, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tracing::{debug, info, warn};

use crate::api::errors::upstream_error;
use crate::api::tools::{next_history_id, now_secs};
use crate::app_status::{AppState, ReqId};
use crate::config::config::Config;
use crate::driver::{Driver, StreamEnd};
use crate::openai::{self, ChatRequest};

/// `POST /v1/chat/completions`：非流式一次性返回，流式按字符切片成 SSE。
pub async fn chat_completions(
    State(st): State<Arc<AppState>>,
    Extension(rid): Extension<ReqId>,
    Json(req): Json<ChatRequest>,
) -> Response {
    let started = Instant::now();
    let cfg = st.cfg;
    let model = req
        .model
        .clone()
        .unwrap_or_else(|| fallback_model(cfg));
    let mode = openai::mode_of(&model);
    let created = now_secs();
    let id = format!("chatcmpl-{created}");
    let history_id = next_history_id();
    // 原样保留请求消息，成功后再加上助手回复写进会话历史
    let history_messages: Vec<Value> = req
        .messages
        .iter()
        .filter_map(|m| serde_json::to_value(m).ok())
        .collect();
    let question = openai::build_question(&req);
    let prompt_chars = question.chars().count();
    // 有的站点（网页版）对 tools 提示词很敏感，IGNORE_TOOLS 时直接丢掉
    let tools = if cfg.IgnoreTools {
        Vec::new()
    } else {
        req.tools.clone()
    };
    //TODO:进行特有格式转译避封控
    let tools_text = openai::build_tools_text(&tools);
    info!(
        request_id = %rid.0,
        model = %model,
        mode,
        stream = req.stream,
        messages = req.messages.len(),
        tools = tools.len(),
        prompt_chars,
        "chat request"
    );
    let driver = match st.manager.get().await {
        Ok(d) => d,
        Err(e) => {
            warn!(request_id = %rid.0, error = %e, "driver unavailable");
            return upstream_error(&e);
        }
    };

    // 非流式：直接 await 完整结果再一次性返回。
    // _permit 是 RAII：离开作用域（包括提前 return）自动归还信号量。
    if !req.stream {
        let _permit = st.sem.acquire().await;
        return match ask(&driver, question, tools_text, tools, &model, mode, cfg).await {
            Ok(end) => {
                let reply_chars = end.result.as_deref().unwrap_or("").chars().count();
                // 工具调用现在由模型以 ```TYC 代码块（YAML）返回，这里从原始回复里解析
                let tool_call = end.result.as_deref().and_then(openai::parse_tool_call_yaml);
                info!(
                    request_id = %rid.0,
                    reply_chars,
                    tool_calls = usize::from(tool_call.is_some()),
                    elapsed_ms = started.elapsed().as_millis() as u64,
                    "chat reply"
                );
                // OpenAI 约定：有 tool_calls 时 content 必须是 null，finish_reason 用 tool_calls
                let mut message =
                    json!({ "role": "assistant", "content": end.result.clone().unwrap_or_default() });
                let finish = if tool_call.is_some() {
                    "tool_calls"
                } else {
                    "stop"
                };
                if let Some(call) = &tool_call {
                    message["content"] = Value::Null;
                    message["tool_calls"] = json!([tool_call_json(call, 0)]);
                }
                // 非流式在这里就能拿到完整回复，直接把请求消息 + 助手回复写入历史
                {
                    let mut history = st.history.lock().await;
                    let mut messages = history_messages;
                    messages.push(message.clone());
                    match history.Import(history_id, messages) {
                        Err(err) => {
                            warn!(session = history_id, error = %err, "history import failed")
                        }
                        Ok(()) => {
                            history.SetModel(history_id, &model);
                            if let Err(err) = history.SaveSession(history_id) {
                                warn!(session = history_id, error = %err, "history save failed");
                            }
                        }
                    }
                }
                Json(json!({
                    "id": id,
                    "object": "chat.completion",
                    "created": created,
                    "model": model,
                    "choices": [{ "index": 0, "message": message, "finish_reason": finish }],
                    "usage": { "prompt_tokens": 0, "completion_tokens": 0, "total_tokens": 0 }
                }))
                .into_response()
            }
            Err(e) => {
                warn!(
                    request_id = %rid.0,
                    error = %e,
                    elapsed_ms = started.elapsed().as_millis() as u64,
                    "chat failed"
                );
                upstream_error(&e)
            }
        };
    }

    // 流式：handler 本身只返回“响应头 + 一个流”，真正的上游请求放到 spawn 的任务里。
    // mpsc 容量 32 提供背压：客户端读得慢，send 就会 await。
    let (tx, rx) = mpsc::channel::<Result<Event, Infallible>>(32);
    let state = st.clone();
    let driver = driver.clone();
    // 标记处要先落一份“请求”记录（流被中断时也能查到），再让流任务补写回复
    let history_model = model.clone();
    let request_messages = history_messages.clone();
    tokio::spawn(async move {
        let request_id = rid.0.clone();
        let _permit = state.sem.acquire().await;
        debug!(request_id = %request_id, "stream opened, waiting for upstream");
        // 先发一个空 role 分片：客户端收到就能确认流已建立（然后才是等待上游的静默期）
        let greeting = json!({
            "id": id.as_str(),
            "object": "chat.completion.chunk",
            "created": created,
            "model": model.as_str(),
            "choices": [{ "index": 0, "delta": { "role": "assistant", "content": "" }, "finish_reason": Value::Null }]
        });
        if tx
            .send(Ok(Event::default().data(greeting.to_string())))
            .await
            .is_err()
        {
            debug!(request_id = %request_id, "client disconnected before upstream");
            return;
        }
        match ask(&driver, question, tools_text, tools, &model, mode, state.cfg).await {
            Ok(end) => {
                let reply_chars = end.result.as_deref().unwrap_or("").chars().count();
                // 工具调用现在由模型以 ```TYC 代码块（YAML）返回，这里从原始回复里解析(TYC作为一种类似魔数的标记,避免工具调用的检测)
                let tool_call = end.result.as_deref().and_then(openai::parse_tool_call_yaml);
                let tool_calls = usize::from(tool_call.is_some());
                match tool_call {
                    None => {
                        let text = end.result.unwrap_or_default();
                        for part in openai::split_chars(&text, 120) {
                            let body = json!({
                                "id": id.as_str(),
                                "object": "chat.completion.chunk",
                                "created": created,
                                "model": model.as_str(),
                                "choices": [{ "index": 0, "delta": { "content": part }, "finish_reason": Value::Null }]
                            });
                            if tx
                                .send(Ok(Event::default().data(body.to_string())))
                                .await
                                .is_err()
                            {
                                debug!(request_id = %request_id, "client disconnected mid-stream");
                                return;
                            }
                        }
                        // finish_reason 只在最后一片非 null，这是 OpenAI 流式协议的要求
                        let body = json!({
                            "id": id.as_str(),
                            "object": "chat.completion.chunk",
                            "created": created,
                            "model": model.as_str(),
                            "choices": [{ "index": 0, "delta": {}, "finish_reason": "stop" }]
                        });
                        let _ = tx.send(Ok(Event::default().data(body.to_string()))).await;
                        // 流式完成：把整段会话覆盖为“请求消息 + 助手回复”
                        {
                            let mut history = state.history.lock().await;
                            let mut messages = history_messages;
                            messages.push(json!({ "role": "assistant", "content": text }));
                            match history.Import(history_id, messages) {
                                Err(err) => {
                                    warn!(session = history_id, error = %err, "history import failed")
                                }
                                Ok(()) => {
                                    history.SetModel(history_id, &model);
                                    if let Err(err) = history.SaveSession(history_id) {
                                        warn!(session = history_id, error = %err, "history save failed");
                                    }
                                }
                            }
                        }
                    }
                    Some(call) => {
                        // OpenAI 流式 tool_calls：首片带 id/name，后续分片只累加 arguments
                        let index = 0usize;
                        let name = call
                            .get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or_default()
                            .to_string();
                        let args = args_json_string(&call);
                        let head = json!({
                            "tool_calls": [{
                                "index": index,
                                "id": tool_call_id(index),
                                "type": "function",
                                "function": { "name": name, "arguments": "" }
                            }]
                        });
                        let body = json!({
                            "id": id.as_str(),
                            "object": "chat.completion.chunk",
                            "created": created,
                            "model": model.as_str(),
                            "choices": [{ "index": 0, "delta": head, "finish_reason": Value::Null }]
                        });
                        if tx
                            .send(Ok(Event::default().data(body.to_string())))
                            .await
                            .is_err()
                        {
                            debug!(request_id = %request_id, "client disconnected mid-stream");
                            return;
                        }
                        for part in openai::split_chars(&args, 60) {
                            let delta = json!({
                                "tool_calls": [{
                                    "index": index,
                                    "function": { "arguments": part }
                                }]
                            });
                            let body = json!({
                                "id": id.as_str(),
                                "object": "chat.completion.chunk",
                                "created": created,
                                "model": model.as_str(),
                                "choices": [{ "index": 0, "delta": delta, "finish_reason": Value::Null }]
                            });
                            if tx
                                .send(Ok(Event::default().data(body.to_string())))
                                .await
                                .is_err()
                            {
                                debug!(request_id = %request_id, "client disconnected mid-stream");
                                return;
                            }
                        }
                        let body = json!({
                            "id": id.as_str(),
                            "object": "chat.completion.chunk",
                            "created": created,
                            "model": model.as_str(),
                            "choices": [{ "index": 0, "delta": {}, "finish_reason": "tool_calls" }]
                        });
                        let _ = tx.send(Ok(Event::default().data(body.to_string()))).await;
                        // 流式完成（工具调用）：把整段会话覆盖为“请求消息 + 助手回复”
                        {
                            let mut history = state.history.lock().await;
                            let mut messages = history_messages;
                            messages.push(json!({
                                "role": "assistant",
                                "content": Value::Null,
                                "tool_calls": [tool_call_json(&call, 0)]
                            }));
                            match history.Import(history_id, messages) {
                                Err(err) => {
                                    warn!(session = history_id, error = %err, "history import failed")
                                }
                                Ok(()) => {
                                    history.SetModel(history_id, &model);
                                    if let Err(err) = history.SaveSession(history_id) {
                                        warn!(session = history_id, error = %err, "history save failed");
                                    }
                                }
                            }
                        }
                    }
                }
                info!(
                    request_id = %request_id,
                    reply_chars,
                    tool_calls,
                    elapsed_ms = started.elapsed().as_millis() as u64,
                    "stream reply"
                );
            }
            Err(e) => {
                warn!(
                    request_id = %request_id,
                    error = %e,
                    elapsed_ms = started.elapsed().as_millis() as u64,
                    "stream failed"
                );
                let text = format!("[错误] {e}");
                let body = json!({
                    "id": id.as_str(),
                    "object": "chat.completion.chunk",
                    "created": created,
                    "model": model.as_str(),
                    "choices": [{ "index": 0, "delta": { "content": text }, "finish_reason": Value::Null }]
                });
                let _ = tx.send(Ok(Event::default().data(body.to_string()))).await;
                let body = json!({
                    "id": id.as_str(),
                    "object": "chat.completion.chunk",
                    "created": created,
                    "model": model.as_str(),
                    "choices": [{ "index": 0, "delta": {}, "finish_reason": "stop" }]
                });
                let _ = tx.send(Ok(Event::default().data(body.to_string()))).await;
            }
        }
        // [DONE] 是 SSE 的结束哨兵（不是 JSON），客户端看到它就停止读取
        let _ = tx.send(Ok(Event::default().data("[DONE]"))).await;
    });

    // Sse 负责加 Content-Type: text/event-stream，并把每个 Event 编码成 "data: ...\n\n"；
    // KeepAlive 在长时间没有数据时发注释行，防止代理/客户端判超时。
    let response = Sse::new(ReceiverStream::new(rx))
        .keep_alive(KeepAlive::default())
        .into_response();

    //记录History：先把本次请求消息落盘（流中断也能查到），
    //助手回复由 spawn 任务在拿到完整结果后补写；会话已存在时不覆盖。
    {
        let mut history = st.history.lock().await;
        match history.ImportIfAbsent(history_id, request_messages) {
            Err(err) => warn!(session = history_id, error = %err, "history import failed"),
            Ok(()) => {
                history.SetModel(history_id, &history_model);
                if let Err(err) = history.SaveSession(history_id) {
                    warn!(session = history_id, error = %err, "history save failed");
                }
            }
        }
    }

    response
}

// 把“一次 OpenAI 请求”翻译成 driver 的 streamAsk 参数，并等待最终结果。
// driver 是事件模型，rpc() 拿 streamId，wait_stream() 等 stream-end。
async fn ask(
    driver: &Arc<Driver>,
    question: String,
    tools_text: String,
    tools: Vec<Value>,
    model: &str,
    mode: &str,
    cfg: &Config,
) -> Result<StreamEnd, String> {
    // reset: "auto" 表示沿用网页端会话历史（只发最后一条消息）；
    // 超过 maxTurnsPerChat 时 driver 会自己摘要并开新会话。
    let mut params = json!({
        "question": question,
        "model": model,
        "profile": "default",
        "headless": cfg.Headless,
        "reset": "auto",
        "maxTurnsPerChat": 50,
        "timeoutMs": cfg.Timeout().as_millis() as u64
    });
    if !tools_text.is_empty() {
        params["toolsText"] = json!(tools_text);
    }
    if !tools.is_empty() {
        params["tools"] = json!(tools);
    }
    // DeepSeek 的专家模式不是参数开关，而是“回放一串点击”切换到页面上的专家模型
    if mode == "expert" && cfg.SiteName() == "deepseek" {
        params["calibKey"] = json!("deepseek-reasoner");
    }
    debug!(
        mode,
        headless = cfg.Headless,
        timeout_ms = cfg.Timeout().as_millis() as u64,
        "sending driver streamAsk"
    );
    let started = driver
        .rpc("streamAsk", params, Duration::from_secs(30))
        .await?;
    let stream_id = started
        .get("streamId")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "driver returned no streamId".to_string())?
        .to_string();
    debug!(stream_id = %stream_id, "driver stream registered");
    // 等 stream-end 事件；比 driver 内部 timeout 多留 60s，让内层先超时、错误更具体
    let end = driver
        .wait_stream(&stream_id, cfg.Timeout() + Duration::from_secs(60))
        .await?;
    if !end.ok {
        let err = end
            .error
            .unwrap_or_else(|| format!("{} web error", cfg.SiteName()));
        return Err(friendly_error(&err, cfg));
    }
    Ok(end)
}

// 这里做一层翻译对于driver的原始错误的转译,不然对API使用者不友好（还带着它自己的端口号），
fn friendly_error(err: &str, cfg: &Config) -> String {
    if err.starts_with("login required") {
        format!(
            "{} web is not logged in. Open http://{}/login in a browser (GET /login opens one), sign in with the 'keep me logged in' option, then retry.",
            cfg.SiteName(), cfg.Host
        )
    } else {
        err.to_string()
    }
}

// 把 YAML 解析出的 tool_call（{name, args}）转成 OpenAI 的 tool_calls 项。
// 注意 arguments 必须是“JSON 字符串”，不能是对象。
fn tool_call_json(call: &Value, index: usize) -> Value {
    let name = call.get("name").and_then(|v| v.as_str()).unwrap_or_default();
    json!({
        "index": index,
        "id": tool_call_id(index),
        "type": "function",
        "function": { "name": name, "arguments": args_json_string(call) }
    })
}

// args 可能是对象、字符串或其它标量，统一序列化成 JSON 字符串。
fn args_json_string(call: &Value) -> String {
    match call.get("args") {
        Some(Value::String(s)) => s.clone(),
        Some(v) => serde_json::to_string(v).unwrap_or_else(|_| "{}".to_string()),
        None => "{}".to_string(),
    }
}

// 每次调用给一个唯一 id，客户端靠它把后续分片归组。
fn tool_call_id(index: usize) -> String {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    format!("call_{millis}_{index}")
}

// 请求没带 model 时按站点给个默认值
fn fallback_model(cfg: &Config) -> String {
    match cfg.SiteName().as_str() {
        "qwen" => "qwen3.7-plus".to_string(),
        "doubao" => "doubao-pro".to_string(),
        "gpt" => "auto".to_string(),
        _ => "deepseek-chat".to_string(),
    }
}
