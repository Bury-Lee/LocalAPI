use std::path::{Path, PathBuf};

use clap::{Args, Parser, Subcommand};

use crate::api::router::BuildRouter;
use crate::app_status::AppState;
use crate::config::config::{
    self, Config, DefaultConfigPath, LoadDefaultConfig, ProjectDir, ResolvePath, UserConfigPath,
};

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

    /// 生成默认 config.json（已存在时需 --force）
    #[command(alias = "Init")]
    Init(InitArgs),

    /// 清空运行数据/缓存（浏览器 profile、Cookie、校准数据）
    #[command(alias = "Clean")]
    Clean(CleanArgs),
}

#[derive(Debug, Args)]
struct InitArgs {
    /// 写入指定路径；相对路径按当前目录解析
    #[arg(long, value_name = "FILE")]
    path: Option<PathBuf>,

    /// 写入用户级配置（%APPDATA%\dsh-web-gateway\config.json）
    #[arg(long)]
    global: bool,

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

/// `run`：原有的启动流程。
async fn runGateway() {
    // 配置查找：DSWEB_CONFIG → ./config.json → 全局；相对路径一律按“启动目录”解析
    let (cfg, configPath) = LoadDefaultConfig();

    // 日志初始化失败不阻断网关：stderr/控制台还能用，先让服务起来
    if let Err(err) = Config::InitLog() {
        eprintln!("{err}");
    }

    tracing::info!(
        config = %configPath.display(),
        cwd = %config::LaunchDir().display(),
        project = %config::ProjectDir().display(),
        "config loaded"
    );

    // 配置瑕疵只提示；具体路径/驱动问题在真正请求时再报，便于先进控制台看状态
    if let Err(err) = cfg.Check() {
        eprintln!("{err}");
    }

    let state = AppState::new();
    let app = BuildRouter(state.clone());
    let addr = cfg.HostAndPortOrPanic();

    println!("请访问: http://{}/control 开始", cfg.Host);

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
        driver = %config::ResolvePath(&cfg.Driver).display(),
        base = %config::ResolvePath(&cfg.Base).display(),
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

/// `init`：写一份默认配置。
fn initCmd(args: InitArgs) -> Result<(), String> {
    let path = initTarget(&args)?;
    // 配置里可能有用户手调的代理/路径，默认不覆盖，--force 才视为明确同意
    if path.exists() && !args.force {
        return Err(format!(
            "{} 已存在；如需覆盖请加 --force",
            path.display()
        ));
    }

    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("创建目录 {} 失败: {e}", parent.display()))?;
    }

    // 模板跟着默认配置走：新增字段不会漏，也不会出现手抄导致的漂移
    let text =
        serde_json::to_string_pretty(&Config::default()).map_err(|e| format!("序列化默认配置失败: {e}"))?;
    std::fs::write(&path, format!("{text}\n"))
        .map_err(|e| format!("写入 {} 失败: {e}", path.display()))?;

    println!("已初始化配置: {}", path.display());
    Ok(())
}

/// 目标优先级：`--path` > `--global` > `DSWEB_CONFIG` > 项目根；与运行时查找配置的顺序对齐。
fn initTarget(args: &InitArgs) -> Result<PathBuf, String> {
    if let Some(path) = &args.path {
        // 显式路径跟随当前目录；只有配置里的相对路径才锚定项目根
        return if path.is_absolute() {
            Ok(path.to_path_buf())
        } else {
            std::env::current_dir()
                .map(|cwd| cwd.join(path))
                .map_err(|e| format!("获取当前目录失败: {e}"))
        };
    }
    if args.global {
        return Ok(UserConfigPath());
    }
    if let Some(path) = std::env::var("DSWEB_CONFIG")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
    {
        return Ok(ResolvePath(&path));
    }
    Ok(ProjectDir().join(DefaultConfigPath))
}

/// `clean`：删除运行时数据目录；`--logs` 时连日志目录一起删。
fn cleanCmd(args: CleanArgs) -> Result<(), String> {
    // 清理必须对准运行时真正使用的目录：环境变量或外部配置可能已经改过位置
    let (cfg, _) = LoadDefaultConfig();
    let project = ProjectDir();

    // Base 里是 profile/Cookie/校准；日志默认保留，排查问题时还有据可查
    let mut targets = vec![cfg.BasePath()];
    if args.logs {
        targets.push(ResolvePath(&cfg.LogDir));
    }

    for target in targets {
        checkCleanTarget(&target, &project)?;
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

/// 防呆：不清理项目根目录，也不清理“包含项目”的上级目录。
fn checkCleanTarget(target: &Path, project: &Path) -> Result<(), String> {
    // 目标可能尚未创建：取不到真实位置时退回绝对路径，防呆仍能生效
    let abs = |p: &Path| -> PathBuf {
        std::fs::canonicalize(p).unwrap_or_else(|_| {
            if p.is_absolute() {
                p.to_path_buf()
            } else {
                std::env::current_dir()
                    .map(|cwd| cwd.join(p))
                    .unwrap_or_else(|_| p.to_path_buf())
            }
        })
    };

    // 按真实落盘位置比较：".."、符号链接或大小写差异都不能绕过防呆
    let targetAbs = abs(target);
    let projectAbs = abs(project);

    if targetAbs == projectAbs {
        return Err(format!("拒绝清理项目根目录: {}", targetAbs.display()));
    }
    if projectAbs.starts_with(&targetAbs) {
        return Err(format!(
            "拒绝清理包含项目的目录: {}\n项目位于 {}",
            targetAbs.display(),
            projectAbs.display()
        ));
    }
    if targetAbs.parent().is_none() {
        return Err(format!("拒绝清理磁盘根目录: {}", targetAbs.display()));
    }
    Ok(())
}
