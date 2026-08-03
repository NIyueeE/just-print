//! Web API：路由装配与 Bearer 令牌保护。

pub mod auth;
pub mod files;
pub mod jobs;
pub mod print;
pub mod printers;

use std::sync::Arc;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::middleware;
use axum::routing::{get, post};
use tower_http::services::{ServeDir, ServeFile};

use crate::state::AppState;

/// 构建完整路由：`/api/*` 业务接口 + 静态前端 fallback。
pub fn router(state: Arc<AppState>) -> Router {
    let web_dir = state.config.web_dir.clone();
    let index = web_dir.join("index.html");
    let api = Router::new()
        .route("/files", post(files::upload))
        .route("/files/{id}", get(files::preview))
        .route("/printers", get(printers::list))
        .route("/print", post(print::submit))
        .route("/jobs/{id}", get(jobs::get))
        .layer(DefaultBodyLimit::max(state.config.max_upload_bytes))
        .route_layer(middleware::from_fn_with_state(
            Arc::clone(&state),
            auth::require_auth,
        ))
        .with_state(state);
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .nest("/api", api)
        .fallback_service(ServeDir::new(web_dir).not_found_service(ServeFile::new(index)))
}
