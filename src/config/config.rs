use serde::{Deserialize, Serialize};
use std::env;
use std::fs::{create_dir_all, read_to_string, OpenOptions};
use std::io;
use std::net::SocketAddr;
use std::net::ToSocketAddrs;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

/// 默认配置文件路径（相对启动目录）
pub const DefaultConfigPath: &str = "config.json";
/// 全局配置目录名：`%APPDATA%\dsh-web-gateway\config.json`
pub const AppDirName: &str = "dsh-web-gateway";





#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// 监听地址
    pub Host: String,
    /// 最小记录等级,迟点改为日志库枚举
    pub LogLeastLevel: String,
    /// 日志目录,里面按本地日期建当日文件
    pub LogDir: String,
    /// 网页驱动入口脚本(相对启动目录),如 cjs/qwen-driver.cjs
    pub Driver: String,
    /// 站点标识,用于默认驱动与 profile 命名:deepseek/qwen/doubao/gpt
    pub Site: String,
    /// 运行时数据目录(浏览器 profile、校准、Cookie)
    pub Base: String,
    /// 是否无头浏览器;首次登录必须为 false
    pub Headless: bool,
    /// 等待网页端回答的秒数
    pub TimeoutSecs: u64,
    /// 等待人工登录的秒数
    pub LoginWaitSecs: u64,
    /// 同时占用浏览器的请求数(网页端只适合串行,建议 1)
    pub MaxConcurrent: usize,
    /// 指定浏览器可执行文件路径;空则自动选择
    pub ChromePath: Option<String>,
    /// 浏览器代理,如 http://127.0.0.1:7890;空则不用
    pub Proxy: String,
    /// 是否忽略请求里的 tools,避免网页端输出 tool_call 格式
    pub IgnoreTools: bool,
    ///速率控制.为null时自动(预备字段)
    pub SpeedControl:Option<u8>
}

impl Default for Config {
    fn default() -> Self {
        Config {
            Host: "127.0.0.1:5688".into(),
            LogLeastLevel: "".into(),
            LogDir: "logs".into(),
            Driver: "cjs/deepseek-driver.cjs".into(),
            Site: "".into(),
            Base: ".data".into(),
            Headless: true,
            TimeoutSecs: 240,
            LoginWaitSecs: 300,
            MaxConcurrent: 1,
            ChromePath: None,
            Proxy: "".into(),
            IgnoreTools: false,
            SpeedControl:None
        }
    }
}

impl Config {
    /// 从 JSON 文件读取配置（容忍 Windows 记事本写入的 UTF-8 BOM）。
    pub fn ReadFromJson(path: &str) -> Result<Config, String> {
        let text = read_to_string(path).map_err(|e| format!("读取配置文件失败 {path}: {e}"))?;
        let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
        let cfg: Config =
            serde_json::from_str(text).map_err(|e| format!("解析配置文件失败 {path}: {e}"))?;
        Ok(cfg)
    }

    /// 先读 JSON(失败则用默认值),再用环境变量覆盖。
    /// 环境变量优先级最高,方便临时切换驱动/站点而不改文件。
    pub fn Load(path: &str) -> Config {
        let mut cfg = Config::ReadFromJson(path).unwrap_or_default();
        cfg
    }

    /// 启动前做一次静态校验:地址可解析、驱动脚本存在。
    pub fn Check(&self) -> Result<(), String> {
        self.HostAndPort()
            .map_err(|e| format!("Host {:?} 无法解析为 SocketAddr: {e}", self.Host))?;
        let driver = self.DriverPath();
        if !driver.is_file() {
            return Err(format!(
                "驱动脚本不存在: {}（DSWEB_DRIVER 或 config.json 的 Driver 需指向 cjs/*.cjs，且必须在项目根目录启动）",
                driver.display()
            ));
        }
        Ok(())
    }

    /// 解析出监听地址。
    pub fn HostAndPort(&self) -> io::Result<SocketAddr> {
        self.Host
            .to_socket_addrs()?
            .next()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Host 解析结果为空"))
    }

    pub fn HostAndPortOrPanic(&self) -> SocketAddr {
        self.HostAndPort()
            .unwrap_or_else(|e| panic!("配置的 Host {:?} 无法解析为 SocketAddr: {e}", self.Host))
    }

    /// 驱动脚本路径（相对当前目录）。
    pub fn DriverPath(&self) -> PathBuf {
        PathBuf::from(&self.Driver)
    }

    /// 站点标识：显式配置优先，否则从驱动文件名推断（qwen-driver.cjs -> qwen）。
    pub fn SiteName(&self) -> String {
        let site = self.Site.trim();
        if !site.is_empty() {
            return site.to_string();
        }
        let file = self
            .Driver
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        for known in ["deepseek", "qwen", "doubao", "gpt"] {
            if file.starts_with(known) {
                return known.to_string();
            }
        }
        "deepseek".to_string()
    }

    /// 运行时数据目录。
    pub fn BasePath(&self) -> PathBuf {
        PathBuf::from(&self.Base)
    }

    /// 浏览器 profile 目录:<base>/profiles/<site>-<profile>
    pub fn ProfileDir(&self, site: &str, profile: &str) -> PathBuf {
        let site = if site.trim().is_empty() { self.SiteName() } else { site.trim().to_string() };
        let profile = if profile.trim().is_empty() { "default" } else { profile.trim() };
        self.BasePath().join("profiles").join(format!("{site}-{profile}"))
    }

    /// 等待网页端回答的超时。
    pub fn Timeout(&self) -> Duration {
        Duration::from_secs(self.TimeoutSecs.max(1))
    }

    /// 等待人工登录的超时。
    pub fn LoginWait(&self) -> Duration {
        Duration::from_secs(self.LoginWaitSecs.max(1))
    }

    /// 传给驱动子进程的环境变量(驱动侧读 DS_WEB_*,网关侧读 DSWEB_*)。
    /// `DS_WEB_BASE` 由 driver.rs 以绝对路径注入，这里不重复。
    pub fn DriverEnv(&self) -> Vec<(String, String)> {
        let mut out = vec![
            ("DS_WEB_SITE".to_string(), self.SiteName()),
            ("DSWEB_HEADLESS".to_string(), if self.Headless { "1".into() } else { "0".into() }),
        ];
        if let Some(chrome) = self.ChromePath.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            out.push(("DS_WEB_CHROME".to_string(), chrome.to_string()));
        }
        if !self.Proxy.trim().is_empty() {
            out.push(("DS_WEB_PROXY".to_string(), self.Proxy.clone()));
        }
        if self.IgnoreTools {
            out.push(("DSWEB_IGNORE_TOOLS".to_string(), "1".to_string()));
        }
        out
    }

    /// 初始化日志：建日志目录，按本地日期建当日文件并作为 tracing 输出。
    pub fn InitLog() -> Result<(), String> {
        use chrono::Local;
        use tracing_subscriber::{fmt, EnvFilter};

        let cfg = GetConfig();
        let raw = if cfg.LogDir.trim().is_empty() { "logs" } else { cfg.LogDir.trim() };
        let dir = PathBuf::from(raw);
        create_dir_all(&dir).map_err(|e| format!("创建日志目录 {} 失败: {e}", dir.display()))?;

        let path = dir.join(format!("{}.log", Local::now().format("%Y-%m-%d")));
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| format!("创建日志文件 {} 失败: {e}", path.display()))?;

        let level = cfg.LogLeastLevel.trim().to_string();
        let filter = if level.is_empty() {
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"))
        } else {
            EnvFilter::try_new(&level).map_err(|e| format!("日志级别 {level} 非法: {e}"))?
        };

        fmt()
            .with_env_filter(filter)
            .with_ansi(false)
            .with_writer(file)
            .try_init()
            .map_err(|e| format!("初始化日志失败: {e}"))?;

        Ok(())
    }
}


static GlobalConfig: OnceLock<Config> = OnceLock::new();

/// 初始化：仅第一次调用生效。
/// 成功返回 Ok(())；若失败,则报错并记录日志
pub fn InitConfig(cfg: Config) -> Result<(), String> {
    match GlobalConfig.set(cfg) {
        Ok(()) => return Ok(()),
        Err(err) => {
            return Err(format!("初始化配置失败：配置已经初始化过（{:?}）", err));
        }
    }
}

/// 读取：未初始化时用 Default 兜底，返回 &'static Config（零拷贝）。
pub fn GetConfig() -> &'static Config {
    GlobalConfig.get_or_init(Config::default)
}

pub fn TryUpdateConfig(cfg: Config) -> &'static Config {
    GlobalConfig.get_or_init(|| cfg)
}

/// 读配置文件 + 环境变量覆盖 + 装入全局；重复调用返回已装入的那份。
pub fn LoadConfig(path: &str) -> &'static Config {
    let _ = GlobalConfig.set(Config::Load(path));
    GetConfig()
}

///保存配置
pub fn SaveConfig() {
    print!("TODO:保存配置")
}
