//! Just Print — 基于 Rust + Preact 构建的轻量打印机 Web 服务。
//!
//! 容器是唯一交付形态：后端提供 `Web API` 与静态前端，镜像内置 `LibreOffice`
//! 用于文档转 `PDF`，`PJL` 层直接与 `/dev/usb/lp*` 通信，不依赖 `CUPS` 或驱动。

mod api;
mod config;
mod conversion;
mod error;
mod ids;
mod pjl;
mod registry;
mod state;
mod store;
mod workers;

use std::process::ExitCode;
use std::sync::Arc;

use tracing::info;
use tracing_subscriber::EnvFilter;

use crate::config::Config;
use crate::state::{AppState, TempDir};

/// 程序入口：初始化配置、存储与后台任务后启动 HTTP 服务。
#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("just-print failed to start: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    init_tracing();
    let config = Config::from_env()?;
    let temp_dir = TempDir::prepare()?;
    let state = Arc::new(AppState::new(config.clone(), temp_dir.root));

    let printers = Arc::clone(&state.printers);
    let _discovery = tokio::spawn(async move { printers.run().await });

    let files = Arc::clone(&state.files);
    let jobs = Arc::clone(&state.jobs);
    let cleanup_interval = config.cleanup_interval;
    let temp_ttl = config.temp_ttl;
    let _cleanup = tokio::spawn(async move {
        loop {
            tokio::time::sleep(cleanup_interval).await;
            let removed = files.cleanup(temp_ttl);
            let pruned = jobs.cleanup();
            if removed > 0 || pruned > 0 {
                info!(removed, pruned, "临时文件与任务记录清理完成");
            }
        }
    });

    let app = api::router(Arc::clone(&state));
    let listener = tokio::net::TcpListener::bind(config.addr).await?;
    info!(addr = %config.addr, web_dir = %config.web_dir.display(), "just-print 已启动");
    axum::serve(listener, app).await?;
    Ok(())
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();
}
