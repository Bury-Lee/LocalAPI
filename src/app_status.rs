//! 进程内共享状态：所有 handler 通过 axum 的 `State<Arc<AppState>>` 拿到它。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::{Mutex, Semaphore};

use crate::config::config::{Config, GetConfig};
use crate::driver::DriverManager;
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
        // 相对路径按启动目录补全：driver 脚本与 `.data`
        let manager = DriverManager::new(
            PathBuf::from(&cfg.Driver),
            PathBuf::from(&cfg.Base),
            cfg.DriverEnv(),
        );
        // 历史文件坏了不影响网关，提示后从空历史开始
        let mut history = History::default();
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
    /// 其他厂商按 `cjs/<site>-driver.cjs` 懒启动并缓存。
    pub async fn manager_for_site(
        &self,
        site: &str,
        driver: Option<PathBuf>,
    ) -> Result<Arc<DriverManager>, String> {
        let site = site.trim().to_ascii_lowercase();
        let configured = self.cfg.SiteName().to_ascii_lowercase();
        if driver.is_none() && (site.is_empty() || site == configured) {
            return Ok(self.manager.clone());
        }
        if site.is_empty() {
            return Err("site 不能为空（deepseek/qwen/doubao/gpt）".to_string());
        }

        let driver_path =
            driver.unwrap_or_else(|| PathBuf::from(format!("cjs/{site}-driver.cjs")));
        if !driver_path.is_file() {
            return Err(format!(
                "厂商 {} 的驱动脚本不存在: {}（需要 cjs/{}-driver.cjs）",
                site,
                driver_path.display(),
                site
            ));
        }
        let key = format!("{site}|{}", driver_path.display());

        let mut managers = self.managers.lock().await;
        if let Some(manager) = managers.get(&key) {
            return Ok(manager.clone());
        }
        // 登录必须是有头浏览器；同一个 base 下各厂商的 profile 目录互不冲突：
        // deepseek 用 profiles/default，多站点驱动用 profiles/<site>-<profile>。
        //TODO:解决一下历史遗留问题,这里应该使用统一的env
        let mut envs = self.cfg.DriverEnv();
        set_env(&mut envs, "DS_WEB_SITE", &site);
        set_env(&mut envs, "DSWEB_HEADLESS", "0");
        let manager = DriverManager::new(driver_path, PathBuf::from(&self.cfg.Base), envs);
        managers.insert(key, manager.clone());
        Ok(manager)
    }
}

fn set_env(envs: &mut Vec<(String, String)>, key: &str, value: &str) {
    if let Some(entry) = envs.iter_mut().find(|(k, _)| k == key) {
        entry.1 = value.to_string();
    } else {
        envs.push((key.to_string(), value.to_string()));
    }
}
