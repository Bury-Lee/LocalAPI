//! 浏览器页面：新手引导页与控制台。

use std::sync::Arc;

use axum::extract::State;
use axum::response::Html;

use crate::app_status::AppState;

/// 控制台/引导页里展示的厂商列表（与 cjs/<site>-driver.cjs 对应）。
pub(crate) const VENDORS: &[(&str, &str)] = &[
    ("deepseek", "DeepSeek"),
    ("qwen", "通义千问 Qwen"),
    ("doubao", "豆包 Doubao"),
    ("gpt", "ChatGPT"),
];

/// `GET /guide`：首次运行的新手引导页，和 `/control` 控制台区分开。
pub async fn guide(State(st): State<Arc<AppState>>) -> Html<String> {
    let cfg = st.cfg;
    let host = escape_html(&cfg.Host.replace("0.0.0.0", "127.0.0.1"));
    let site = escape_html(&cfg.SiteName());
    let mut vendors = String::new();
    for (id, name) in VENDORS {
        let label = escape_html(name);
        vendors.push_str(&format!(
            r#"<a class="btn" href="/login?site={id}">登录 {label}</a>"#
        ));
    }
    Html(format!(
        r#"<!doctype html>
<html lang="zh-CN"><head><meta charset="utf-8"><title>LocalAPI 新手引导</title>
<style>
body{{font:14px/1.7 system-ui,"Segoe UI",sans-serif;max-width:720px;margin:40px auto;padding:0 16px;color:#222}}
h1{{font-size:22px;margin:0 0 4px}}.sub{{color:#666;margin:0 0 24px}}
.step{{border:1px solid #eee;border-radius:8px;padding:14px 16px;margin-bottom:12px}}
.step h2{{font-size:15px;margin:0 0 6px}}
.n{{display:inline-block;width:22px;height:22px;line-height:22px;text-align:center;border-radius:50%;background:#0b57d0;color:#fff;font-size:12px;margin-right:8px}}
code{{background:#f4f4f4;padding:1px 5px;border-radius:4px}}
.btn{{display:inline-block;margin:8px 8px 0 0;padding:6px 12px;border-radius:6px;background:#0b57d0;color:#fff;text-decoration:none;font-size:13px}}
.btn.ghost{{background:#f4f4f4;color:#0b57d0}}
</style></head><body>
<h1>欢迎使用 LocalAPI</h1>
<p class="sub">检测到这是第一次运行，已自动生成 <code>config.json</code>。按下面三步即可用起来。</p>
<div class="step"><h2><span class="n">1</span>确认站点与驱动</h2>
当前站点：<code>{site}</code>。需要换厂商时，修改 <code>config.json</code> 的 <code>Site</code>/<code>Driver</code> 后重启。
</div>
<div class="step"><h2><span class="n">2</span>登录网页端</h2>
先点下面的按钮，在弹出的浏览器里完成人工登录（首次必须有头窗口）：
<div>{vendors}</div>
</div>
<div class="step"><h2><span class="n">3</span>连接 OpenAI 客户端</h2>
把客户端地址设为 <code>http://{host}/v1</code>，apiKey 随便填即可。
</div>
<p><a class="btn ghost" href="/control">进入控制台</a></p>
</body></html>"#
    ))
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
        vendors.push_str(&format!(r#"<a href="/login?site={id}">{label}</a><br>"#));
    }
    Html(format!(
        r#"<!doctype html>
<html lang="zh-CN"><head><meta charset="utf-8"><title>LocalAPI</title>
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
<tr><td>历史</td><td><a href="/history">/history</a>（查看并导出聊天记录）</td></tr>
<tr><td>状态</td><td><a href="/status">/status</a></td></tr>
<tr><td>模型</td><td><a href="/v1/models">/v1/models</a></td></tr>
<tr><td>对话</td><td><code>POST /v1/chat/completions</code></td></tr>
<tr><td>健康检查</td><td><a href="/health">/health</a></td></tr>
</table>
<p>OpenAI 客户端用 <code>baseURL=http://{host}/v1</code>，apiKey 随便填。</p>
</body></html>"#
    ))
}

/// 页面里只放配置值/历史内容，做最小转义避免破坏标签。
pub(crate) fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
