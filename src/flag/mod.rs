// MIT License
//
// Copyright (c) 2026 Bury
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in all
// copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
// SOFTWARE.

use std::path::{Path, PathBuf};

use clap::{Args, Parser, Subcommand};

use crate::api::router::BuildRouter;
use crate::app_root;
use crate::app_status::AppState;
use crate::config::config::{self, Config};

/// 无子命令时默认按 `run` 处理，保持旧版直接双击启动的行为。
#[derive(Debug, Parser)]
#[command(name = "LocalAPI", version, about = "OpenAI 兼容的本地网页网关")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// 启动 HTTP 网关
    #[command(alias = "serve")]
    Run,

    /// 在程序根目录生成默认 config.json（已存在时需 --force）
    #[command(alias = "Init")]
    Init(InitArgs),

    /// 清空运行数据/缓存（浏览器 profile、Cookie、历史、校准数据）
    #[command(alias = "Clean")]
    Clean(CleanArgs),
}

#[derive(Debug, Args)]
struct InitArgs {
    /// 覆盖已存在的配置文件
    #[arg(long)]
    force: bool,
}

#[derive(Debug, Args)]
struct CleanArgs {
    /// 同时清空日志目录
    #[arg(long)]
    logs: bool,

    /// 只打印将要删除的目录，不实际删除
    #[arg(long)]
    dryRun: bool,
}

/// 解析参数并执行对应子命令。
pub async fn Dispatch() {
    let cli = Cli::parse();
    match cli.command.unwrap_or(Command::Run) {
        Command::Run => runGateway().await,
        Command::Init(args) => {
            if let Err(err) = initCmd(args) {
                fail(&err);
            }
        }
        Command::Clean(args) => {
            if let Err(err) = cleanCmd(args) {
                fail(&err);
            }
        }
    }
}

fn fail(err: &str) -> ! {
    eprintln!("错误: {err}");
    std::process::exit(1);
}

/// `run`：启动网关。配置/数据/日志全部固定在 exe 所在目录。
async fn runGateway() {
    let configPath = app_root::ConfigPath();

    let (cfg, firstRun) = if configPath.is_file() {
        match Config::ReadFromJson(&configPath.to_string_lossy()) {
            Ok(cfg) => {
                (cfg, false)
            }
            Err(err) => fail(&err),
        }
    } else {
        // 第一次使用：内置默认配置作为模板，先落盘保存，再据此启动引导页面
        let cfg = Config::default();
        if let Err(err) = saveConfig(&configPath, &cfg) {
            fail(&err);
        }
        println!("首次运行：已生成默认配置 {}", configPath.display());
        (cfg, true)
    };
    if let Err(err) = config::InitConfig(cfg) {
        fail(&err);
    }
    //上一个cfg用于初始化,这里再拿一个引用
    let cfg = config::GetConfig();

    // 日志初始化失败不阻断网关,使用stderr控制台
    if let Err(err) = Config::InitLog() {
        eprintln!("{err}");
    }

    tracing::info!(
        config = %configPath.display(),
        root = %app_root::Root().display(),
        "config loaded"
    );

    // 配置瑕疵只提示；具体路径/驱动问题在真正请求时再报，便于先进控制台看状态
    if let Err(err) = cfg.Check() {
        eprintln!("{err}");
    }

    let state = AppState::new();
    let app = BuildRouter(state.clone());
    let addr = cfg.HostAndPortOrPanic();

    let host = cfg.Host.clone();
    let control_url = format!("http://{host}/control");
    println!("请访问: {control_url} 开始");

    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!(%addr, error = %e, "bind failed");
            std::process::exit(1);
        }
    };

    // 首次运行：监听已经就绪，打开新手引导页（与控制台 `/control` 不同）
    if firstRun {
        let guide_url = format!("http://{host}/guide");
        if let Err(err) = open::that(&guide_url) {
            eprintln!("无法自动打开浏览器，请手动访问 {guide_url}: {err}");
        }
    }
    if let Err(err) = open::that(&control_url) {
        eprintln!("无法自动打开浏览器，请手动访问 {control_url}: {err}");
    }
    tracing::info!(
        %addr,
        site = %cfg.SiteName(),
        driver = %app_root::DriverPath(&cfg.SiteName()).display(),
        base = %app_root::DataDir().display(),
        headless = cfg.Headless,
        timeout_secs = cfg.TimeoutSecs,
        max_concurrent = cfg.MaxConcurrent,
        "gateway listening"
    );

    // 顺序很重要：先停止接收新请求，等在途请求结束，最后再收掉 node 驱动
    let shutdownState = state.clone();
    let serve = axum::serve(listener, app).with_graceful_shutdown(async move {
        let _ = tokio::signal::ctrl_c().await;
        tracing::info!("shutdown signal received");
    });
    if let Err(e) = serve.await {
        tracing::error!(error = %e, "server error");
    }
    shutdownState.manager.shutdown().await;
}

/// `init`：在程序根目录写一份默认配置。
fn initCmd(args: InitArgs) -> Result<(), String> {
    let path = app_root::ConfigPath();
    // 配置里有用户手调的代理/浏览器路径，默认不覆盖，--force 才视为明确同意
    if path.exists() && !args.force {
        return Err(format!("{} 已存在；如需覆盖请加 --force", path.display()));
    }

    // 模板跟着默认配置走：新增字段不会漏，也不会出现手抄导致的漂移
    saveConfig(&path, &Config::default())?;

    println!("已初始化配置: {}", path.display());
    Ok(())
}

/// 把配置按模板格式（缩进 JSON）写入指定路径。
fn saveConfig(path: &Path, cfg: &Config) -> Result<(), String> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("创建目录 {} 失败: {e}", parent.display()))?;
    }
    let text = serde_json::to_string_pretty(cfg).map_err(|e| format!("序列化配置失败: {e}"))?;
    std::fs::write(path, format!("{text}\n"))
        .map_err(|e| format!("写入 {} 失败: {e}", path.display()))?;
    Ok(())
}

/// `clean`：删除 `<root>/data`；`--logs` 时连 `<root>/logs` 一起删。
fn cleanCmd(args: CleanArgs) -> Result<(), String> {
    let root = app_root::Root().to_path_buf();

    // data 里是 profile/Cookie/历史/校准；日志默认保留，排查问题时还有据可查
    let mut targets = vec![app_root::DataDir()];
    if args.logs {
        targets.push(app_root::LogDir());
    }

    for target in targets {
        checkCleanTarget(&target, &root)?;
        if !target.exists() {
            println!("跳过（不存在）: {}", target.display());
            continue;
        }
        if !target.is_dir() {
            return Err(format!("{} 不是目录，拒绝清理", target.display()));
        }
        if args.dryRun {
            println!("将删除: {}", target.display());
        } else {
            std::fs::remove_dir_all(&target)
                .map_err(|e| format!("删除 {} 失败: {e}", target.display()))?;
            println!("已删除: {}", target.display());
        }
    }
    Ok(())
}

/// 防呆：只允许清理程序根目录之下、且不等于根目录本身的位置。
fn checkCleanTarget(target: &Path, root: &Path) -> Result<(), String> {
    // 目标可能尚未创建：取不到真实位置时退回绝对路径，防呆仍能生效
    let abs = |p: &Path| -> PathBuf {
        std::fs::canonicalize(p).unwrap_or_else(|_| {
            if p.is_absolute() {
                p.to_path_buf()
            } else {
                root.join(p)
            }
        })
    };

    // 按真实落盘位置比较：".."、符号链接或大小写差异都不能绕过防呆
    let targetAbs = abs(target);
    let rootAbs = abs(root);

    if targetAbs == rootAbs {
        return Err(format!("拒绝清理程序根目录: {}", targetAbs.display()));
    }
    if !targetAbs.starts_with(&rootAbs) {
        return Err(format!(
            "程序根目录之外的路径: {}（根目录 {}）",
            targetAbs.display(),
            rootAbs.display()
        ));
    }
    if targetAbs.parent().is_none() {
        return Err(format!("拒绝清理磁盘根目录: {}", targetAbs.display()));
    }
    Ok(())
}