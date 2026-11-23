//! 历史记录路由：列表、导出查看、浏览器下载。

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{Html, IntoResponse, Response};

use crate::api::pages::escape_html;
use crate::app_status::AppState;

/// `GET /history`：列出当前已经存在的所有会话（按时间倒序）。
pub async fn list_history(State(st): State<Arc<AppState>>) -> Html<String> {
    let history = st.history.lock().await;
    let mut rows = String::new();
    // id 是毫秒时间戳，倒序就是最新的在前
    for (id, session) in history.Sessions().iter().rev() {
        let model = escape_html(if session.model.is_empty() {
            "-"
        } else {
            &session.model
        });
        rows.push_str(&format!(
            r#"<tr><td>{id}</td><td>{}</td><td>{model}</td><td>{}</td><td><a href="/history/{id}">查看</a> · <a href="/history/{id}/download">下载</a></td></tr>"#,
            format_time(session.created_at),
            session.messages.len(),
        ));
    }
    if rows.is_empty() {
        rows.push_str(r#"<tr><td colspan="5" class="empty">还没有历史记录</td></tr>"#);
    }
    Html(format!(
        r#"<!doctype html>
<html lang="zh-CN"><head><meta charset="utf-8"><title>LocalAPI 历史记录</title>
<style>
body{{font:14px/1.6 system-ui,"Segoe UI",sans-serif;max-width:860px;margin:40px auto;padding:0 16px;color:#222}}
h1{{font-size:20px}}
table{{border-collapse:collapse;width:100%}}th,td{{border-bottom:1px solid #eee;padding:8px 6px;text-align:left;vertical-align:top}}
th{{color:#666;font-weight:500;font-size:13px}}.empty{{color:#888;text-align:center}}
a{{color:#0b57d0;text-decoration:none}}a:hover{{text-decoration:underline}}
p.back{{margin-top:20px}}
</style></head><body>
<h1>历史记录</h1>
<table>
<tr><th>ID</th><th>时间</th><th>模型</th><th>消息数</th><th>操作</th></tr>
{rows}
</table>
<p class="back"><a href="/control">返回控制台</a></p>
</body></html>"#
    ))
}

/// `GET /history/{id}`：导出指定会话为 Markdown（浏览器直接查看）。
pub async fn OutportSession(State(st): State<Arc<AppState>>, Path(id): Path<u64>) -> Response {
    let history = st.history.lock().await;
    match history.OutportMarkdown(id) {
        Some(markdown) => {
            let mut response = markdown.into_response();
            response.headers_mut().insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/markdown; charset=utf-8"),
            );
            response
        }
        None => (StatusCode::NOT_FOUND, format!("会话 {id} 不存在")).into_response(),
    }
}

/// `GET /history/{id}/download`：以附件形式下载指定会话的 JSON 文件。
pub async fn DownloadSession(State(st): State<Arc<AppState>>, Path(id): Path<u64>) -> Response {
    let history = st.history.lock().await;
    let body = match history.OutportJson(id) {
        Some(text) => text,
        None => return (StatusCode::NOT_FOUND, format!("会话 {id} 不存在")).into_response(),
    };
    let mut response = body.into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json; charset=utf-8"),
    );
    if let Ok(value) = HeaderValue::from_str(&format!("attachment; filename=\"session-{id}.json\""))
    {
        response
            .headers_mut()
            .insert(header::CONTENT_DISPOSITION, value);
    }
    response
}

fn format_time(secs: u64) -> String {
    if secs == 0 {
        return "-".to_string();
    }
    chrono::DateTime::from_timestamp(secs as i64, 0)
        .map(|dt| {
            dt.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M:%S")
                .to_string()
        })
        .unwrap_or_else(|| secs.to_string())
}
