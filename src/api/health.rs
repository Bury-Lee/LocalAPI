//! 健康检查与运行状态。

use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::Json;
use serde_json::{json, Value};

use crate::app_status::AppState;

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
