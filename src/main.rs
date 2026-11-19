mod api;
pub mod app_status;
pub mod config;
pub mod driver;
pub mod openai;

use api::router::BuildRouter;
use app_status::AppState;
use config::config::{Config, LoadDefaultConfig};

#[tokio::main]
async fn main() {
    // 配置查找：DSWEB_CONFIG → ./config.json → 全局；相对路径一律按“启动目录”解析
    let (cfg, config_path) = LoadDefaultConfig();

    if let Err(err) = Config::InitLog() {
        eprintln!("{err}");
    }

    tracing::info!(
        config = %config_path.display(),
        cwd = %config::config::LaunchDir().display(),
        project = %config::config::ProjectDir().display(),
        "config loaded"
    );

    if let Err(err) = cfg.Check() {
        eprintln!("{err}");
    }

    // 共享状态
    let state = AppState::new();

    // 启动 Axum
    let app = BuildRouter(state.clone());
    let addr = cfg.HostAndPortOrPanic();

    //打印控制台地址
    println!("访问: http://{}/control 来查看",cfg.Host);

    // 监听地址
    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!(%addr, error = %e, "bind failed");
            std::process::exit(1);
        }
    };

    tracing::info!(
        %addr,
        site = %cfg.SiteName(),
        driver = %config::config::ResolvePath(&cfg.Driver).display(),
        base = %config::config::ResolvePath(&cfg.Base).display(),
        headless = cfg.Headless,
        timeout_secs = cfg.TimeoutSecs,
        max_concurrent = cfg.MaxConcurrent,
        "gateway listening"
    );

    // 收到 Ctrl+C 后停止接受新连接，等在途请求处理完，再收掉 node 驱动
    let shutdown_state = state.clone();
    let serve = axum::serve(listener, app).with_graceful_shutdown(async move {
        let _ = tokio::signal::ctrl_c().await;
        tracing::info!("shutdown signal received");
    });
    if let Err(e) = serve.await {
        tracing::error!(error = %e, "server error");
    }
    shutdown_state.manager.shutdown().await;
}
