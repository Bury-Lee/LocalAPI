use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// 固定配置文件名（位于根目录）。
pub const ConfigFileName: &str = "config.json";

static ROOT: OnceLock<PathBuf> = OnceLock::new();

/// 程序根目录：可执行文件所在目录。
pub fn Root() -> &'static Path {
    ROOT.get_or_init(|| {
        let exe = std::env::current_exe().expect("无法获取 LocalAPI.exe 的路径");
        let dir = exe
            .parent()
            .expect("LocalAPI.exe 没有上级目录，无法确定程序根目录");
        dir.to_path_buf()
    })
}

/// `<root>/config.json`：配置的位置。
pub fn ConfigPath() -> PathBuf {
    Root().join(ConfigFileName)
}

/// `<root>/data`：运行时数据的位置（profile、Cookie、历史、校准）。
pub fn DataDir() -> PathBuf {
    Root().join("data")
}

/// `<root>/logs`：日志目录。
pub fn LogDir() -> PathBuf {
    Root().join("logs")
}

/// `<root>/cjs`：驱动脚本目录。
pub fn CjsDir() -> PathBuf {
    Root().join("cjs")
}

/// `<root>/cjs/<site>-driver.cjs`。
pub fn DriverPath(site: &str) -> PathBuf {
    CjsDir().join(format!("{site}-driver.cjs"))
}

/// 解析可选的浏览器路径：如果是相对路径按根目录展开，否则按绝对路径使用
pub fn ResolveChrome(value: &str) -> Option<PathBuf> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    let path = PathBuf::from(value);
    Some(if path.is_absolute() {
        path
    } else {
        Root().join(path)
    })
}
