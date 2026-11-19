mod api;
pub mod app_status;
pub mod config;
pub mod driver;
pub mod flag;
pub mod openai;

// CLI 解析与子命令都在 flag 模块，main 只做入口转发
#[tokio::main]
async fn main() {
    flag::Dispatch().await;
}
