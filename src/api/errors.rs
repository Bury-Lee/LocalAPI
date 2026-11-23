//! 统一的错误响应构造。

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

// (StatusCode, Json) 元组实现了 IntoResponse，可以直接当错误响应返回
pub(crate) fn upstream_error(message: &str) -> Response {
    (
        StatusCode::BAD_GATEWAY,
        Json(json!({
            "error": { "message": message, "type": "upstream_error", "code": "web_driver" }
        })),
    )
        .into_response()
}

// 参数/配置错误（例如未知厂商）：400，方便控制台直接看到原因
pub(crate) fn client_error(message: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({
            "error": { "message": message, "type": "invalid_request_error", "code": "bad_request" }
        })),
    )
        .into_response()
}
