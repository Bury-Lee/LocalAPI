use std::sync::Arc;

use axum::routing::{get, post};
use axum::Router;
use tower_http::cors::CorsLayer;

use crate::api::{chat, health, history, login, middleware as request_log, models, pages};
use crate::app_status::AppState;

pub fn BuildRouter(state: Arc<AppState>) -> Router {
    Router::new()
        // 基础状态
        .route("/health", get(health::health))
        .route("/status", get(health::status))
        // 登录与页面
        .route("/login", get(login::login))
        .route("/control", get(pages::control))
        .route("/guide", get(pages::guide))
        // 历史记录
        .route("/history", get(history::list_history))
        .route("/history/{id}", get(history::OutportSession))
        .route("/history/{id}/download", get(history::DownloadSession))
        // 模型与对话
        .route("/ping", post(models::ping))
        .route("/v1/models", get(models::models))
        .route("/v1/chat/completions", post(chat::chat_completions))
        // 后添加的 layer 在更外层：CORS 先处理预检请求，再进 access_log
        .layer(axum::middleware::from_fn(request_log::access_log))
        .layer(CorsLayer::permissive())
        .with_state(state)
}
