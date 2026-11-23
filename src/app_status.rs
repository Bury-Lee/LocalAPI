//! 进程内共享状态：所有 handler 通过 axum 的 `State<Arc<AppState>>` 拿到它。

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{Mutex, Semaphore};

use crate::app_root;
use crate::config::config::{Config, GetConfig};
use crate::driver::{DriverManager, DriverOptions};
use crate::model::history::History;

// 请求 id 通过 request extensions 在中间件与 handler 之间传递；
// Extension<T> 提取器取的就是 req.extensions() 里的值，所以 T 需要 Clone。
#[derive(Clone)]
pub struct ReqId(pub String);

pub struct AppState {
    // 懒启动的 node 驱动,第一次用到时才开始 spawn 浏览器
    pub manager: Arc<DriverManager>,
    // 其他厂商的驱动:按 site 懒创建并复用（/login?site=qwen 等）
    managers: Mutex<HashMap<String, Arc<DriverManager>>>,
    // 限制同时占用浏览器的请求数（网页端只适合串行）
    pub sem: Arc<Semaphore>,
    // 会话历史：每个 api 调用一条，便于导出聊天记录
    pub history: Mutex<History>,
    // 配置是全局 OnceLock 里的静态引用
    pub cfg: &'static Config,
}

impl AppState {
    pub fn new() -> Arc<AppState> {
        let cfg = GetConfig();
        let manager = DriverManager::new(
            app_root::DriverPath(&cfg.SiteName()),
            app_root::DataDir(),
            driver_options(cfg),
        );
        // 历史文件坏了不影响网关，提示后从空历史开始
        let mut history = History::WithPath(app_root::DataDir().join("history"));
        if let Err(err) = history.Load() {
            eprintln!("{err}");
        }
        Arc::new(AppState {
            manager,
            managers: Mutex::new(HashMap::new()),
            sem: Arc::new(Semaphore::new(cfg.MaxConcurrent.max(1))),
            history: Mutex::new(history),
            cfg,
        })
    }

    /// 取某个厂商的驱动管理器：当前配置的站点复用主 manager，
    /// 其他厂商按 `<root>/cjs/<site>-driver.cjs` 懒启动并缓存。
    pub async fn manager_for_site(&self, site: &str) -> Result<Arc<DriverManager>, String> {
        let site = site.trim().to_ascii_lowercase();
        if site.is_empty() {
            return Err("site 不能为空（deepseek/qwen/doubao/gpt）".to_string());
        }
        if site == self.cfg.SiteName().to_ascii_lowercase() {
            return Ok(self.manager.clone());
        }

        let driver_path = app_root::DriverPath(&site);
        if !driver_path.is_file() {
            return Err(format!(
                "厂商 {site} 的驱动脚本不存在: {}（应位于程序根目录的 cjs/ 下）",
                driver_path.display()
            ));
        }

        let mut managers = self.managers.lock().await;
        if let Some(manager) = managers.get(&site) {
            return Ok(manager.clone());
        }
        let manager = DriverManager::new(driver_path, app_root::DataDir(), driver_options(self.cfg));
        managers.insert(site, manager.clone());
        Ok(manager)
    }
}

/// 组装下发驱动的运行时参数（chrome 路径已按程序根目录解析）。
fn driver_options(cfg: &Config) -> DriverOptions {
    DriverOptions {
        chrome_path: cfg
            .ChromePath
            .as_deref()
            .and_then(app_root::ResolveChrome)
            .map(|p| p.to_string_lossy().into_owned()),
        proxy: cfg.Proxy.trim().to_string(),
    }
}
