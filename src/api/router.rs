use std::sync::Arc;

use axum::middleware;
use axum::routing::{get, post};
use axum::Router;
use tower_http::cors::CorsLayer;

use crate::api::handle;
use crate::app_status::AppState;

pub fn BuildRouter(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/health", get(handle::health))
        .route("/status", get(handle::status))
        .route("/login", get(handle::login))
        .route("/control", get(handle::control))
        .route("/ping", post(handle::ping))
        .route("/v1/models", get(handle::models))
        .route("/v1/chat/completions", post(handle::chat_completions))
        // 后添加的 layer 在更外层：CORS 先处理预检请求，再进 access_log
        .layer(middleware::from_fn(handle::access_log))
        .layer(CorsLayer::permissive())
        .with_state(state)
}
