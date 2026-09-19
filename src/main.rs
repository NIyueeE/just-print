//! Just Print — 基于 Rust + Preact 构建的打印机 Web 服务容器。
//!
//! 容器是唯一交付形态：后端提供 `Web API` 与静态前端，镜像内置 `LibreOffice`
//! 用于文档转 `PDF`，打印任务通过标准 IPP 直接提交给容器内 `CUPS` 排队与执行。

mod api;
mod config;
mod conversion;
mod cups;
mod error;
mod idempotency;
mod ids;
mod jobs;
mod metrics;
mod state;
mod store;

use std::process::ExitCode;
use std::sync::Arc;

use tokio::signal;
use tracing::info;
use tracing_subscriber::EnvFilter;

use crate::config::Config;
use crate::state::{AppState, TempDir};

/// 程序入口：初始化配置、存储与后台清理任务后启动 HTTP 服务。
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
    let state = Arc::new(AppState::new(config.clone(), temp_dir.root)?);
    refresh_gauges(&state);

    let cleanup_state = Arc::clone(&state);
    let cleanup_interval = config.cleanup_interval;
    let temp_ttl = config.temp_ttl;
    let _cleanup = tokio::spawn(async move {
        loop {
            tokio::time::sleep(cleanup_interval).await;
            let removed = cleanup_state.files.cleanup(temp_ttl);
            cleanup_state.idempotency.prune();
            cleanup_state.jobs.prune();
            refresh_gauges(&cleanup_state);
            if removed > 0 {
                info!(removed, "临时文件清理完成");
            }
        }
    });

    let app = api::router(&state);
    let listener = tokio::net::TcpListener::bind(config.addr).await?;
    info!(
        addr = %config.addr,
        web_dir = %config.web_dir.display(),
        cups_server = %config.cups_server,
        "just-print 已启动"
    );
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

/// 刷新与运行期状态相关的仪表指标。
fn refresh_gauges(state: &AppState) {
    state.metrics.set_gauge(
        "uploaded_files",
        &[],
        i64::try_from(state.files.len()).unwrap_or(i64::MAX),
    );
    state.metrics.set_gauge(
        "uploaded_bytes",
        &[],
        i64::try_from(state.files.total_bytes()).unwrap_or(i64::MAX),
    );
    state.metrics.set_gauge(
        "registered_jobs",
        &[],
        i64::try_from(state.jobs.len()).unwrap_or(i64::MAX),
    );
    state.metrics.set_gauge(
        "idempotency_entries",
        &[],
        i64::try_from(state.idempotency.len()).unwrap_or(i64::MAX),
    );
}

/// 等待 SIGINT / SIGTERM（容器停止信号），随后优雅退出。
async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        let result = signal::unix::signal(signal::unix::SignalKind::terminate());
        let Ok(mut terminate) = result else {
            return;
        };
        let _ = terminate.recv().await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        () = ctrl_c => {}
        () = terminate => {}
    }
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();
}
