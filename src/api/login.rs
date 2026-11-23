//! 登录路由：以有头模式打开站点，等人工登录。

use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Query, State};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;
use tracing::info;

use crate::api::errors::{client_error, upstream_error};
use crate::app_status::AppState;

/// `GET /login`：以有头模式打开站点，等人工登录（最长 `LoginWaitSecs`）。
///
/// 支持按厂商登录，参数（均可省略）：
/// - `site` / `vendor`：deepseek / qwen / doubao / gpt，默认取当前配置的站点
/// - `profile`：浏览器 profile 名，默认 `default`
/// - `wait`：本次等待登录的秒数，默认 `LoginWaitSecs`
/// - `driver`：自定义驱动脚本路径（相对启动目录），默认 `cjs/<site>-driver.cjs`
pub async fn login(State(st): State<Arc<AppState>>, Query(q): Query<LoginParams>) -> Response {
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
        .map(std::path::PathBuf::from);
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
