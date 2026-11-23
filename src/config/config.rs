use serde::{Deserialize, Serialize};
use std::fs::{create_dir_all, read_to_string, OpenOptions};
use std::io;
use std::net::SocketAddr;
use std::net::ToSocketAddrs;
use std::sync::OnceLock;

use crate::app_root;

/// 运行配置。路径从 [`app_root::Root`] 派生
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// 监听地址
    pub Host: String,
    /// 最小记录等级,迟点改为日志库枚举
    pub LogLeastLevel: String,
    /// 使用的默认站点标识:deepseek/qwen/doubao/gpt
    pub Site: String,
    /// 是否无头浏览器;首次登录必须为 false
    pub Headless: bool,
    /// 等待网页端回答的秒数
    pub TimeoutSecs: u64,
    /// 等待人工登录的秒数
    pub LoginWaitSecs: u64,
    /// 同时占用浏览器的请求数(网页端只适合串行,建议 1)
    pub MaxConcurrent: usize,
    /// 指定浏览器可执行文件:相对程序根目录或绝对路径;空则自动选择
    pub ChromePath: Option<String>,
    /// 浏览器代理,如 http://127.0.0.1:7890;空则不用
    pub Proxy: String,
    /// 是否忽略请求里的 tools,避免网页端输出 tool_call 格式
    pub IgnoreTools: bool,
    ///速率控制.为null时自动(预备字段)
    pub SpeedControl: Option<u8>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            Host: "127.0.0.1:5688".into(),
            LogLeastLevel: "".into(),
            Site: "".into(),
            Headless: true,
            TimeoutSecs: 240,
            LoginWaitSecs: 300,
            MaxConcurrent: 1,
            ChromePath: None,
            Proxy: "".into(),
            IgnoreTools: false,
            SpeedControl: None,
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

    /// 启动前做一次静态校验:地址可解析、驱动脚本存在。
    pub fn Check(&self) -> Result<(), String> {
        self.HostAndPort()
            .map_err(|e| format!("Host {:?} 无法解析为 SocketAddr: {e}", self.Host))?;
        let driver = app_root::DriverPath(&self.SiteName());
        if !driver.is_file() {
            return Err(format!(
                "驱动脚本不存在: {}（应位于程序根目录的 cjs/ 下）",
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

    /// 站点标识：空值回落到 deepseek，其余统一小写。
    pub fn SiteName(&self) -> String {
        let site = self.Site.trim().to_ascii_lowercase();
        if site.is_empty() {
            "deepseek".to_string()
        } else {
            site
        }
    }

    /// 初始化日志：固定写到 `<root>/logs/<本地日期>.log`。
    /// 日志级别只来自配置，不读取 `RUST_LOG` 等环境变量。
    pub fn InitLog() -> Result<(), String> {
        use chrono::Local;
        use tracing_subscriber::{fmt, EnvFilter};

        let cfg = GetConfig();
        let dir = app_root::LogDir();
        create_dir_all(&dir).map_err(|e| format!("创建日志目录 {} 失败: {e}", dir.display()))?;

        let path = dir.join(format!("{}.log", Local::now().format("%Y-%m-%d")));
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| format!("创建日志文件 {} 失败: {e}", path.display()))?;

        let level = cfg.LogLeastLevel.trim();
        let filter = if level.is_empty() {
            EnvFilter::new("info")
        } else {
            EnvFilter::try_new(level).map_err(|e| format!("日志级别 {level} 非法: {e}"))?
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
pub fn InitConfig(cfg: Config) -> Result<(), String> {
    match GlobalConfig.set(cfg) {
        Ok(()) => Ok(()),
        Err(err) => Err(format!("初始化配置失败：配置已经初始化过（{:?}）", err)),
    }
}

/// 读取：未初始化时用 Default 兜底，返回 &'static Config（零拷贝）。
pub fn GetConfig() -> &'static Config {
    GlobalConfig.get_or_init(Config::default)
}
