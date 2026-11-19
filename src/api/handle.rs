//! HTTP 层：把 OpenAI 协议翻译成 driver 的行 JSON-RPC。
//!
//! 依赖单向：handle -> driver / openai / app_status，handle 只管协议与并发控制。

use std::convert::Infallible;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::{Extension, Query, State};
use axum::http::{HeaderValue, Request, StatusCode};
use axum::middleware::Next;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{Html, IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tracing::{debug, error, info, warn};

use crate::api::tools::{next_request_id, now_secs};
use crate::app_status::{AppState, ReqId};
use crate::config::config::{Config, ResolvePath};
use crate::driver::{Driver, StreamEnd};
use crate::openai::{self, ChatRequest};

/// `GET /health`
pub async fn health() -> Json<Value> {
    Json(json!({ "ok": true }))
}

/// `GET /status`：驱动/浏览器/登录态。
pub async fn status(State(st): State<Arc<AppState>>) -> Json<Value> {
    let driver = match st.manager.get().await {
        Ok(d) => d,
        Err(e) => return Json(json!({ "error": e })),
    };
    let browser = driver
        .rpc("status", json!({}), Duration::from_secs(20))
        .await
        .map(|v| v.get("browser").cloned().unwrap_or(Value::Null))
        .unwrap_or_else(|e| json!({ "error": e }));
    let inspect = match driver
        .rpc("inspect", json!({}), Duration::from_secs(20))
        .await
    {
        Ok(v) => v,
        Err(e) => json!({ "error": e }),
    };
    Json(json!({
        "site": st.cfg.SiteName(),
        "driver": browser,
        "login": inspect.get("login").cloned().unwrap_or(Value::Null),
        "modelBadge": inspect.get("modelBadge").cloned().unwrap_or(Value::Null)
    }))
}

/// `GET /login`：以有头模式打开站点，等人工登录（最长 `LoginWaitSecs`）。
///
/// 支持按厂商登录，参数（均可省略）：
/// - `site` / `vendor`：deepseek / qwen / doubao / gpt，默认取当前配置的站点
/// - `profile`：浏览器 profile 名，默认 `default`
/// - `wait`：本次等待登录的秒数，默�?`LoginWaitSecs`
/// - `driver`：自定义驱动脚本路径（相对启动目录），默�?`cjs/<site>-driver.cjs`
pub async fn login(
    State(st): State<Arc<AppState>>,
    Query(q): Query<LoginParams>,
) -> Response {
    let site = q
        .site
        .or(q.vendor)
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| st.cfg.SiteName());
    let profile = q
        .profile
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "default".to_string());
    let driver = q
        .driver
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ResolvePath);
    let manager = match st.manager_for_site(&site, driver).await {
        Ok(m) => m,
        Err(e) => return client_error(&e),
    };
    let driver = match manager.get().await {
        Ok(d) => d,
        Err(e) => return upstream_error(&e),
    };
    let wait = q
        .wait
        .filter(|s| *s > 0)
        .map(Duration::from_secs)
        .unwrap_or_else(|| st.cfg.LoginWait());
    let params = json!({
        "profile": profile,
        "headless": false,
        "timeoutMs": wait.as_millis() as u64
    });
    info!(%site, %profile, wait_secs = wait.as_secs(), "login requested");
    match driver
        .rpc("login", params, wait + Duration::from_secs(30))
        .await
    {
        Ok(mut v) => {
            if !v.is_object() {
                v = json!({ "result": v });
            }
            v["site"] = json!(site);
            v["profile"] = json!(profile);
            Json(v).into_response()
        }
        Err(e) => upstream_error(&e),
    }
}

#[derive(Debug, Default, serde::Deserialize)]
pub struct LoginParams {
    pub site: Option<String>,
    pub vendor: Option<String>,
    pub profile: Option<String>,
    pub wait: Option<u64>,
    pub driver: Option<String>,
}

/// `GET /control`：给浏览器看的控制台，把常用入口列出来。
pub async fn control(State(st): State<Arc<AppState>>) -> Html<String> {
    let cfg = st.cfg;
    let site = escape_html(&cfg.SiteName());
    let host = escape_html(&cfg.Host);
    let driver = escape_html(&cfg.Driver);
    let base = escape_html(&cfg.Base);
    let headless = if cfg.Headless { "无头" } else { "有头" };
    // 不同厂商的登录入口：/login 支持 site/profile/wait 参数
    let mut vendors = String::new();
    for (id, name) in VENDORS {
        let label = escape_html(name);
        vendors.push_str(&format!(
            r#"<a href="/login?site={id}">{label}</a><br>"#
        ));
    }
    Html(format!(
        r#"<!doctype html>
<html lang="zh-CN"><head><meta charset="utf-8"><title>dsh-web-gateway</title>
<style>
body{{font:14px/1.6 system-ui,"Segoe UI",sans-serif;max-width:760px;margin:40px auto;padding:0 16px;color:#222}}
h1{{font-size:20px}}code{{background:#f4f4f4;padding:1px 5px;border-radius:4px}}
table{{border-collapse:collapse;width:100%}}td{{border-bottom:1px solid #eee;padding:6px 4px;vertical-align:top}}
a{{color:#0b57d0;text-decoration:none}}a:hover{{text-decoration:underline}}
</style></head><body>
<h1>欢迎使用 LocalAPI</h1>
<table>
<tr><td>站点</td><td><code>{site}</code></td></tr>
<tr><td>监听</td><td><code>{host}</code>（{headless}）</td></tr>
<tr><td>驱动</td><td><code>{driver}</code></td></tr>
<tr><td>数据目录</td><td><code>{base}</code></td></tr>
<tr><td>厂商登录</td><td>{vendors}</td></tr>
<tr><td>登录</td><td><a href="/login">/login</a>（弹出浏览器，人工登录）</td></tr>
<tr><td>状态</td><td><a href="/status">/status</a></td></tr>
<tr><td>模型</td><td><a href="/v1/models">/v1/models</a></td></tr>
<tr><td>对话</td><td><code>POST /v1/chat/completions</code></td></tr>
<tr><td>健康检查</td><td><a href="/health">/health</a></td></tr>
</table>
<p>OpenAI 客户端用 <code>baseURL=http://{host}/v1</code>，apiKey 随便填。</p>
</body></html>"#
    ))
}

/// 控制台里展示的厂商列表（与 cjs/<site>-driver.cjs 对应）。
const VENDORS: &[(&str, &str)] = &[
    ("deepseek", "DeepSeek"),
    ("qwen", "通义千问 Qwen"),
    ("doubao", "豆包 Doubao"),
    ("gpt", "ChatGPT"),
];

/// `GET /v1/models`：优先向驱动的 `models` RPC 要，失败则按站点兜底。
pub async fn models(State(st): State<Arc<AppState>>) -> Json<Value> {
    info!("list models");
    let created = now_secs();
    let mut data: Vec<Value> = Vec::new();
    if let Ok(driver) = st.manager.get().await {
        match driver.rpc("models", json!({}), Duration::from_secs(10)).await {
            Ok(v) => {
                if let Some(list) = v.get("models").and_then(|x| x.as_array()) {
                    for m in list {
                        let id = m.get("id").and_then(|x| x.as_str()).unwrap_or("");
                        if id.is_empty() {
                            continue;
                        }
                        let name = m.get("name").and_then(|x| x.as_str()).unwrap_or(id);
                        data.push(json!({
                            "id": id,
                            "object": "model",
                            "created": created,
                            "owned_by": format!("{}-web", st.cfg.SiteName()),
                            "name": name
                        }));
                    }
                }
            }
            Err(e) => debug!(error = %e, "driver models rpc failed, using fallback"),
        }
    }
    if data.is_empty() {
        data = crate::openai::fallback_models(&st.cfg.SiteName(), created);
    }
    Json(json!({ "object": "list", "data": data }))
}

/// `POST /ping`：请求体是模型 id 数组（空数组=全部），
/// 也接受 `{"models":[...]}`；返回每个模型是否出现在驱动列表里。
pub async fn ping(
    State(st): State<Arc<AppState>>,
    Json(body): Json<Value>,
) -> Response {
    let requested: Vec<String> = match &body {
        Value::Array(list) => list
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect(),
        Value::Object(map) => map
            .get("models")
            .and_then(|v| v.as_array())
            .map(|list| {
                list.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    let driver = match st.manager.get().await {
        Ok(d) => d,
        Err(e) => return upstream_error(&e),
    };
    let pong = driver
        .rpc("ping", json!({}), Duration::from_secs(10))
        .await
        .unwrap_or_else(|e| json!({ "error": e }));
    let available: Vec<String> = driver
        .rpc("models", json!({}), Duration::from_secs(10))
        .await
        .ok()
        .and_then(|v| v.get("models").and_then(|m| m.as_array()).cloned())
        .map(|list| {
            list.iter()
                .filter_map(|m| m.get("id").and_then(|x| x.as_str()).map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let targets: Vec<String> = if requested.is_empty() {
        available.clone()
    } else {
        requested
    };
    let results: Vec<Value> = targets
        .iter()
        .map(|id| {
            json!({
                "model": id,
                "reachable": available.iter().any(|a| a == id)
            })
        })
        .collect();
    Json(json!({ "driver": pong, "models": results })).into_response()
}

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
    let question = openai::build_question(&req);
    let prompt_chars = question.chars().count();
    // 有的站点（网页版）对 tools 提示词很敏感，IGNORE_TOOLS 时直接丢掉
    let tools = if cfg.IgnoreTools {
        Vec::new()
    } else {
        req.tools.clone()
    };
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
    // mpsc 容量 32 提供背压：客户端读得慢，emit() 就会 await。
    let (tx, rx) = mpsc::channel::<Result<Event, Infallible>>(32);
    let state = st.clone();
    let driver = driver.clone();
    tokio::spawn(async move {
        let request_id = rid.0.clone();
        let _permit = state.sem.acquire().await;
        debug!(request_id = %request_id, "stream opened, waiting for upstream");
        // 先发一个空 role 分片：客户端收到就能确认流已建立（然后才是等待上游的静默期）
        if !emit(
            &tx,
            &id,
            created,
            &model,
            json!({ "role": "assistant", "content": "" }),
            Value::Null,
        )
        .await
        {
            debug!(request_id = %request_id, "client disconnected before upstream");
            return;
        }
        match ask(&driver, question, tools_text, tools, &model, mode, state.cfg).await {
            Ok(end) => {
                let reply_chars = end.result.as_deref().unwrap_or("").chars().count();
                // 工具调用现在由模型以 ```TYC 代码块（YAML）返回，这里从原始回复里解析
                let tool_call = end.result.as_deref().and_then(openai::parse_tool_call_yaml);
                let tool_calls = usize::from(tool_call.is_some());
                match tool_call {
                    None => {
                        let text = end.result.unwrap_or_default();
                        for part in openai::split_chars(&text, 120) {
                            if !emit(&tx, &id, created, &model, json!({ "content": part }), Value::Null)
                                .await
                            {
                                debug!(request_id = %request_id, "client disconnected mid-stream");
                                return;
                            }
                        }
                        let _ = emit(&tx, &id, created, &model, json!({}), json!("stop")).await;
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
                        if !emit(&tx, &id, created, &model, head, Value::Null).await {
                            debug!(request_id = %request_id, "client disconnected mid-stream");
                            return;
                        }
                        for part in openai::split_chars(&args, 60) {
                            let body = json!({
                                "tool_calls": [{
                                    "index": index,
                                    "function": { "arguments": part }
                                }]
                            });
                            if !emit(&tx, &id, created, &model, body, Value::Null).await {
                                debug!(request_id = %request_id, "client disconnected mid-stream");
                                return;
                            }
                        }
                        let _ = emit(
                            &tx,
                            &id,
                            created,
                            &model,
                            json!({}),
                            json!("tool_calls"),
                        )
                        .await;
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
                let _ = emit(
                    &tx,
                    &id,
                    created,
                    &model,
                    json!({ "content": text }),
                    Value::Null,
                )
                .await;
                let _ = emit(&tx, &id, created, &model, json!({}), json!("stop")).await;
            }
        }
        // [DONE] 是 SSE 的结束哨兵（不是 JSON），客户端看到它就停止读取
        let _ = tx.send(Ok(Event::default().data("[DONE]"))).await;
    });

    // Sse 负责加 Content-Type: text/event-stream，并把每个 Event 编码成 "data: ...\n\n"；
    // KeepAlive 在长时间没有数据时发注释行，防止代理/客户端判超时。
    Sse::new(ReceiverStream::new(rx))
        .keep_alive(KeepAlive::default())
        .into_response()
}

/// 每个请求一条访问日志，并把 request id 塞进 extensions 供 handler 读取。
pub async fn access_log(mut req: Request<axum::body::Body>, next: Next) -> Response {
    let rid = req
        .headers()
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .unwrap_or_else(next_request_id);
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    req.extensions_mut().insert(ReqId(rid.clone()));
    let started = Instant::now();
    let mut res = next.run(req).await;
    let elapsed_ms = started.elapsed().as_millis() as u64;
    if let Ok(value) = HeaderValue::from_str(&rid) {
        res.headers_mut().insert("x-request-id", value);
    }
    let status = res.status().as_u16();
    match status {
        500..=599 => error!(request_id = %rid, %method, %path, status, elapsed_ms, "request"),
        400..=499 => warn!(request_id = %rid, %method, %path, status, elapsed_ms, "request"),
        _ => info!(request_id = %rid, %method, %path, status, elapsed_ms, "request"),
    }
    res
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

// driver 的原始错误对 API 使用者不友好（还带着它自己的端口号），这里做一层翻译
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

// 发一个 chat.completion.chunk。返回 false 说明客户端已断开（channel 关闭）。
// 注意 finish_reason 只在最后一片非 null，这是 OpenAI 流式协议的要求。
async fn emit(
    tx: &mpsc::Sender<Result<Event, Infallible>>,
    id: &str,
    created: u64,
    model: &str,
    delta: Value,
    finish_reason: Value,
) -> bool {
    let body = json!({
        "id": id,
        "object": "chat.completion.chunk",
        "created": created,
        "model": model,
        "choices": [{ "index": 0, "delta": delta, "finish_reason": finish_reason }]
    });
    tx.send(Ok(Event::default().data(body.to_string())))
        .await
        .is_ok()
}

// (StatusCode, Json) 元组实现了 IntoResponse，可以直接当错误响应返回
fn upstream_error(message: &str) -> Response {
    (
        StatusCode::BAD_GATEWAY,
        Json(json!({
            "error": { "message": message, "type": "upstream_error", "code": "web_driver" }
        })),
    )
        .into_response()
}

// 参数/配置错误（例如未知厂商）：400，方便控制台直接看到原因
fn client_error(message: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({
            "error": { "message": message, "type": "invalid_request_error", "code": "bad_request" }
        })),
    )
        .into_response()
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

// 控制台 HTML 里只放配置值，做最小转义避免破坏标签
fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
