//! Just Print — 一个基于 Rust + Preact 构建的轻量打印机 Web 服务。
//!
//! 当前处于骨架阶段：容器内提供静态前端与健康检查，打印与 API 业务
//! 按 `README.md` 中的路线图逐步实现。

use std::env;
use std::net::SocketAddr;
use std::path::PathBuf;

use axum::{Router, routing::get};
use tokio::net::TcpListener;
use tower_http::services::{ServeDir, ServeFile};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let addr = env::var("JUST_PRINT_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".into());
    let web_dir =
        env::var("JUST_PRINT_WEB_DIR").unwrap_or_else(|_| "/usr/share/just-print/web".into());
    let addr: SocketAddr = addr.parse()?;
    let index = PathBuf::from(&web_dir).join("index.html");

    let app = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .fallback_service(ServeDir::new(&web_dir).not_found_service(ServeFile::new(index)));

    let listener = TcpListener::bind(addr).await?;
    println!("just-print listening on {addr}, serving web from {web_dir}");
    axum::serve(listener, app).await?;
    Ok(())
}
