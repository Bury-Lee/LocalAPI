//! 请求日志中间件：为每个请求生成 request id 并记录访问日志。

use std::time::Instant;

use axum::http::{HeaderValue, Request};
use axum::middleware::Next;
use axum::response::Response;
use tracing::{error, info, warn};

use crate::api::tools::next_request_id;
use crate::app_status::ReqId;

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
