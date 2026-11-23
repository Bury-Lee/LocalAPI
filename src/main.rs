mod api;
pub mod app_root;
pub mod app_status;
pub mod config;
pub mod driver;
pub mod flag;
pub mod model;
pub mod openai;

/// CLI 解析与子命令都放在 flag 模块
#[tokio::main]
async fn main() {
    flag::Dispatch().await;//调用命令行主程序
}
