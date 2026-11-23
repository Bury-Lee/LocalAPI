//! 模型列表与连通性探测。

use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};
use tracing::{debug, info};

use crate::api::errors::upstream_error;
use crate::api::tools::now_secs;
use crate::app_status::AppState;

/// `GET /v1/models`：向驱动的 `models` RPC 要
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
pub async fn ping(State(st): State<Arc<AppState>>, Json(body): Json<Value>) -> Response {
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
